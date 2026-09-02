//! The operator's supervised live write: the create, edit, publish and delete
//! lifecycle against the founder's own TPT account.
//!
//! Every mode cleans up after itself. Whatever it creates carries a
//! `ZZ-DELETE-ME` title, is deleted before the run ends, and the delete is
//! confirmed by a read that no longer finds the product. A mode that cannot
//! confirm the removal fails loudly rather than leaving a stray listing
//! behind.
//!
//! Usage, from the repo root, with the operator's cookie jar at
//! `probes/local/tpt-cookies.jar` or named by `--jar <path>`:
//!
//!   cargo run -p tam-marketplace-tpt --example live_write -- preflight
//!   cargo run -p tam-marketplace-tpt --example live_write -- draft  <pdf>
//!   cargo run -p tam-marketplace-tpt --example live_write -- edit   <pdf>
//!   cargo run -p tam-marketplace-tpt --example live_write -- paid   <pdf> [--yes-publish-live]
//!   cargo run -p tam-marketplace-tpt --example live_write -- delete <product-id>
//!
//! `preflight` renders one form and creates nothing. `draft` creates a draft,
//! reads it back and deletes it. `edit` creates a draft, rewrites it, reads
//! the change back and deletes it. `paid` creates a draft at the form's own
//! minimum price and, only with `--yes-publish-live`, publishes it before
//! reading it back and deleting it.
//!
//! `delete` creates nothing and takes down a product this script did not
//! make. It exists for the listing another runner left behind: the engine's
//! own write path creates through `tam-worker`, and a run that wedges after
//! the create has no cleanup guard of its own to fall back on. The confirming
//! read is the same one every mode above ends with, so a delete that answered
//! 200 and removed nothing is caught here too.

use std::io::Read as _;

use tam_marketplace::{
    AdapterError, FetchReason, FieldSet, FileContent, FileSource, FileSourceError, FormId,
    IdempotencyKey, ListingLocator, MarketplaceAdapter as _, NativeTerm, ObservedListing, Pause,
    ProjectedListing, RemoteLifecycle, RemoteListingId, WriteAttemptId,
};
use tam_marketplace_tpt::write_model::{self, AuthorshipDeclaration, StatusUser};
use tam_marketplace_tpt::{ProductId, ReqwestTransport, TptAdapter, TptSession};
use tam_types::{CopyFormat, Currency, FieldKey, FileId, Money, PriceIntent, Timestamp, Uuid};

const DEFAULT_JAR: &str = "probes/local/tpt-cookies.jar";
/// The identifier does not reach TPT: it is what the projection carries for
/// the one file the operator named on the command line.
const FILE: FileId = FileId(Uuid([1; 16]));

/// A file the operator named on the command line, handed back whichever id is
/// asked for: this script's projection carries exactly one.
struct OneFile(FileContent);

impl FileSource for OneFile {
    fn fetch(
        &self,
        _file: FileId,
    ) -> impl core::future::Future<Output = Result<FileContent, FileSourceError>> + Send {
        core::future::ready(Ok(self.0.clone()))
    }
}

/// The queue cadence the connector asks for, actually waited out. The
/// cassette tests use `InstantPause`; a live run polling a real conversion
/// job without pausing would exhaust its patience in milliseconds.
struct SleepingPause;

impl Pause for SleepingPause {
    fn pause(&self, ms: u32) -> impl core::future::Future<Output = ()> + Send {
        tokio::time::sleep(core::time::Duration::from_millis(u64::from(ms)))
    }
}

type Adapter = TptAdapter<ReqwestTransport, OneFile, SleepingPause>;
/// `Send + Sync` so the futures below stay `Send`: a cleanup guard holds the
/// operation's outcome across the await that deletes the product, and a
/// boxed error that is not `Send` would make every mode's future unspawnable.
type Failure = Box<dyn std::error::Error + Send + Sync>;

