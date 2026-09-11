//! One-shot operator seeding of the taxonomy hub from the committed crawl
//! captures and polled vocabularies. Parsing, pairing and validation are pure
//! (`tam-taxonomy`); the durable side is `TaxonomyRepo`. Deterministic
//! canonical ids make a re-run an explicit no-op, reported as existing rather
//! than re-inserted.
//!
//! Every derivation's edges pass `check_native_ids` before any of them is
//! written. A native identifier the target marketplace did not issue is a
//! wrong tag on a live listing, and this is the last point at which the whole
//! relation can be checked at once rather than one cross-listing at a time.
//! It is not the only path that authors an edge: the reconciliation queue's
//! resolution writes one from the API, and runs the same check on it there.
//!
//! Usage: tam-taxonomy-seed <db-url> <gb.json> <nz.json> <tpt-vocab.json>
//! <tes-vocab.json> <subject-pairs.json> <resource-type-pairs.json>
//!
//! Every argument is required. The TPT vocabulary and the authored pairing
//! were optional while the canonical subject and topic axes were minted from
//! Tes alone; they are not optional now that TPT is the base, because a run
//! without them would seed a relation in which no TPT facet resolves, which
//! is a half-seeded hub rather than a smaller one.

#![forbid(unsafe_code)]

use std::io::Read as _;

use tam_storage::TaxonomyRepo;
use tam_taxonomy::grades::derive_grade_crosswalk;
use tam_taxonomy::licences::derive_licence_crosswalk;
use tam_taxonomy::provenance::check_native_ids;
use tam_taxonomy::resource_types::derive_resource_type_crosswalk;
use tam_taxonomy::subjects::derive_subject_crosswalk;
use tam_taxonomy::tes::parse_tree;
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
    let tree_path = arguments.get(1).ok_or("missing Tes capture path")?;
    let tpt_json = read_file(arguments.get(2).ok_or("missing TPT vocabulary path")?)?;
    let tes_json = read_file(arguments.get(3).ok_or("missing Tes vocabulary path")?)?;
    let pairs_json = read_file(arguments.get(4).ok_or("missing subject pairing path")?)?;
    let resource_pairs_json = read_file(
        arguments
            .get(5)
            .ok_or("missing resource-type pairing path")?,
    )?;

    let tree = parse_tree(&read_file(tree_path)?)?;
    let at = wall_now()?;
    let crosswalk = derive_subject_crosswalk(&tree, &tpt_json, &pairs_json, at)?;
    check_native_ids(&crosswalk.edges)?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(db_url)
        .await?;
    let repo = TaxonomyRepo::new(pool);
    let report = repo.seed(&crosswalk.terms, &crosswalk.edges).await?;
    let absences = repo
        .seed_no_counterparts(&crosswalk.no_counterparts)
        .await?;

    eprintln!(
        "subjects and topics: {} terms inserted, {} existing; {} edges inserted, {} existing; \
         {} absences inserted, {} existing",
        report.terms_inserted,
        report.terms_existing,
        report.edges_inserted,
        report.edges_existing,
        absences.inserted,
        absences.existing,
    );
    eprintln!(
        "residue (Tes tree): {} withdrawn",
        crosswalk.tes_residue.mismatched.len(),
    );
    eprintln!(
        "residue (TPT base): {} facets reach no Tes node, {} Tes nodes reach no facet, \
         {} withdrawn or contested, {} hidden facets seeded neither way",
        crosswalk.residue.tpt_only.len(),
        crosswalk.residue.tes_only.len(),
        crosswalk.residue.mismatched.len(),
        crosswalk.residue.skipped_hidden.len(),
    );
    // The pairing table is authored and awaiting founder confirmation, so the
    // rows it forced are printed rather than counted: each is a claim one row
    // made that another row overrode.
    for mismatch in &crosswalk.residue.mismatched {
        eprintln!(
            "pairing: Tes {} ({}) {}{}",
            mismatch.node.native_id,
            mismatch.node.description,
            mismatch.reason,
            mismatch
                .counterpart
                .as_ref()
                .map_or_else(String::new, |counterpart| format!(
                    ", against TPT {}",
                    counterpart.native_id
                )),
        );
    }

    let grades = derive_grade_crosswalk(&tpt_json, &tes_json, at)?;
    check_native_ids(&grades.edges)?;
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
    check_native_ids(&licences.edges)?;
    let seeded = repo.seed(&licences.terms, &licences.edges).await?;
    eprintln!(
        "licences: {} terms inserted, {} existing; {} edges inserted, {} existing",
        seeded.terms_inserted, seeded.terms_existing, seeded.edges_inserted, seeded.edges_existing,
    );

    let resources = derive_resource_type_crosswalk(&tpt_json, &tes_json, &resource_pairs_json, at)?;
    check_native_ids(&resources.edges)?;
    let seeded = repo.seed(&resources.terms, &resources.edges).await?;
    let ambiguous = seeded.ambiguous_terms;
    eprintln!(
        "resource types: {} terms inserted, {} existing; {} edges inserted, {} existing",
        seeded.terms_inserted, seeded.terms_existing, seeded.edges_inserted, seeded.edges_existing,
    );
    eprintln!(
        "resource types: {} facets reach no Tes value, {} hidden facets seeded neither way, \
         {} Tes values no facet reaches",
        resources.unreached.len(),
        resources.skipped_hidden.len(),
        resources.unclaimed_targets.len(),
    );
    for target in &resources.unclaimed_targets {
        eprintln!(
            "resource types: Tes mainType {} ({}) is reached by no TPT facet",
            target.native_id, target.description,
        );
    }

    // A defect figure rather than an outcome: a term the relation projects two
    // ways is a question no product can answer, and the kill gate reads it as
    // a number rather than discovering it as a re-raised queue item.
    eprintln!("relation: {ambiguous} terms project ambiguously");
    Ok(())
}
