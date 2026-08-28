//! The operator's supervised live check against the founder's own Tes account.
//!
//! Every mode that creates something cleans up after itself: what it creates
//! carries a `ZZ-SMOKE-DELETE-ME` title, is deleted before the run ends, and
//! the delete is proved by a read that no longer finds it. A mode that cannot
//! confirm the removal fails loudly rather than leaving a stray listing on the
//! account.
//!
//! Usage, from the repo root with the operator's cookie jar:
//!   cargo run -p tam-marketplace-tes --example live_smoke -- <jar> preflight
//!   cargo run -p tam-marketplace-tes --example live_smoke -- <jar> list
//!   cargo run -p tam-marketplace-tes --example live_smoke -- <jar> draft <pdf>
//!   cargo run -p tam-marketplace-tes --example live_smoke -- <jar> edit <pdf>
//!   cargo run -p tam-marketplace-tes --example live_smoke -- <jar> paid <pdf> \
//!       [--price <minor units>] [--yes-publish-live]
//!   cargo run -p tam-marketplace-tes --example live_smoke -- <jar> \
//!       delete-published <pdf> --yes-publish-live
//!
//! `preflight` probes the draft schema and creates nothing that outlives it.
//! `list` reads the seller's own catalogue and writes nothing at all.
//! `draft` creates a draft, reads it back and deletes it. `edit` creates a
//! draft, rewrites its title and description, reads the change back and
//! deletes it. `paid` creates a priced draft and, only with
//! `--yes-publish-live`, publishes it before reading it back and deleting it.
//! `delete-published` publishes a free listing for the sole purpose of
//! deleting it through the published-resource route.

use std::io::Read as _;

use tam_marketplace::{
    AdapterError, FetchReason, FileContent, FileSource, FileSourceError, FormId, ListingLocator,
    MarketplaceAdapter as _, ObservedListing, RemoteLifecycle, RemoteListingId, WriteAttemptId,
};
use tam_marketplace_tes::endpoints::{
    self, DraftId, FreeLicence, TesListing, TesPrice, TesPricing, ZZ_TITLE_PREFIX,
};
use tam_marketplace_tes::{ListingState, ReqwestTransport, TesAdapter, TesSession};
use tam_types::{FieldKey, FileId, InventoryId, OrgId, Timestamp, Uuid};

/// Neither identifier reaches Tes. The org scopes nothing here because this
/// script holds no database, and the file id is what the one file the operator
/// named on the command line answers to.
const ORG: OrgId = OrgId(Uuid([0; 16]));

/// The price a paid run posts unless `--price` names another, in the minor
/// units the wire carries. It is the amount the 2026-08-28 publish capture
/// carries (GBP 5.00); Tes's own minimum has never been measured, so this is
/// the only evidenced value rather than the smallest permitted one.
const SMOKE_PRICE_MINOR_UNITS: i64 = 500;

/// `Send + Sync` so the futures below stay `Send`: the cleanup guard holds the
/// operation's outcome across the await that deletes the listing.
type Failure = Box<dyn std::error::Error + Send + Sync>;
type Adapter = TesAdapter<ReqwestTransport, OneFile>;

struct OneFile(FileContent);

impl FileSource for OneFile {
    async fn fetch(&self, _file: FileId) -> Result<FileContent, FileSourceError> {
        Ok(self.0.clone())
    }
}