/// What an operator should do about each condition, which is the whole reason
/// the classifier tells a lapsed session apart from an edge refusal.
fn describe(error: &AdapterError) -> String {
    match *error {
        AdapterError::SessionExpired => {
            format!("the session lapsed: sign in with a browser and re-export {DEFAULT_JAR}")
        }
        AdapterError::Challenge(kind) => format!(
            "the edge refused this caller ({kind:?}): a managed challenge or a rule against the \
             address this ran from. Refreshing the cookie jar will not clear it"
        ),
        AdapterError::Ambiguous(cause) => format!(
            "the write may have landed and cannot be settled from here ({cause:?}): check the \
             catalogue before running anything else"
        ),
        AdapterError::RateLimited { retry_after } => {
            format!("TPT rate-limited this run (retry after {retry_after:?})")
        }
        AdapterError::NotSent(cause) => format!("the request never left ({cause:?})"),
        AdapterError::Rejected { code, ref detail } => format!("refused ({code:?}): {}", detail.0),
        AdapterError::SchemaDrift(ref drift) => {
            format!(
                "the form's shape moved; it no longer declares {:?}",
                drift.removed
            )
        }
        AdapterError::Uncaptured { capability } => {
            format!("no capture settles {capability}, so there was nothing to send")
        }
    }
}

fn failed(what: &str, error: &AdapterError) -> Failure {
    format!("{what}: {}", describe(error)).into()
}

#[expect(
    clippy::disallowed_methods,
    reason = "the live runner is a clock-reading process boundary; time enters the adapter as data from here"
)]
fn wall_now() -> Result<Timestamp, Failure> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

/// One run's identifiers, all derived from the instant it started so a second
/// run never reuses the first one's idempotency key.
fn seed(now: Timestamp) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    let millis = now.0.to_le_bytes();
    for (slot, byte) in bytes.iter_mut().zip(millis.iter().chain(millis.iter())) {
        *slot = *byte;
    }
    bytes
}

fn read_file(path: &str) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The listing every mode writes. Both taxonomy slugs are TPT's own, from the
/// captured create; no seller shelf is named, so nothing lands on one of the
/// founder's categories.
fn projection(title: String, body: String, price: PriceIntent) -> ProjectedListing {
    ProjectedListing {
        title,
        body,
        price,
        taxonomy: vec![NativeTerm {
            native_id: Some("algebra".to_owned()),
            segments: vec!["Math".to_owned(), "Algebra".to_owned()],
        }],
        grades: vec![NativeTerm {
            native_id: Some("not-grade-specific".to_owned()),
            segments: vec!["Not grade specific".to_owned()],
        }],
        ages: None,
        files: vec![FILE],
        body_format: CopyFormat::Html,
        natives: Vec::new(),
    }
}

fn title_for(mode: &str, now: Timestamp) -> String {
    format!("ZZ-DELETE-ME-{mode}-{}", now.0)
}

fn fields_of(adapter: &Adapter, listing: &ProjectedListing) -> Result<FieldSet, Failure> {
    adapter
        .project_fields(listing)
        .map_err(|error| failed("the projection was refused", &error))
}

async fn create(
    adapter: &Adapter,
    listing: &ProjectedListing,
    now: Timestamp,
) -> Result<ProductId, Failure> {
    let fields = fields_of(adapter, listing)?;
    let evidence = adapter
        .submit(IdempotencyKey(Uuid(seed(now))), fields, now)
        .await
        .map_err(|error| failed("the create failed", &error))?;
    match evidence.landed {
        Some(RemoteListingId::Tpt { product_id }) => {
            println!(
                "  created draft {product_id} at {}",
                evidence.landed_on_route.unwrap_or_default()
            );
            Ok(ProductId(product_id))
        }
        landed => Err(format!("the submit landed on {landed:?} rather than a TPT product").into()),
    }
}

async fn observe(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
) -> Result<ObservedListing, AdapterError> {
    adapter
        .read_back(
            ListingLocator::Durable(product.remote()),
            FetchReason::VerifyAttempt {
                attempt: WriteAttemptId(Uuid(seed(now))),
            },
            now,
        )
        .await
}

fn reported(observed: &ObservedListing, key: FieldKey) -> &str {
    observed
        .fields
        .iter()
        .find(|(field, _)| *field == key)
        .map_or("<absent>", |(_, value)| value.as_str())
}

