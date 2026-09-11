//! The two-tenant negative tests: row-level security, not the queries'
//! `WHERE org_id` clauses, keeps tenant A's rows out of tenant B's reads. The
//! raw probes deliberately omit any org filter so they fail if a policy is
//! dropped, disabled, or not FORCEd onto the owner. The catalogue tests also
//! prove the deferred payload trigger and per-tenant blob deduplication.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::{
    AgeInterval, CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration,
    TermKind, VocabularyId, VocabularyPath,
};
use tam_storage::{ProductFileSourceRepo, ProductRepo};
use tam_types::{
    CanonicalTermId, ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, InventoryId,
    ListingCopy, Observation, OrgId, PayloadSet, PriceIntent, ProductFile, ProductId, ScanOutcome,
    Timestamp, Title, Uuid,
};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const PRODUCT_1: ProductId = ProductId(Uuid([0x01; 16]));
const TERM_MATHS: CanonicalTermId = CanonicalTermId(Uuid([0x11; 16]));
const TERM_FRACTIONS: CanonicalTermId = CanonicalTermId(Uuid([0x12; 16]));

fn db_uuid(id: Uuid) -> uuid::Uuid {
    uuid::Uuid::from_bytes(id.0)
}

fn file(
    id_byte: u8,
    role: FileRole,
    kind: FileKind,
    hash_byte: u8,
    scan: ScanOutcome,
) -> ProductFile {
    ProductFile {
        id: FileId(Uuid([id_byte; 16])),
        role,
        kind,
        bytes: FileBytes::Held {
            hash: ContentHash([hash_byte; 32]),
            byte_len: 4,
            scan,
        },
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a malformed fixture is a broken test and should panic"
)]
fn sample_product(org: OrgId) -> CanonicalProduct {
    let interval = AgeInterval::new(7, 11).expect("7..11 is an ordered interval");
    CanonicalProduct {
        id: PRODUCT_1,
        org,
        title: Title("Fractions revision pack".to_owned()),
        body: ListingCopy {
            body: "A worked example pack.".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: Some(PayloadSet::new(
            file(
                0x21,
                FileRole::Payload,
                FileKind::Pdf,
                0x51,
                ScanOutcome::Pending,
            ),
            vec![file(
                0x22,
                FileRole::Payload,
                FileKind::Zip,
                0x51,
                ScanOutcome::Clean { at: Timestamp(2) },
            )],
        )),
        cover: Some(file(
            0x23,
            FileRole::Cover,
            FileKind::Image,
            0x52,
            ScanOutcome::Pending,
        )),
        previews: vec![],
        subjects: vec![TERM_MATHS, TERM_FRACTIONS],
        grades: GradeDeclaration {
            source: DeclarationSource::Seller,
            raw: vec![VocabularyPath {
                vocabulary: VocabularyId(InventoryId::Tes, TermKind::Phase),
                segments: vec!["primary".to_owned(), "ks2".to_owned()],
                native_id: None,
            }],
            derived: Some(interval),
        },
        price: PriceIntent::Free,
        // The declared arm is the fixture, so the aggregate round trip is
        // also the rights round trip: a grant is kept as the source's own
        // value, and a licence is a kind a term may have.
        rights: RightsDeclaration::Declared {
            source: VocabularyPath {
                vocabulary: VocabularyId(InventoryId::Tes, TermKind::Licence),
                segments: vec!["Creative Commons Attribution-ShareAlike".to_owned()],
                native_id: Some("CC-BY-SA".to_owned()),
            },
        },
        native_residue: vec![],
    }
}

async fn seed_fixture(pool: &PgPool) -> Result<(), sqlx::Error> {
    for (org, name) in [(ORG_A, "org-a"), (ORG_B, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(db_uuid(org.0))
            .bind(name)
            .execute(pool)
            .await?;
    }
    for (term, label) in [(TERM_MATHS, "mathematics"), (TERM_FRACTIONS, "fractions")] {
        sqlx::query(
            "INSERT INTO canonical_term (id, kind, parent, label) VALUES ($1, 'subject', NULL, $2)",
        )
        .bind(db_uuid(term.0))
        .bind(label)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Counts every visible row of the named table with NO org filter, under the
/// given pin. The table name comes from a fixed test-local set, never input.
async fn visible_rows(pool: &PgPool, table: &str, pin: Option<OrgId>) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    if let Some(org) = pin {
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(db_uuid(org.0).to_string())
            .execute(&mut *tx)
            .await?;
    }
    let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(count)
}

#[sqlx::test(migrations = "./migrations")]
async fn the_aggregate_round_trips(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let repo = ProductRepo::new(pool.clone());
    let product = sample_product(ORG_A);
    repo.insert(ORG_A, &product, Timestamp(1_756_000_000_000))
        .await
        .expect("tenant A inserts the aggregate");

    let record = repo
        .get(ORG_A, PRODUCT_1)
        .await
        .expect("tenant A reads back")
        .expect("the product exists for tenant A");
    assert_eq!(
        record.product, product,
        "the aggregate must survive the write-read round trip unchanged"
    );
    assert_eq!(
        record.created_at,
        Timestamp(1_756_000_000_000),
        "the write instant enters as data and survives"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_product_that_stated_no_grant_reads_back_as_having_stated_none(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let repo = ProductRepo::new(pool.clone());
    let mut product = sample_product(ORG_A);
    product.rights = RightsDeclaration::Unstated;
    repo.insert(ORG_A, &product, Timestamp(1))
        .await
        .expect("tenant A inserts the aggregate");

    let record = repo
        .get(ORG_A, PRODUCT_1)
        .await
        .expect("tenant A reads back")
        .expect("the product exists for tenant A");
    assert_eq!(
        record.product.rights,
        RightsDeclaration::Unstated,
        "an absent grant is the honest state of every product imported before the column \
         existed, and it must not read back as a plausible default"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_half_declared_grant_is_refused_by_the_database_and_not_only_by_the_type(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG_A.0).to_string())
        .execute(&pool)
        .await
        .expect("the tenant pin is set");
    let refused = sqlx::query(
        "INSERT INTO product \
         (org_id, id, title, body, price_kind, rights_state, rights_source_inventory, \
          created_at, updated_at) \
         VALUES ($1, $2, 'x', 'y', 'free', 'declared', 'tes', now(), now())",
    )
    .bind(db_uuid(ORG_A.0))
    .bind(db_uuid(Uuid([0x5A; 16])))
    .execute(&pool)
    .await;
    assert!(
        refused.is_err(),
        "the encoder makes the illegal combination unrepresentable, and the CHECK refuses it \
         for anything that writes the row directly"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn two_files_sharing_a_hash_share_one_blob(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG_A, &sample_product(ORG_A), Timestamp(1))
        .await
        .expect("tenant A inserts the aggregate");

    assert_eq!(
        visible_rows(&pool, "blob", Some(ORG_A))
            .await
            .expect("the pinned probe runs"),
        2,
        "three files over two distinct hashes deduplicate to two blobs"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_payload_less_product_commits_alone_and_not_with_a_mapping(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG_A.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO product \
         (org_id, id, title, body, body_format, price_kind, price_minor_units, \
          price_currency, rights_state, created_at, updated_at) \
         VALUES ($1, $2, 't', 'b', 'markdown', 'free', NULL, NULL, 'unstated', now(), now())",
    )
    .bind(db_uuid(ORG_A.0))
    .bind(db_uuid(PRODUCT_1.0))
    .execute(&mut *tx)
    .await
    .expect("the bare row inserts; the deferred trigger has not run yet");
    tx.commit()
        .await
        .expect("a product no mapping names may carry no payload at all (D32)");

    // The other direction — a mapping onto a product with no payload — is
    // `mapping_add::a_mapping_onto_a_resource_with_no_file_is_refused_by_the_database`,
    // which drives it through the repository rather than composing a `mapping`
    // insert by hand here; the column list is the repository's to know.
}

#[sqlx::test(migrations = "./migrations")]
async fn tenant_b_sees_nothing_of_tenant_a(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG_A, &sample_product(ORG_A), Timestamp(1))
        .await
        .expect("tenant A inserts the aggregate");

    // One observation for A, so the probe below has a row to fail to hide.
    // Written through the repository rather than by raw SQL, because a table
    // whose only rows arrived by a path that bypasses the policy would prove
    // nothing about the path that does not.
    let mut tx = pool.begin().await.expect("a transaction opens");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG_A.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("the tenant pins");
    sqlx::query(
        "INSERT INTO device \
         (org_id, id, name, os, arch, app_version, first_seen_at, last_seen_at) \
         VALUES ($1, 'device-a', 'laptop', 'linux', 'x86_64', '0.2.0', now(), now())",
    )
    .bind(db_uuid(ORG_A.0))
    .execute(&mut *tx)
    .await
    .expect("the device inserts");
    tx.commit().await.expect("the device commits");
    ProductFileSourceRepo::new(pool.clone())
        .observe(
            ORG_A,
            FileId(Uuid([0x21; 16])),
            &Observation {
                device: "device-a".to_owned(),
                hash: ContentHash([0x51; 32]),
                byte_len: 4,
                scan: ScanOutcome::Pending,
                observed_at: Timestamp(1),
            },
            Timestamp(1),
        )
        .await
        .expect("tenant A records an observation");

    let found = repo
        .get(ORG_A, PRODUCT_1)
        .await
        .expect("tenant A reads back");
    assert!(
        found.is_some(),
        "positive control: tenant A must see its own row, or every assertion below is vacuous"
    );

    for table in [
        "product",
        "product_file",
        "product_file_observation",
        "blob",
        "grade_declaration",
    ] {
        let a_rows = visible_rows(&pool, table, Some(ORG_A))
            .await
            .expect("the pinned probe runs");
        assert!(
            a_rows > 0,
            "probe control: the unfiltered {table} probe must see A's rows under A's pin"
        );
        assert_eq!(
            visible_rows(&pool, table, Some(ORG_B))
                .await
                .expect("the pinned probe runs"),
            0,
            "row-level security must hide A's {table} rows from B even without a WHERE clause"
        );
    }
    let listed = repo.list(ORG_B).await.expect("tenant B lists");
    assert!(
        listed.is_empty(),
        "the repository surface must return nothing for tenant B"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unpinned_connection_sees_an_empty_table(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let repo = ProductRepo::new(pool.clone());
    repo.insert(ORG_A, &sample_product(ORG_A), Timestamp(1))
        .await
        .expect("tenant A inserts the aggregate");

    assert_eq!(
        visible_rows(&pool, "product", None)
            .await
            .expect("the unpinned probe runs"),
        0,
        "a connection that never declared its tenant must fail closed and see nothing"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn tenant_b_cannot_write_a_row_into_tenant_a(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(db_uuid(ORG_B.0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let smuggled = sqlx::query(
        "INSERT INTO product \
         (org_id, id, title, body, price_kind, price_minor_units, price_currency, \
          rights_state, created_at, updated_at) \
         VALUES ($1, $2, 't', 'b', 'free', NULL, NULL, 'unstated', now(), now())",
    )
    .bind(db_uuid(ORG_A.0))
    .bind(db_uuid(PRODUCT_1.0))
    .execute(&mut *tx)
    .await;
    assert!(
        smuggled.is_err(),
        "the WITH CHECK half of the policy must refuse a write into another tenant"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_org_mismatched_aggregate_is_refused(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let repo = ProductRepo::new(pool.clone());
    let product = sample_product(ORG_B);
    let refused = repo.insert(ORG_A, &product, Timestamp(1)).await;
    assert!(
        matches!(refused, Err(tam_storage::StorageError::OrgMismatch)),
        "an aggregate naming organisation B cannot be written under a pin for A"
    );
}
