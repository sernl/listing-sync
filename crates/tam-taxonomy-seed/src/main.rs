//! One-shot operator seeding of the taxonomy hub from the committed crawl
//! captures. Parsing, pairing and validation are pure (`tam-taxonomy`);
//! the durable side is `TaxonomyRepo`. Deterministic canonical ids make a
//! re-run an explicit no-op, reported as existing rather than re-inserted.
//!
//! Usage: tam-taxonomy-seed <db-url> <gb.json> <nz.json>

#![forbid(unsafe_code)]

use std::io::Read as _;

use tam_storage::TaxonomyRepo;
use tam_taxonomy::tes::{derive_crosswalk, parse_tree};
use tam_types::Timestamp;

fn read_file(path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut text = String::new();
    std::fs::File::open(path)?.read_to_string(&mut text)?;
    Ok(text)
}

#[expect(
    clippy::disallowed_methods,
    reason = "the seeder is a clock-reading process boundary; time enters the edge records as data from here"
)]
fn wall_now() -> Result<Timestamp, Box<dyn std::error::Error>> {
    Ok(Timestamp(i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let db_url = arguments.first().ok_or("missing db url")?;
    let gb_path = arguments.get(1).ok_or("missing GB capture path")?;
    let nz_path = arguments.get(2).ok_or("missing NZ capture path")?;

    let gb = parse_tree(&read_file(gb_path)?)?;
    let nz = parse_tree(&read_file(nz_path)?)?;
    let crosswalk = derive_crosswalk(&gb, &nz, wall_now()?)?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(db_url)
        .await?;
    let report = TaxonomyRepo::new(pool)
        .seed(&crosswalk.terms, &crosswalk.edges)
        .await?;

    eprintln!(
        "terms: {} inserted, {} existing; edges: {} inserted, {} existing",
        report.terms_inserted, report.terms_existing, report.edges_inserted, report.edges_existing,
    );
    eprintln!(
        "residue: {} GB-only, {} NZ-only, {} mismatched",
        crosswalk.residue.gb_only.len(),
        crosswalk.residue.nz_only.len(),
        crosswalk.residue.mismatched.len(),
    );
    Ok(())
}