/// The status selector an edit has to restate, taken from what the product
/// actually reads back rather than from what this script assumed it left
/// behind. An edit is a full replace, so a guess here is how a draft gets
/// published or a live listing gets pulled.
fn status_of(observed: &ObservedListing) -> Result<StatusUser, Failure> {
    match observed.lifecycle {
        RemoteLifecycle::Live { .. } => Ok(StatusUser::Live),
        RemoteLifecycle::Draft => Ok(StatusUser::Draft),
        // The TPT read reports ACTIVE or not-yet-live and nothing else, so
        // any other state is one this adapter has never seen upstream.
        RemoteLifecycle::Absent
        | RemoteLifecycle::Submitted { .. }
        | RemoteLifecycle::InReview { .. }
        | RemoteLifecycle::Rejected { .. }
        | RemoteLifecycle::Withdrawn { .. } => Err(format!(
            "{:?} is not a state this adapter reports; refusing to restate a status rather than \
             guessing which side of the draft line to post",
            observed.lifecycle
        )
        .into()),
    }
}

/// How many times a verification re-reads before it gives up, and how long it
/// waits between tries.
///
/// TPT's read API lags its write API by seconds, measured live on 2026-08-28:
/// a delete answered `resourceDelete` 200 while the product still read back
/// two seconds later and was gone by twenty, and an edit that submitted
/// cleanly read back its old title. One instant read is therefore not a
/// verdict about this API — it is a false negative waiting to happen — and
/// every verification below polls until the state it expects appears.
///
/// The budget is sized to the slowest convergence actually observed rather
/// than to the fastest: eleven reads two seconds apart is twenty seconds of
/// waiting. The production read-back path — the driver that settles a write
/// receipt, and the reconciliation behind it — faces the same lag and will
/// need the same treatment; this constant is the example's own and settles
/// nothing for that path.
const VERIFY_TRIES: u32 = 11;
/// See [`VERIFY_TRIES`].
const VERIFY_INTERVAL_MS: u32 = 2_000;

/// Re-reads the product until `settled` accepts what came back, or until the
/// budget is spent, and hands the caller the last answer either way. The
/// caller re-checks its own condition on that answer, so a budget that runs
/// out is reported as the condition failing rather than as a read failing.
async fn poll_until(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
    settled: impl Fn(&Result<ObservedListing, AdapterError>) -> bool,
) -> Result<ObservedListing, AdapterError> {
    let mut seen = observe(adapter, product, now).await;
    for read in 1..VERIFY_TRIES {
        if settled(&seen) {
            return seen;
        }
        println!(
            "  not settled after read {read}/{VERIFY_TRIES}; waiting {VERIFY_INTERVAL_MS}ms — \
             TPT's read lags its write"
        );
        SleepingPause.pause(VERIFY_INTERVAL_MS).await;
        seen = observe(adapter, product, now).await;
    }
    seen
}

fn describe_observation(observed: &ObservedListing) {
    println!(
        "  reads back: title={:?} price={} lifecycle={:?}",
        reported(observed, FieldKey::Title),
        reported(observed, FieldKey::Price),
        observed.lifecycle,
    );
}

/// The product exists and reads back, polled because a create does not appear
/// on the read API the instant its form post returns.
///
/// [`RemoteLifecycle::Absent`] is not a presence proof. Since absence became
/// an observation rather than a read failure, a read that settles on it is the
/// catalogue answering that the product is not there — exactly what this check
/// exists to rule out, and what `Result::is_ok` alone accepts.
async fn confirm_present(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
) -> Result<ObservedListing, Failure> {
    let observed = poll_until(adapter, product, now, |seen| {
        seen.as_ref()
            .is_ok_and(|observed| !matches!(observed.lifecycle, RemoteLifecycle::Absent))
    })
    .await
    .map_err(|error| {
        failed(
            "the read-back never settled, so nothing here proves the product is there",
            &error,
        )
    })?;
    describe_observation(&observed);
    if matches!(observed.lifecycle, RemoteLifecycle::Absent) {
        return Err(format!(
            "{product} never appeared within the verification budget: the catalogue still \
             answers that it is not there"
        )
        .into());
    }
    Ok(observed)
}

