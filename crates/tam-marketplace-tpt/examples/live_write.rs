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
//!   cargo run -p tam-marketplace-tpt --example live_write -- draft <pdf>
//!   cargo run -p tam-marketplace-tpt --example live_write -- edit  <pdf>
//!   cargo run -p tam-marketplace-tpt --example live_write -- paid  <pdf> [--yes-publish-live]
//!
//! `preflight` renders one form and creates nothing. `draft` creates a draft,
//! reads it back and deletes it. `edit` creates a draft, rewrites it, reads
//! the change back and deletes it. `paid` creates a draft at the form's own
//! minimum price and, only with `--yes-publish-live`, publishes it before
//! reading it back and deleting it.

use std::io::Read as _;

use tam_marketplace::{
    AdapterError, FetchReason, FieldSet, FileContent, FileSource, FileSourceError, FormId,
    IdempotencyKey, ListingLocator, MarketplaceAdapter as _, NativeTerm, ObservedListing, Pause,
    ProjectedListing, RemoteLifecycle, RemoteListingId, WriteAttemptId,
};
use tam_marketplace_tpt::write_model::{self, AuthorshipDeclaration, StatusUser};
use tam_marketplace_tpt::{ProductId, ReqwestTransport, TptAdapter, TptSession};
use tam_types::{Currency, FieldKey, FileId, Money, OrgId, PriceIntent, Timestamp, Uuid};

const DEFAULT_JAR: &str = "probes/local/tpt-cookies.jar";
/// Neither identifier reaches TPT. The org scopes nothing here because this
/// script holds no database, and the file id is what the projection carries
/// for the one file the operator named on the command line.
const ORG: OrgId = OrgId(Uuid([0; 16]));
/// See [`ORG`].
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
type Failure = Box<dyn std::error::Error>;

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
        .submit(ORG, IdempotencyKey(Uuid(seed(now))), fields, now)
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
            ORG,
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

async fn read_back(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
) -> Result<ObservedListing, Failure> {
    let observed = observe(adapter, product, now)
        .await
        .map_err(|error| failed("the read-back failed", &error))?;
    println!(
        "  reads back: title={:?} price={} lifecycle={:?}",
        reported(&observed, FieldKey::Title),
        reported(&observed, FieldKey::Price),
        observed.lifecycle,
    );
    Ok(observed)
}

/// The cleanup every mode ends with: delete, then prove the product is gone
/// by a read that no longer finds it.
async fn delete_and_confirm(
    adapter: &Adapter,
    product: ProductId,
    now: Timestamp,
) -> Result<(), Failure> {
    adapter
        .delete(product)
        .await
        .map_err(|error| failed("the delete failed", &error))?;
    println!("  deleted {product}");
    match observe(adapter, product, now).await {
        Ok(observed) => {
            Err(format!("{product} still reads back after its delete: {observed:?}").into())
        }
        Err(AdapterError::Ambiguous(_)) => {
            println!("  confirmed gone: the catalogue no longer returns {product}");
            Ok(())
        }
        Err(error) => Err(format!(
            "the delete of {product} could not be confirmed: {}",
            describe(&error)
        )
        .into()),
    }
}

async fn run_preflight(adapter: &Adapter) -> Result<(), Failure> {
    let fingerprint = adapter
        .assert_form_schema(ORG, FormId(Uuid([1; 16])))
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
    let product = create(adapter, &listing, now).await?;
    read_back(adapter, product, now).await?;
    delete_and_confirm(adapter, product, now).await
}

async fn run_edit(adapter: &Adapter, now: Timestamp) -> Result<(), Failure> {
    let listing = projection(
        title_for("edit", now),
        "<p>The first body.</p>".to_owned(),
        PriceIntent::Free,
    );
    println!("edit: creating {:?}", listing.title);
    let product = create(adapter, &listing, now).await?;
    let observed = read_back(adapter, product, now).await?;
    // An edit restates every field including the status, so the status comes
    // from the read-back rather than from what the create is assumed to have
    // left behind.
    let status = status_of(&observed)?;
    // The product read carries no description, so the title moves with it:
    // the edit changes both and the title is what a read-back can prove.
    let rewritten = projection(
        format!("{}-edited", listing.title),
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
    let observed = read_back(adapter, product, now).await?;
    if reported(&observed, FieldKey::Title) != rewritten.title {
        return Err(format!(
            "the edit did not take: {product} still reads back as {:?}",
            reported(&observed, FieldKey::Title)
        )
        .into());
    }
    println!("  confirmed edited: the read-back carries the new title");
    delete_and_confirm(adapter, product, now).await
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
    read_back(adapter, product, now).await?;
    if publish {
        let fields = fields_of(adapter, &listing)?;
        adapter
            .publish(product, &fields)
            .await
            .map_err(|error| failed("the publish failed", &error))?;
        println!("  PUBLISHED {product} live; it is deleted again below");
        read_back(adapter, product, now).await?;
    } else {
        println!("  left as a draft; pass --yes-publish-live to publish it");
    }
    delete_and_confirm(adapter, product, now).await
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
    let bytes = if mode == "preflight" {
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
        other => {
            Err(format!("unknown mode {other:?}; expected preflight, draft, edit or paid").into())
        }
    }
}
