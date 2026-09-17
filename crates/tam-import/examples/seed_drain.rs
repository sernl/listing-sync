//! Puts real import-drain measurements in the ledger so the operator page can
//! be rendered against rows Postgres actually holds, without contacting a
//! marketplace.
//!
//! Usage: seed_drain <db-url> <org-hex> <source> <target> <count> [--barren-first]
//!
//! The writer is real and the counts are chosen. Every row goes through
//! `record_drain_report`, so each is a `job` row and an `ImportDrainMeasured`
//! event written by the repositories the operator import writes through, in
//! the payload spelling
//! `the_drain_report_lands_as_a_job_event_the_client_can_read` pins byte for
//! byte. What is invented is the six counts: deriving them from a real import
//! would need a cassette, a taxonomy seed, an object store and a fixture
//! archive to answer a question about how a table draws.
//!
//! The series is shaped for what the page has to show. The share falls from
//! the first run to the last, because the kill gate compares a tenant's first
//! migration against its tenth and a flat series leaves that window nothing to
//! report; most runs carry unmapped terms, which is the column drawn in the
//! warn colour; and `--barren-first` makes the first run project no canonical
//! term at all, which is the run whose share is unmeasurable rather than zero
//! and the case the gate sentence has to state rather than count.
//!
//! Development only. Nothing in the product runs this, and the job it opens
//! carries no items, so the worker's lease scan cannot claim it.

#![forbid(unsafe_code)]

use sqlx::PgPool;
use tam_import::{record_drain_report, DrainTotals, ImportRun};
use tam_types::{InventoryId, OrgId, Timestamp, Uuid};

const USAGE: &str =
    "usage: seed_drain <db-url> <org-hex> <source> <target> <count> [--barren-first]";

/// The 32-character form the operator import takes, hyphens optional.
fn org_from_hex(raw: &str) -> Result<OrgId, Box<dyn std::error::Error>> {
    let hex: String = raw.chars().filter(|glyph| *glyph != '-').collect();
    if hex.len() != 32 {
        return Err("org hex must be 32 characters, hyphens optional".into());
    }
    let mut array = [0u8; 16];
    for (index, slot) in array.iter_mut().enumerate() {
        let start = index * 2;
        let pair = hex.get(start..start + 2).ok_or("org hex is malformed")?;
        *slot = u8::from_str_radix(pair, 16)?;
    }
    Ok(OrgId(Uuid(array)))
}

/// Read through serde rather than a match, so the names accepted here are the
/// same names the recorded payload carries and cannot drift from them.
fn inventory(name: &str) -> Result<InventoryId, Box<dyn std::error::Error>> {
    Ok(serde_json::from_value(serde_json::Value::String(
        name.to_owned(),
    ))?)
}

#[expect(
    clippy::disallowed_methods,
    reason = "the seeder is a clock-reading process boundary; time enters the rows as data from here"
)]
fn wall_now() -> Result<Timestamp, Box<dyn std::error::Error>> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

/// The counts one run of the series states.
///
/// The share is `items_new / (covered + new + already_open)`, which here is
/// `12 / (7 * index + 12)`: a fixed number of new questions against a
/// crosswalk that fills in as the runs go by. It falls fast over the first ten
/// runs and keeps falling after them, which is what the gate window has to be
/// able to see. Deriving it from the run's own position rather than from the
/// series length is deliberate — a shape tuned to `count` would flatten to
/// nothing across the first ten runs of a long series, and the first ten are
/// the only ones the gate reads.
fn totals_for(index: u64, barren_first: bool) -> DrainTotals {
    if barren_first && index == 0 {
        // Nothing covered, nothing raised and nothing already open: the run
        // projected no canonical term, so its share is unmeasurable rather
        // than a drained-looking zero.
        return DrainTotals {
            rows: 4,
            terms_seen: 9,
            terms_unmapped: 2,
            ..DrainTotals::default()
        };
    }
    let unmapped = index % 3;
    DrainTotals {
        rows: 4 + index,
        terms_seen: 12 + index * 7 + unmapped,
        terms_unmapped: unmapped,
        terms_covered: index * 6,
        items_new: 12,
        items_already_open: index,
    }
}

async fn seed(
    run: &mut ImportRun,
    count: u64,
    barren_first: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    for index in 0..count {
        run.now = wall_now()?;
        let totals = totals_for(index, barren_first);
        let job = record_drain_report(run, totals).await?;
        eprintln!(
            "run {of} of {count}: {new} new, {open} already open, {covered} covered, \
             {unmapped} unmapped over {seen} term use(s) -> job {job}",
            of = index + 1,
            new = totals.items_new,
            open = totals.items_already_open,
            covered = totals.terms_covered,
            unmapped = totals.terms_unmapped,
            seen = totals.terms_seen,
            job = job.0.to_hyphenated(),
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let db_url = arguments.first().ok_or(USAGE)?;
    let org = org_from_hex(arguments.get(1).ok_or(USAGE)?)?;
    let source = inventory(arguments.get(2).ok_or(USAGE)?)?;
    let target = inventory(arguments.get(3).ok_or(USAGE)?)?;
    let count: u64 = arguments.get(4).ok_or(USAGE)?.parse()?;
    if count == 0 {
        return Err("a series of no runs writes nothing".into());
    }
    let barren_first = arguments.iter().any(|word| word == "--barren-first");
    let mut run = ImportRun {
        request: None,
        pool: PgPool::connect(db_url).await?,
        org,
        source,
        target: Some(target),
        now: wall_now()?,
    };
    seed(&mut run, count, barren_first).await?;
    eprintln!(
        "seeded {count} drain measurement(s) for {org_hex}, {source:?} into {target:?}",
        org_hex = org.0.to_hyphenated(),
    );
    Ok(())
}