async fn confirm_title(
    adapter: &Adapter,
    product: ProductId,
    expected: &str,
    now: Timestamp,
) -> Result<(), Failure> {
    let observed = poll_until(adapter, product, now, |seen| {
        seen.as_ref()
            .is_ok_and(|observed| reported(observed, FieldKey::Title) == expected)
    })
    .await
    .map_err(|error| failed("the read-back after the edit failed", &error))?;
    describe_observation(&observed);
    if reported(&observed, FieldKey::Title) == expected {
        println!("  confirmed edited: the read-back carries the new title");
        return Ok(());
    }
    Err(format!(
        "the edit did not appear within the verification budget: {product} still reads back as {:?}",
        reported(&observed, FieldKey::Title)
    )
    .into())
}

async fn confirm_live(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
) -> Result<(), Failure> {
    let observed = poll_until(adapter, product, now, |seen| {
        seen.as_ref()
            .is_ok_and(|observed| matches!(observed.lifecycle, RemoteLifecycle::Live { .. }))
    })
    .await
    .map_err(|error| failed("the read-back after the publish failed", &error))?;
    describe_observation(&observed);
    if matches!(observed.lifecycle, RemoteLifecycle::Live { .. }) {
        println!("  confirmed live: the read-back reports the published state");
        return Ok(());
    }
    Err(format!(
        "the publish did not appear within the verification budget: {product} still reads back \
         as {:?}",
        observed.lifecycle
    )
    .into())
}

/// What the confirming read after a delete settled on.
///
/// The three answers are kept apart because collapsing two of them is what
/// reported every clean removal as a failed cleanup: a product the parsed
/// catalogue does not carry reads back as [`RemoteLifecycle::Absent`], and the
/// check that predates that contract waited for an `Ambiguous` read that no
/// longer comes.
enum Removal {
    /// The read-back settled on `Absent`: the catalogue no longer carries it.
    Gone,
    /// The read-back still carries the product after the whole budget.
    StillThere(RemoteLifecycle),
    /// The read never settled either way, so the removal is unproven — which
    /// is a different claim from the product still being there.
    Unconfirmed(String),
}

/// Delete, then prove the product is gone by a read that no longer finds it —
/// polled, because a delete this API answered 200 to was still readable two
/// seconds later in the live run.
async fn delete_and_confirm(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
) -> Result<Removal, Failure> {
    adapter
        .delete(product)
        .await
        .map_err(|error| failed("the delete failed", &error))?;
    println!("  deleted {product}");
    let settled = poll_until(adapter, product, now, |seen| {
        seen.as_ref()
            .is_ok_and(|observed| matches!(observed.lifecycle, RemoteLifecycle::Absent))
    })
    .await;
    Ok(match settled {
        Ok(observed) if matches!(observed.lifecycle, RemoteLifecycle::Absent) => {
            println!("  confirmed gone: the catalogue no longer returns {product}");
            Removal::Gone
        }
        Ok(observed) => {
            describe_observation(&observed);
            Removal::StillThere(observed.lifecycle)
        }
        Err(error) => Removal::Unconfirmed(describe(&error)),
    })
}

/// The operator-facing account of a removal that was not proven, and `None`
/// where it was. An unsettled read is reported as unconfirmed rather than as a
/// failed cleanup: one says a listing is on the store, the other says nobody
/// knows, and they ask the operator for different things.
fn unresolved(removal: &Removal, product: ProductId) -> Option<String> {
    match *removal {
        Removal::Gone => None,
        Removal::StillThere(ref lifecycle) => Some(format!(
            "CLEANUP FAILED: {product} still reads back as {lifecycle:?} after its delete and \
             the whole verification budget, so it is still on the store"
        )),
        Removal::Unconfirmed(ref why) => Some(format!(
            "CLEANUP UNCONFIRMED: the delete of {product} was issued and the confirming read \
             never settled ({why}), so check the store before running anything else"
        )),
    }
}

