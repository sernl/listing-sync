//! What a template is, and who may see one.
//!
//! Three halves, and the first is the one the rest are measured against. Two
//! tenants each keep a template, and neither reaches the other's through any
//! of the five statements — if that failed, every other assertion here would
//! be about a table with no fence, and the operator read at the foot of the
//! file would prove nothing because every caller would already be crossing.
//!
//! The second is the column's own CHECK constraints, asserted by driving the
//! table directly. The API bounds refuse first and would otherwise hide them,
//! and the point of a CHECK is that it holds for a writer that is not the API.
//!
//! The third is the naming and the ceiling: one template per folded name per
//! organisation, and a shelf that fills.

#![cfg(feature = "pg-tests")]

use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Postgres, Transaction};
use tam_storage::{
    NewResourceTemplate, ResourceTemplateRepo, TemplateChange, TemplateEdit, TemplateWrite,
    TEMPLATES_PER_ORG_MAX,
};
use tam_types::{OrgId, Timestamp, Uuid};

mod common;
use common::{seed_org_a, ORG_A};

const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const MADE: Timestamp = Timestamp(1_000);
const EDITED: Timestamp = Timestamp(9_000);

async fn seed_org_b(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-b', now())")
        .bind(uuid::Uuid::from_bytes(ORG_B.0 .0))
        .execute(pool)
        .await?;
    Ok(())
}

/// A transaction with the tenant pin set, for the direct statements that have
/// to answer as the column rather than as the repository.
async fn pinned(pool: &PgPool, org: OrgId) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(org.0 .0).to_string())
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// The operator role's own connection to this test's database. The read below
/// is unpinned, which under forced row-level security is the application role
/// seeing nothing at all — so a cross-tenant read through the wrong pool would
/// be empty rather than wrong, and prove nothing.
async fn backoffice_pool(app: &PgPool) -> Result<PgPool, sqlx::Error> {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await?;
    PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_backoffice:tam_backoffice_dev@127.0.0.1:5433/{database}"
        ))
        .await
}

fn draft(title: &str) -> serde_json::Value {
    serde_json::json!({ "name": title, "grades": ["1st-grade"] })
}

fn template<'a>(id: Uuid, name: &'a str, held: &'a serde_json::Value) -> NewResourceTemplate<'a> {
    NewResourceTemplate {
        id,
        name,
        draft: held,
        created_at: MADE,
    }
}

const TEMPLATE_A: Uuid = Uuid([0x0A; 16]);
const TEMPLATE_B: Uuid = Uuid([0x0B; 16]);

