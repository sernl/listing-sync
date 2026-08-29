//! One-shot operator seeding of the taxonomy hub from the committed crawl
//! captures and polled vocabularies. Parsing, pairing and validation are pure
//! (`tam-taxonomy`); the durable side is `TaxonomyRepo`. Deterministic
//! canonical ids make a re-run an explicit no-op, reported as existing rather
//! than re-inserted.
//!
//! Usage: tam-taxonomy-seed <db-url> <gb.json> <nz.json> <tpt-vocab.json> <tes-vocab.json>
//!
//! The last two arguments are optional. Without them the run seeds the
//! subject and topic crosswalk alone, which is what it did before the grade
//! axis existed; with them it also seeds the grade relation and the Tes
//! licence vocabulary.

#![forbid(unsafe_code)]

use std::io::Read as _;

use tam_storage::TaxonomyRepo;
use tam_taxonomy::grades::derive_grade_crosswalk;
use tam_taxonomy::licences::derive_licence_crosswalk;
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
    let vocabularies = match (arguments.get(3), arguments.get(4)) {
        (Some(tpt), Some(tes)) => Some((read_file(tpt)?, read_file(tes)?)),
        (None, None) => None,
        _ => return Err("both vocabulary paths are needed, or neither".into()),
    };

    let gb = parse_tree(&read_file(gb_path)?)?;
    let nz = parse_tree(&read_file(nz_path)?)?;
    let at = wall_now()?;
    let crosswalk = derive_crosswalk(&gb, &nz, at)?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(db_url)
        .await?;
    let repo = TaxonomyRepo::new(pool);
    let report = repo.seed(&crosswalk.terms, &crosswalk.edges).await?;

    eprintln!(
        "subjects and topics: {} terms inserted, {} existing; {} edges inserted, {} existing",
        report.terms_inserted, report.terms_existing, report.edges_inserted, report.edges_existing,
    );
    eprintln!(
        "residue: {} GB-only, {} NZ-only, {} mismatched",
        crosswalk.residue.gb_only.len(),
        crosswalk.residue.nz_only.len(),
        crosswalk.residue.mismatched.len(),
    );

    let mut ambiguous = report.ambiguous_terms;
    if let Some((tpt_json, tes_json)) = vocabularies {
        let grades = derive_grade_crosswalk(&tpt_json, &tes_json, at)?;
        let seeded = repo.seed(&grades.terms, &grades.edges).await?;
        let absences = repo.seed_no_counterparts(&grades.no_counterparts).await?;
        eprintln!(
            "grades: {} terms inserted, {} existing; {} edges inserted, {} existing; \
             {} absences inserted, {} existing",
            seeded.terms_inserted,
            seeded.terms_existing,
            seeded.edges_inserted,
            seeded.edges_existing,
            absences.inserted,
            absences.existing,
        );
        for row in &grades.uncovered {
            eprintln!(
                "grades: year group {} ({}) declares ages {:?} that no Tes GB band covers; \
                 recorded as having no counterpart rather than snapped to the nearest band",
                row.year_group, row.label, row.human_ages,
            );
        }

        let licences = derive_licence_crosswalk(&tes_json, at)?;
        let seeded = repo.seed(&licences.terms, &licences.edges).await?;
        ambiguous = seeded.ambiguous_terms;
        eprintln!(
            "licences: {} terms inserted, {} existing; {} edges inserted, {} existing",
            seeded.terms_inserted,
            seeded.terms_existing,
            seeded.edges_inserted,
            seeded.edges_existing,
        );
    }

    // A defect figure rather than an outcome: a term the relation projects two
    // ways is a question no product can answer, and the kill gate reads it as
    // a number rather than discovering it as a re-raised queue item.
    eprintln!("relation: {ambiguous} terms project ambiguously");
    Ok(())
}