/// The cleanup guard every mode that creates something ends with.
///
/// Whatever the operation concluded, the product this run created is deleted
/// and its removal confirmed, and the two outcomes are reported separately: a
/// failed assertion must never be a reason to leave a listing on the
/// founder's store, which is exactly what an early return once did.
async fn finish(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
    outcome: Result<(), Failure>,
) -> Result<(), Failure> {
    println!("cleanup: deleting {product} whatever the operation concluded");
    let cleanup = match delete_and_confirm(adapter, product, now).await {
        Ok(removal) => unresolved(&removal, product),
        Err(refused) => Some(format!(
            "CLEANUP FAILED: {refused}, so {product} may still be on the store"
        )),
    };
    match (outcome, cleanup) {
        (Ok(()), None) => {
            println!("ok: the operation succeeded and {product} is gone");
            Ok(())
        }
        (Err(operation), None) => {
            println!("cleanup ok: {product} is gone, so nothing was left behind");
            Err(format!("the operation failed: {operation}").into())
        }
        (Ok(()), Some(cleanup)) => Err(format!("the operation succeeded but {cleanup}").into()),
        (Err(operation), Some(cleanup)) => {
            Err(format!("the operation failed ({operation}) AND {cleanup}").into())
        }
    }
}

async fn run_preflight(adapter: &Adapter) -> Result<(), Failure> {
    let fingerprint = adapter
        .assert_form_schema(FormId(Uuid([1; 16])))
        .await
        .map_err(|error| failed("the preflight failed", &error))?;
    println!(
        "preflight ok; the create form declares every field this adapter writes. \
         fingerprint {:02x?}",
        fingerprint.0 .0
    );
    Ok(())
}

async fn run_draft(adapter: &Adapter, now: Timestamp) -> Result<(), Failure> {
    let listing = projection(
        title_for("draft", now),
        "<p>A throwaway listing written by live_write. Delete on sight.</p>".to_owned(),
        PriceIntent::Free,
    );
    println!("draft: creating {:?}", listing.title);
    // The id is captured the moment it exists, and every step after it runs
    // under the cleanup guard.
    let product = create(adapter, &listing, now).await?;
    let outcome = confirm_present(adapter, product, now).await.map(|_| ());
    finish(adapter, product, now, outcome).await
}

/// The edit itself, run under [`finish`]: nothing in here may return in a way
/// that skips the delete.
async fn edit_and_verify(
    adapter: &Adapter,
    product: ProductId,
    title: &str,
    now: Timestamp,
) -> Result<(), Failure> {
    let observed = confirm_present(adapter, product, now).await?;
    // An edit restates every field including the status, so the status comes
    // from the read-back rather than from what the create is assumed to have
    // left behind.
    let status = status_of(&observed)?;
    // The product read carries no description, so the title moves with it:
    // the edit changes both and the title is what a read-back can prove.
    let rewritten = projection(
        format!("{title}-edited"),
        "<p>The second body, written by the edit.</p>".to_owned(),
        PriceIntent::Free,
    );
    let fields = fields_of(adapter, &rewritten)?;
    adapter
        .update(product, &fields, status)
        .await
        .map_err(|error| failed("the edit failed", &error))?;
    println!(
        "  edited {product}: description rewritten, title moved with it, status kept at {status:?}"
    );
    confirm_title(adapter, product, &rewritten.title, now).await
}

async fn run_edit(adapter: &Adapter, now: Timestamp) -> Result<(), Failure> {
    let listing = projection(
        title_for("edit", now),
        "<p>The first body.</p>".to_owned(),
        PriceIntent::Free,
    );
    println!("edit: creating {:?}", listing.title);
    let product = create(adapter, &listing, now).await?;
    let outcome = edit_and_verify(adapter, product, &listing.title, now).await;
    finish(adapter, product, now, outcome).await
}