#[sqlx::test(migrations = "./migrations")]
async fn each_tenant_keeps_its_own_templates_and_neither_reaches_the_other(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_org_b(&pool).await.expect("org b seeds");
    let repo = ResourceTemplateRepo::new(pool.clone());

    let mine = draft("Phonics pack");
    let theirs = draft("Fractions unit");
    for (org, id, name, held) in [
        (ORG_A, TEMPLATE_A, "Phonics", &mine),
        (ORG_B, TEMPLATE_B, "Fractions", &theirs),
    ] {
        assert!(
            matches!(
                repo.create(org, &template(id, name, held)).await,
                Ok(TemplateWrite::Saved(_))
            ),
            "each tenant saves its own template"
        );
    }

    let listed = repo.list(ORG_A).await.expect("the listing runs");
    assert_eq!(
        listed
            .iter()
            .map(|summary| summary.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Phonics"],
        "a listing under one pin holds that organisation's templates and no other's"
    );
    assert_eq!(
        repo.get(ORG_A, TEMPLATE_A)
            .await
            .expect("the read runs")
            .map(|record| record.draft),
        Some(mine.clone()),
        "and the draft the listing leaves in the column is what the fetch carries"
    );

    assert_eq!(
        repo.get(ORG_A, TEMPLATE_B).await.expect("the read runs"),
        None,
        "another organisation's template is absent rather than readable, and the \
         identifier is the one thing a caller could guess"
    );
    assert_eq!(
        repo.update(ORG_A, TEMPLATE_B, &TemplateEdit::Rename("stolen"), EDITED,)
            .await
            .expect("the edit runs"),
        TemplateChange::Missing,
        "and it cannot be renamed across the fence"
    );
    assert!(
        !repo
            .delete(ORG_A, TEMPLATE_B)
            .await
            .expect("the delete runs"),
        "nor removed across it"
    );
    assert!(
        repo.get(ORG_B, TEMPLATE_B)
            .await
            .expect("the read runs")
            .is_some(),
        "and the template the other tenant owns is still there afterwards"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_column_refuses_what_no_writer_may_store(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let long_name = "n".repeat(81);
    // Past the column's own 131072 backstop, which is deliberately looser than
    // the route's 65536: this row asserts the backstop, not the route's bound.
    let long_draft = format!("{{\"description\": \"{}\"}}", "a".repeat(140_000));
    for (name, held, created, updated, refused) in [
        ("", "{}", MADE.0, MADE.0, "a template with no name"),
        (
            long_name.as_str(),
            "{}",
            MADE.0,
            MADE.0,
            "a name past eighty characters",
        ),
        (
            "Array",
            "[]",
            MADE.0,
            MADE.0,
            "a draft that is not a JSON object",
        ),
        (
            "Scalar",
            "7",
            MADE.0,
            MADE.0,
            "a draft that is a bare number",
        ),
        (
            "Huge",
            long_draft.as_str(),
            MADE.0,
            MADE.0,
            "a draft past the column's own backstop",
        ),
        (
            "Backwards",
            "{}",
            EDITED.0,
            MADE.0,
            "an edit that predates the row it edits",
        ),
    ] {
        let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
        let written = sqlx::query(
            "INSERT INTO resource_template \
             (org_id, id, name, draft, created_at, updated_at) \
             VALUES ($1, gen_random_uuid(), $2, $3::jsonb, \
                     to_timestamp($4 / 1000.0), to_timestamp($5 / 1000.0))",
        )
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .bind(name)
        .bind(held)
        .bind(created)
        .bind(updated)
        .execute(&mut *tx)
        .await;
        assert!(
            written.is_err(),
            "the column refuses this whoever writes it: {refused}"
        );
    }
}

/// The column's backstop, pinned on both sides.
///
/// The refusal row in the test above proves a draft far past the bound is
/// refused, which a much tighter CHECK would also do — including one set to the
/// route's own 65536, which is what M2 was. Only the accepted half tells 131072
/// apart from that, so it is the half that keeps the route's arithmetic true:
/// the route admits 65536 compact bytes, Postgres re-renders them as at most
/// 98304, and this is the number that has to leave room for it.
#[sqlx::test(migrations = "./migrations")]
async fn the_column_s_backstop_is_the_number_the_route_s_arithmetic_assumes(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    // Postgres renders `{"description": "<fill>"}`: eighteen bytes of wrapper
    // plus the space it inserts after the colon, so the rendering is fill + 19.
    for (fill, accepted) in [(131_072 - 19, true), (131_072 - 18, false)] {
        let held = format!("{{\"description\":\"{}\"}}", "a".repeat(fill));
        let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
        let written = sqlx::query(
            "INSERT INTO resource_template \
             (org_id, id, name, draft, created_at, updated_at) \
             VALUES ($1, gen_random_uuid(), $2, $3::jsonb, now(), now())",
        )
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .bind(format!("row of {fill}"))
        .bind(held)
        .execute(&mut *tx)
        .await;
        assert_eq!(
            written.is_ok(),
            accepted,
            "the backstop is 131072 octets of the rendering Postgres itself makes, \
             and a draft filled to {fill} renders as {}",
            fill + 19
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn one_template_per_folded_name_per_organisation(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_org_b(&pool).await.expect("org b seeds");
    let repo = ResourceTemplateRepo::new(pool.clone());
    let held = draft("Phonics pack");

    assert!(
        matches!(
            repo.create(ORG_A, &template(TEMPLATE_A, "Autumn Unit", &held))
                .await,
            Ok(TemplateWrite::Saved(_))
        ),
        "the first template of a name lands"
    );
    assert_eq!(
        repo.create(ORG_A, &template(TEMPLATE_B, "autumn unit", &held))
            .await
            .expect("the second create runs"),
        TemplateWrite::NameTaken,
        "a seller who types the same name in another casing means the template they have"
    );
    assert!(
        matches!(
            repo.create(ORG_B, &template(TEMPLATE_B, "Autumn Unit", &held))
                .await,
            Ok(TemplateWrite::Saved(_))
        ),
        "and the name is one organisation's own, not the platform's"
    );

    let second = Uuid([0x0C; 16]);
    assert!(
        matches!(
            repo.create(ORG_A, &template(second, "Spring Unit", &held))
                .await,
            Ok(TemplateWrite::Saved(_))
        ),
        "a second name lands"
    );
    assert_eq!(
        repo.update(ORG_A, second, &TemplateEdit::Rename("AUTUMN UNIT"), EDITED,)
            .await
            .expect("the rename runs"),
        TemplateChange::NameTaken,
        "and renaming onto a name already held is the index's refusal rather than a fault"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_edit_replaces_only_the_part_it_names(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ResourceTemplateRepo::new(pool.clone());
    let first = draft("Phonics pack");
    repo.create(ORG_A, &template(TEMPLATE_A, "Phonics", &first))
        .await
        .expect("the template saves");

    let renamed = repo
        .update(
            ORG_A,
            TEMPLATE_A,
            &TemplateEdit::Rename("Phonics, revised"),
            EDITED,
        )
        .await
        .expect("the rename runs");
    let TemplateChange::Saved(record) = renamed else {
        panic!("a rename of a template this organisation holds saves");
    };
    assert_eq!(
        (record.name.as_str(), &record.draft),
        ("Phonics, revised", &first),
        "a rename leaves the draft exactly as it was"
    );
    assert_eq!(
        (record.created_at, record.updated_at),
        (MADE, EDITED),
        "and moves only the instant that records the edit"
    );

    let second = draft("Phonics pack, second edition");
    let replaced = repo
        .update(ORG_A, TEMPLATE_A, &TemplateEdit::Redraft(&second), EDITED)
        .await
        .expect("the draft replacement runs");
    let TemplateChange::Saved(record) = replaced else {
        panic!("replacing the draft of a template this organisation holds saves");
    };
    assert_eq!(
        (record.name.as_str(), &record.draft),
        ("Phonics, revised", &second),
        "and replacing the draft leaves the name it was saved under"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_shelf_fills_at_the_ceiling_the_unpaged_listing_rests_on(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let mut tx = pinned(&pool, ORG_A).await.expect("the pin sets");
    sqlx::query(
        "INSERT INTO resource_template \
         (org_id, id, name, draft, created_at, updated_at) \
         SELECT $1, gen_random_uuid(), 'seeded ' || n, '{}'::jsonb, now(), now() \
           FROM generate_series(1, $2) AS n",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(i32::try_from(TEMPLATES_PER_ORG_MAX).expect("the ceiling fits an int"))
    .execute(&mut *tx)
    .await
    .expect("the shelf fills");
    tx.commit().await.expect("the seeding commits");

    let held = draft("One too many");
    assert_eq!(
        ResourceTemplateRepo::new(pool.clone())
            .create(ORG_A, &template(TEMPLATE_A, "One too many", &held))
            .await
            .expect("the create runs"),
        TemplateWrite::TooMany,
        "the ceiling is what bounds an answer that has no cursor, so it has to refuse"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_operator_role_cannot_reach_a_seller_s_drafts_at_all(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    seed_org_b(&pool).await.expect("org b seeds");
    let repo = ResourceTemplateRepo::new(pool.clone());
    let held = draft("Phonics pack");
    for (org, id, name) in [
        (ORG_A, TEMPLATE_A, "Phonics"),
        (ORG_B, TEMPLATE_B, "Fractions"),
    ] {
        repo.create(org, &template(id, name, &held))
            .await
            .expect("the template saves");
    }

    // Rows exist under both tenants, so an empty answer below would be the
    // grant's doing rather than an empty table.
    let backoffice = backoffice_pool(&pool).await.expect("the role connects");
    for statement in [
        "SELECT count(*) FROM resource_template",
        "SELECT name FROM resource_template",
        "SELECT draft FROM resource_template",
    ] {
        let denied = sqlx::query(statement).fetch_all(&backoffice).await;
        assert!(
            denied.is_err(),
            "migration 0056 grants this role nothing on resource_template: a template \
             is listing copy the seller has not published, and no operator route reads \
             one: {statement}"
        );
    }
}

/// The rename the route's own documentation promises, which the folded-name
/// refusal above would break if it were read rather than left to the index.
///
/// An implementation that compared folded names before writing would refuse
/// this while passing every other test here, and a seller correcting the
/// casing of a name they already hold would be told it is taken.
#[sqlx::test(migrations = "./migrations")]
async fn a_template_may_be_renamed_into_another_casing_of_its_own_name(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ResourceTemplateRepo::new(pool.clone());
    let held = draft("Phonics pack");
    repo.create(ORG_A, &template(TEMPLATE_A, "Autumn Unit", &held))
        .await
        .expect("the template saves");

    let renamed = repo
        .update(
            ORG_A,
            TEMPLATE_A,
            &TemplateEdit::Rename("AUTUMN UNIT"),
            EDITED,
        )
        .await
        .expect("the rename runs");
    let TemplateChange::Saved(record) = renamed else {
        panic!("a template renamed into another casing of its own name is not taken");
    };
    assert_eq!(
        record.name.as_str(),
        "AUTUMN UNIT",
        "the folded index sees the same row, so the refusal is the seller's own \
         correction landing rather than a conflict with themselves"
    );
}

/// A clock that steps backwards between a create and an edit is an ordinary
/// rename, not a fault.
#[sqlx::test(migrations = "./migrations")]
async fn an_edit_stamped_before_the_row_was_made_is_clamped_rather_than_refused(pool: PgPool) {
    seed_org_a(&pool).await.expect("org a seeds");
    let repo = ResourceTemplateRepo::new(pool.clone());
    let held = draft("Phonics pack");
    repo.create(ORG_A, &template(TEMPLATE_A, "Phonics", &held))
        .await
        .expect("the template saves");

    let stepped_back = Timestamp(MADE.0 - 60_000);
    let edited = repo
        .update(
            ORG_A,
            TEMPLATE_A,
            &TemplateEdit::Rename("Phonics II"),
            stepped_back,
        )
        .await
        .expect("the rename runs rather than raising the CHECK as a fault");
    let TemplateChange::Saved(record) = edited else {
        panic!("the rename lands");
    };
    assert_eq!(
        (record.created_at, record.updated_at),
        (MADE, MADE),
        "clamped to the instant the row was made, which is the one statement about \
         a backwards clock that is not a lie"
    );
}