/// What an operator should do about each condition, which is the whole reason
/// the classifier tells a lapsed session apart from an edge refusal.
fn describe(error: &AdapterError) -> String {
    match *error {
        AdapterError::SessionExpired => {
            "the session lapsed: sign in with a browser and re-export the cookie jar".to_owned()
        }
        AdapterError::Challenge(kind) => format!(
            "the edge refused this caller ({kind:?}); refreshing the cookie jar will not clear it"
        ),
        AdapterError::Ambiguous(cause) => format!(
            "the write may have landed and cannot be settled from here ({cause:?}): check the \
             dashboard before running anything else"
        ),
        AdapterError::RateLimited { retry_after } => {
            format!("Tes rate-limited this run (retry after {retry_after:?})")
        }
        AdapterError::NotSent(cause) => format!("the request never left ({cause:?})"),
        AdapterError::Rejected { code, ref detail } => format!("refused ({code:?}): {}", detail.0),
        AdapterError::SchemaDrift(ref drift) => format!(
            "the draft's shape moved; it no longer declares {:?}",
            drift.removed
        ),
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

fn read_file(path: &str) -> Result<Vec<u8>, std::io::Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The listing every mode writes: the probe's cassette-proven taxonomy with a
/// disposable title, so nothing here names a category the crosswalk has not
/// already seen.
fn listing(mode: &str, now: Timestamp, pricing: TesPricing) -> TesListing {
    TesListing {
        title: format!("{ZZ_TITLE_PREFIX}-{mode}-{}", now.0),
        description_markdown: format!(
            "A throwaway listing written by live_smoke ({mode}). **Delete on sight.**"
        ),
        pricing,
        ..endpoints::probe_listing()
    }
}

const FREE: TesPricing = TesPricing::Free(FreeLicence::CcBy);

/// How many times a verification re-reads before it gives up, and how long it
/// waits between tries.
///
/// Tes has never been measured for read-after-write lag; TPT's read API was,
/// and lags its write API by up to twenty seconds. One instant read is
/// therefore not a verdict about an API whose consistency is unknown, so every
/// verification below polls, and a run that needs no retry says so.
const VERIFY_TRIES: u32 = 8;
/// See [`VERIFY_TRIES`].
const VERIFY_INTERVAL: core::time::Duration = core::time::Duration::from_secs(2);

async fn observe(
    adapter: &Adapter,
    id: DraftId,
    now: Timestamp,
) -> Result<ObservedListing, AdapterError> {
    adapter
        .read_back(
            ORG,
            ListingLocator::Durable(RemoteListingId::Tes {
                url: format!("{}/api/v2/resources/{}", endpoints::ORIGIN, id.0),
            }),
            FetchReason::VerifyAttempt {
                attempt: WriteAttemptId(Uuid([2; 16])),
            },
            now,
        )
        .await
}

/// Re-reads until `settled` accepts what came back, or until the budget is
/// spent, and hands the caller the last answer either way. The caller re-checks
/// its own condition on that answer, so a spent budget is reported as the
/// condition failing rather than as a read failing.
async fn poll_until(
    adapter: &Adapter,
    id: DraftId,
    now: Timestamp,
    settled: impl Fn(&Result<ObservedListing, AdapterError>) -> bool,
) -> Result<ObservedListing, AdapterError> {
    let mut seen = observe(adapter, id, now).await;
    for read in 1..VERIFY_TRIES {
        if settled(&seen) {
            if read > 1 {
                println!("  settled after {read} reads; this API lagged its own write");
            }
            return seen;
        }
        println!("  not settled after read {read}/{VERIFY_TRIES}; waiting {VERIFY_INTERVAL:?}");
        tokio::time::sleep(VERIFY_INTERVAL).await;
        seen = observe(adapter, id, now).await;
    }
    seen
}

fn reported(observed: &ObservedListing, key: FieldKey) -> &str {
    observed
        .fields
        .iter()
        .find(|(field, _)| *field == key)
        .map_or("<absent>", |(_, value)| value.as_str())
}

fn describe_observation(observed: &ObservedListing) {
    println!(
        "  reads back: title={:?} lifecycle={:?}",
        reported(observed, FieldKey::Title),
        observed.lifecycle,
    );
}

async fn confirm_present(
    adapter: &Adapter,
    id: DraftId,
    now: Timestamp,
) -> Result<ObservedListing, Failure> {
    let observed = poll_until(adapter, id, now, Result::is_ok)
        .await
        .map_err(|error| failed("the read-back never found the resource", &error))?;
    describe_observation(&observed);
    Ok(observed)
}

async fn confirm_title(
    adapter: &Adapter,
    id: DraftId,
    expected: &str,
    now: Timestamp,
) -> Result<(), Failure> {
    let observed = poll_until(adapter, id, now, |seen| {
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
        "the edit did not appear within the verification budget: {} still reads back as {:?}",
        id.0,
        reported(&observed, FieldKey::Title)
    )
    .into())
}

async fn confirm_live(adapter: &Adapter, id: DraftId, now: Timestamp) -> Result<(), Failure> {
    let observed = poll_until(adapter, id, now, |seen| {
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
        "the publish did not appear within the verification budget: {} still reads back as {:?}",
        id.0, observed.lifecycle
    )
    .into())
}

/// The states a listing can still be answering on, checked directly rather
/// than through the read-back the presence checks use.
///
/// The catalogue-backed read lags a delete: the founder's live run saw the
/// dashboard still listing a resource whose own route had already gone to 404,
/// so it cannot witness a removal. These two routes can, and a delete has to
/// silence both — `/resources/{id}` is the buyer-facing read and `/{id}/draft`
/// serves the overlay, or the published resource itself where none exists.
async fn still_answering(
    adapter: &Adapter,
    id: DraftId,
) -> Result<Option<ListingState>, AdapterError> {
    for state in [ListingState::Published, ListingState::Draft] {
        if adapter.is_present(id, state).await? {
            return Ok(Some(state));
        }
    }
    Ok(None)
}

/// Polls both routes until neither answers. The proof of a delete, and the
/// only proof: the adapter's delete asserts once on the instant read, and this
/// is what an API that has not caught up with its own write gets measured
/// against.
async fn confirm_gone(adapter: &Adapter, id: DraftId) -> Result<(), Failure> {
    let mut last = format!("{} was never read after its delete", id.0);
    for read in 1..=VERIFY_TRIES {
        match still_answering(adapter, id).await {
            Ok(None) => {
                println!("  confirmed gone: neither route answers for {}", id.0);
                return Ok(());
            }
            Ok(Some(state)) => {
                last = format!("{} still answers on its {} route", id.0, state.name());
            }
            Err(error) => last = describe(&error),
        }
        if read < VERIFY_TRIES {
            println!("  {last}; waiting {VERIFY_INTERVAL:?} ({read}/{VERIFY_TRIES})");
            tokio::time::sleep(VERIFY_INTERVAL).await;
        }
    }
    Err(format!("the whole verification budget is spent and {last}").into())
}

/// Deletes the state the caller knows it created, then proves it gone against
/// the routes rather than against the delete's own verdict: the adapter
/// asserts on one immediate read, and this API does not always answer that
/// read with what it has already done.
async fn delete_and_confirm(
    adapter: &Adapter,
    id: DraftId,
    state: ListingState,
) -> Result<(), Failure> {
    if let Err(error) = adapter.delete(id, state).await {
        println!(
            "  the {} delete of {} is unproven so far ({}); re-reading",
            state.name(),
            id.0,
            describe(&error)
        );
    } else {
        println!("  deleted the {} {}", state.name(), id.0);
    }
    confirm_gone(adapter, id).await
}

/// The cleanup guard every mode that creates something ends with.
///
/// It issues BOTH deletes rather than choosing one from a read. `DELETE
/// /resources/{id}` is a 404 no-op for a draft-only resource and the draft
/// delete only ever removes an overlay, so issuing both cannot remove anything
/// the right single choice would have kept — while choosing one from a route
/// that lags a publish is precisely how a live listing survived its own
/// cleanup on 2026-08-29. The order is fixed — the resource before its overlay
/// — and the stated state is reported, not acted on. Each delete's own verdict
/// is printed and never trusted: a resource-route delete of a draft-only
/// listing reports a clean 404 having removed nothing, so the proof is the
/// read on both routes afterwards.
async fn sweep(adapter: &Adapter, id: DraftId, state: ListingState) -> Result<(), Failure> {
    for attempt in [ListingState::Published, ListingState::Draft] {
        if let Err(error) = adapter.delete(id, attempt).await {
            println!(
                "  the {} delete answered {}",
                attempt.name(),
                describe(&error)
            );
        }
    }
    println!(
        "  swept both routes for what this run left as a {}",
        state.name()
    );
    confirm_gone(adapter, id).await
}

/// Whatever the operation concluded, what this run created is removed and its
/// removal confirmed, and the two outcomes are reported separately: a failed
/// assertion must never be a reason to leave a listing on the founder's
/// account.
async fn finish(
    adapter: &Adapter,
    id: DraftId,
    state: ListingState,
    outcome: Result<(), Failure>,
) -> Result<(), Failure> {
    println!(
        "cleanup: removing {} whatever the operation concluded",
        id.0
    );
    let cleaned = sweep(adapter, id, state).await;
    match (outcome, cleaned) {
        (Ok(()), Ok(())) => {
            println!("ok: the operation succeeded and {} is gone", id.0);
            Ok(())
        }
        (Err(operation), Ok(())) => {
            println!("cleanup ok: {} is gone, so nothing was left behind", id.0);
            Err(format!("the operation failed: {operation}").into())
        }
        (Ok(()), Err(cleanup)) => Err(format!(
            "the operation succeeded but CLEANUP FAILED: {} may still be on the account — \
             {cleanup}",
            id.0
        )
        .into()),
        (Err(operation), Err(cleanup)) => Err(format!(
            "the operation failed ({operation}) AND CLEANUP FAILED: {} may still be on the \
             account — {cleanup}",
            id.0
        )
        .into()),
    }
}

async fn create(
    adapter: &Adapter,
    listing: &TesListing,
    file: FileContent,
) -> Result<DraftId, Failure> {
    let id = adapter
        .create_listing(listing, &[file])
        .await
        .map_err(|error| failed("the create failed", &error))?;
    println!("  created draft {}", id.0);
    Ok(id)
}

async fn run_preflight(adapter: &Adapter) -> Result<(), Failure> {
    let fingerprint = adapter
        .assert_form_schema(ORG, FormId(Uuid([1; 16])))
        .await
        .map_err(|error| failed("the preflight failed", &error))?;
    println!(
        "preflight ok; the draft declares every field this adapter writes. fingerprint {:02x?}",
        fingerprint.0 .0
    );
    Ok(())
}

/// Which of the two lists a catalogue row came from, as the row itself
/// reports it.
fn state(published: bool) -> &'static str {
    if published {
        "published"
    } else {
        "draft"
    }
}

/// A catalogue row's price in pounds, from the pence the API carries. A row
/// with no price at all reads as `-` rather than as free, because the two are
/// different answers.
fn pounds(price_pence: Option<i64>) -> String {
    match price_pence {
        None => "-".to_owned(),
        Some(0) => "free".to_owned(),
        Some(pence) => format!(
            "\u{a3}{}.{:02}",
            pence.checked_div(100).unwrap_or_default(),
            pence.checked_rem(100).unwrap_or_default().unsigned_abs(),
        ),
    }
}

/// The seller's own catalogue, published rows then drafts. Read-only: it
/// creates nothing and so has nothing to clean up.
async fn run_list(adapter: &Adapter) -> Result<(), Failure> {
    let entries = adapter
        .list_own_resources(&FetchReason::FirstPartyExport {
            inventory: InventoryId::TesGb,
        })
        .await
        .map_err(|error| failed("the catalogue read failed", &error))?;
    for entry in &entries {
        println!(
            "  {} | {} | {} | {} | {}",
            entry.id,
            state(entry.published),
            entry.licence.as_deref().unwrap_or("-"),
            pounds(entry.price_pence),
            entry.title,
        );
    }
    println!("catalogue: {} rows", entries.len());
    Ok(())
}

async fn run_draft(adapter: &Adapter, file: FileContent, now: Timestamp) -> Result<(), Failure> {
    let listing = listing("draft", now, FREE);
    println!("draft: creating {:?}", listing.title);
    let id = create(adapter, &listing, file).await?;
    let outcome = confirm_present(adapter, id, now).await.map(drop);
    finish(adapter, id, ListingState::Draft, outcome).await
}

/// The edit itself, run under [`finish`]: nothing in here may return in a way
/// that skips the delete.
async fn edit_and_verify(
    adapter: &Adapter,
    id: DraftId,
    created: &TesListing,
    now: Timestamp,
) -> Result<(), Failure> {
    confirm_present(adapter, id, now).await?;
    let rewritten = TesListing {
        title: format!("{}-edited", created.title),
        description_markdown: "The second body, written by the edit.".to_owned(),
        ..created.clone()
    };
    adapter
        .update(id, &rewritten)
        .await
        .map_err(|error| failed("the edit failed", &error))?;
    println!("  edited {}: title and description both rewritten", id.0);
    confirm_title(adapter, id, &rewritten.title, now).await
}

async fn run_edit(adapter: &Adapter, file: FileContent, now: Timestamp) -> Result<(), Failure> {
    let listing = listing("edit", now, FREE);
    println!("edit: creating {:?}", listing.title);
    let id = create(adapter, &listing, file).await?;
    let outcome = edit_and_verify(adapter, id, &listing, now).await;
    finish(adapter, id, ListingState::Draft, outcome).await
}

/// The publish, and the state change that must outlive it.
///
/// The state is stated before the request leaves rather than after it answers:
/// a publish whose response is lost or ambiguous may still have gone live, and
/// the cleanup that follows has to be told to expect a live listing in that
/// case. Getting this backwards is how a live paid listing survived its own
/// cleanup on 2026-08-29.
async fn publish_and_verify(
    adapter: &Adapter,
    id: DraftId,
    listing: &TesListing,
    now: Timestamp,
    state: &mut ListingState,
) -> Result<(), Failure> {
    *state = ListingState::Published;
    adapter
        .publish(id, listing)
        .await
        .map_err(|error| failed("the publish failed", &error))?;
    println!("  PUBLISHED {} live", id.0);
    confirm_live(adapter, id, now).await
}

async fn run_paid(
    adapter: &Adapter,
    file: FileContent,
    now: Timestamp,
    price: TesPrice,
    publish: bool,
) -> Result<(), Failure> {
    let listing = listing("paid", now, TesPricing::Paid(price));
    println!(
        "paid: creating {:?} at {} minor units",
        listing.title,
        price.minor_units()
    );
    let id = create(adapter, &listing, file).await?;
    // Every step after the id exists runs under the cleanup guard, and the
    // state it will clean up is tracked from here.
    let mut state = ListingState::Draft;
    let outcome = match confirm_present(adapter, id, now).await {
        Err(error) => Err(error),
        Ok(_) if !publish => {
            println!("  left as a priced draft; pass --yes-publish-live to publish it");
            Ok(())
        }
        Ok(_) => publish_and_verify(adapter, id, &listing, now, &mut state).await,
    };
    finish(adapter, id, state, outcome).await
}

/// Publishes a free listing for the sole purpose of deleting it through the
/// published-resource route, which is the only way to exercise that route
/// without touching anything the founder meant to keep.
async fn publish_then_delete(
    adapter: &Adapter,
    id: DraftId,
    listing: &TesListing,
    now: Timestamp,
    state: &mut ListingState,
) -> Result<(), Failure> {
    confirm_present(adapter, id, now).await?;
    publish_and_verify(adapter, id, listing, now, state).await?;
    // The state is stated, never probed: `/resources/{id}` lags a publish, so
    // a probe here reads a live listing as a draft and deletes its overlay
    // instead — which is what left a live paid listing standing on 2026-08-29.
    println!(
        "  deleting {} through DELETE /api/v2/resources/{{id}}",
        id.0
    );
    delete_and_confirm(adapter, id, ListingState::Published).await
}

async fn run_delete_published(
    adapter: &Adapter,
    file: FileContent,
    now: Timestamp,
    publish: bool,
) -> Result<(), Failure> {
    if !publish {
        return Err(
            "this mode publishes a listing live to delete it; pass --yes-publish-live"
                .to_owned()
                .into(),
        );
    }
    let listing = listing("delete-published", now, FREE);
    println!("delete-published: creating {:?}", listing.title);
    let id = create(adapter, &listing, file).await?;
    let mut state = ListingState::Draft;
    let outcome = publish_then_delete(adapter, id, &listing, now, &mut state).await;
    finish(adapter, id, state, outcome).await
}

fn flag(arguments: &[String], name: &str) -> bool {
    arguments.iter().any(|argument| argument == name)
}

fn price_argument(arguments: &[String]) -> Result<TesPrice, Failure> {
    let named = arguments
        .iter()
        .position(|argument| argument == "--price")
        .and_then(|at| arguments.get(at.saturating_add(1)));
    let minor_units = match named {
        Some(text) => text.parse()?,
        None => SMOKE_PRICE_MINOR_UNITS,
    };
    Ok(TesPrice::new(minor_units)?)
}

fn file_argument(arguments: &[String]) -> Result<FileContent, Failure> {
    let path = arguments
        .get(2)
        .filter(|argument| !argument.starts_with("--"))
        .ok_or("this mode needs a path to the file to upload")?;
    Ok(FileContent {
        file_name: "zz-delete-me.pdf".to_owned(),
        content_type: "application/pdf".to_owned(),
        bytes: read_file(path)?,
    })
}

fn empty_file() -> FileContent {
    FileContent {
        file_name: String::new(),
        content_type: String::new(),
        bytes: Vec::new(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Failure> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let jar_path = arguments.first().ok_or(
        "usage: live_smoke <jar> [preflight|list|draft <pdf>|edit <pdf>|paid <pdf> \
         [--price <minor units>] [--yes-publish-live]|delete-published <pdf> --yes-publish-live]",
    )?;
    let mut jar = String::new();
    std::fs::File::open(jar_path)?.read_to_string(&mut jar)?;
    let session = TesSession::from_netscape_jar(&jar)?;
    let transport = ReqwestTransport::new(&session)?;
    let mode = arguments.get(1).map_or("preflight", String::as_str);
    let file = if matches!(mode, "preflight" | "list") {
        empty_file()
    } else {
        file_argument(&arguments)?
    };
    let adapter = TesAdapter::new(InventoryId::TesGb, transport, OneFile(file.clone()))?;
    let now = wall_now()?;
    let publish = flag(&arguments, "--yes-publish-live");

    match mode {
        "preflight" => run_preflight(&adapter).await,
        "list" => run_list(&adapter).await,
        "draft" => run_draft(&adapter, file, now).await,
        "edit" => run_edit(&adapter, file, now).await,
        "paid" => run_paid(&adapter, file, now, price_argument(&arguments)?, publish).await,
        "delete-published" => run_delete_published(&adapter, file, now, publish).await,
        other => Err(format!(
            "unknown mode {other:?}; expected preflight, list, draft, edit, paid or \
             delete-published"
        )
        .into()),
    }
}