/// The priced half, run under [`finish`] for the same reason the edit is.
async fn paid_and_verify(
    adapter: &Adapter,
    product: ProductId,
    listing: &ProjectedListing,
    publish: bool,
    now: Timestamp,
) -> Result<(), Failure> {
    confirm_present(adapter, product, now).await?;
    if !publish {
        println!("  left as a draft; pass --yes-publish-live to publish it");
        return Ok(());
    }
    let fields = fields_of(adapter, listing)?;
    adapter
        .publish(product, &fields)
        .await
        .map_err(|error| failed("the publish failed", &error))?;
    println!("  PUBLISHED {product} live; the cleanup below deletes it again");
    confirm_live(adapter, product, now).await
}

async fn run_paid(adapter: &Adapter, now: Timestamp, publish: bool) -> Result<(), Failure> {
    // The form's own stated minimum, in the store's own units. TPT names no
    // currency on the wire; the `Currency` here travels only inside the
    // projection, where a mismatch between inventories stays visible.
    let money = Money::new(write_model::MIN_PRICE_MINOR_UNITS, Currency::Usd)?;
    let listing = projection(
        title_for("paid", now),
        "<p>A throwaway priced listing written by live_write. Delete on sight.</p>".to_owned(),
        PriceIntent::Paid(money),
    );
    println!(
        "paid: creating {:?} at {} minor units",
        listing.title,
        money.minor_units()
    );
    let product = create(adapter, &listing, now).await?;
    let outcome = paid_and_verify(adapter, product, &listing, publish, now).await;
    finish(adapter, product, now, outcome).await
}

/// Take down a product this run did not create, named on the command line.
///
/// The identifier is required rather than defaulted: every other mode deletes
/// what it just made and knows the id for certain, and a cleanup mode that
/// guessed one would delete a listing nobody asked it to.
async fn run_delete(
    adapter: &Adapter,
    arguments: &[String],
    now: Timestamp,
) -> Result<(), Failure> {
    let raw = arguments
        .get(1)
        .filter(|argument| !argument.starts_with("--"))
        .ok_or("delete needs the numeric TPT product id to take down")?;
    let product = ProductId(
        raw.parse::<u64>()
            .map_err(|error| format!("{raw:?} is not a TPT product id: {error}"))?,
    );
    println!("deleting {product}, which this run did not create");
    let removal = delete_and_confirm(adapter, product, now).await?;
    if let Some(unproven) = unresolved(&removal, product) {
        return Err(unproven.into());
    }
    Ok(())
}

fn adapter_for(mode: &str, arguments: &[String], now: Timestamp) -> Result<Adapter, Failure> {
    let jar_path = arguments
        .iter()
        .position(|argument| argument == "--jar")
        .and_then(|at| arguments.get(at.saturating_add(1)))
        .map_or(DEFAULT_JAR, String::as_str);
    let mut jar = String::new();
    std::fs::File::open(jar_path)?.read_to_string(&mut jar)?;
    let session = TptSession::from_netscape_jar(&jar)?;
    // Neither mode uploads anything: one renders a form, the other names a
    // product that already exists.
    let bytes = if matches!(mode, "preflight" | "delete") {
        Vec::new()
    } else {
        let path = arguments
            .get(1)
            .filter(|argument| !argument.starts_with("--"))
            .ok_or("this mode needs a path to the file to upload")?;
        read_file(path)?
    };
    Ok(TptAdapter::new(
        ReqwestTransport::new(&session)?,
        OneFile(FileContent {
            file_name: "zz-delete-me.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            bytes,
        }),
        SleepingPause,
    )
    .attesting(AuthorshipDeclaration::attested(
        "the operator running live_write".to_owned(),
        now,
    )))
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let mode = arguments
        .first()
        .map_or("preflight", String::as_str)
        .to_owned();
    let now = wall_now()?;
    let adapter = adapter_for(&mode, &arguments, now)?;
    match mode.as_str() {
        "preflight" => run_preflight(&adapter).await,
        "draft" => run_draft(&adapter, now).await,
        "edit" => run_edit(&adapter, now).await,
        "paid" => {
            let publish = arguments
                .iter()
                .any(|argument| argument == "--yes-publish-live");
            run_paid(&adapter, now, publish).await
        }
        "delete" => run_delete(&adapter, &arguments, now).await,
        other => Err(format!(
            "unknown mode {other:?}; expected preflight, draft, edit, paid or delete"
        )
        .into()),
    }
}
