//! The TPT-base sidecar: the round trip, the tenant fence, and the two
//! absences that mean different things.
//!
//! Every property here is a property of the write, not of the route above it.
//! The request-level counterpart is `tam-api/tests/catalogue_flow.rs`.

#![cfg(feature = "pg-tests")]

use sqlx::PgPool;
use tam_domain::product::{
    AnswerKey, CategoryGroup, CopyrightDeclaration, DetailGroup, FacetSlug, ListingStatus,
    StandardAlignment, StandardsFramework, TaxCode, TeachingDuration, ThumbnailMode, UploadRef,
};
use tam_domain::{CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration};
use tam_storage::{ProductRepo, TptBaseRecord, TptBaseRepo};
use tam_types::{
    ContentHash, CopyFormat, FileId, FileKind, FileRole, ListingCopy, OrgId, PayloadSet,
    PriceIntent, ProductFile, ProductId, ScanOutcome, Timestamp, Title, Uuid,
};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const ORG_B: OrgId = OrgId(Uuid([0xBB; 16]));
const PRODUCT_A: ProductId = ProductId(Uuid([0x11; 16]));
const PRODUCT_B: ProductId = ProductId(Uuid([0x22; 16]));
const AT: Timestamp = Timestamp(1_700_000_000_000);
const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate; a broken fixture should panic"
)]
async fn provision(pool: &PgPool) {
    let products = ProductRepo::new(pool.clone());
    for (org, name, product) in [(ORG_A, "org-a", PRODUCT_A), (ORG_B, "org-b", PRODUCT_B)] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(pool)
            .await
            .expect("the fixture org inserts");
        // Through the repository rather than by raw insert: the deferred
        // `assert_product_has_payload` trigger fires at commit, so a product
        // written without its payload rows is refused there rather than here.
        products
            .insert(org, &sample(org, product), AT)
            .await
            .expect("the fixture product inserts");
    }
}

fn sample(org: OrgId, id: ProductId) -> CanonicalProduct {
    CanonicalProduct {
        id,
        org,
        title: Title("a resource".to_owned()),
        body: ListingCopy {
            body: "about it".to_owned(),
            format: CopyFormat::Markdown,
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([0x31; 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: ContentHash([0x51; 32]),
                byte_len: 2048,
                scan: ScanOutcome::Clean { at: AT },
            },
            vec![],
        ),
        cover: None,
        previews: vec![],
        subjects: vec![],
        grades: GradeDeclaration {
            source: DeclarationSource::Seller,
            raw: vec![],
            derived: None,
        },
        price: PriceIntent::Free,
        rights: RightsDeclaration::Unstated,
        native_residue: vec![],
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate"
)]
fn slugs(names: &[&str]) -> Vec<FacetSlug> {
    names
        .iter()
        .map(|name| FacetSlug::new(name).expect("a non-empty slug"))
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not free helpers in an integration-test crate"
)]
fn full() -> TptBaseRecord {
    TptBaseRecord {
        thumbnail_mode: ThumbnailMode::UploadNow,
        thumbnails: vec![UploadRef::new(HASH).expect("a hex digest")],
        video_preview: Some(UploadRef::new(HASH).expect("a hex digest")),
        additional_licence_minor_units: Some(405),
        bundle_discount_minor_units: Some(300),
        tax_code: Some(TaxCode::DigitalBooks),
        categories: CategoryGroup {
            grades: vec![],
            subject_areas: slugs(&["math", "science"]),
            tags: slugs(&["centers"]),
            formats: slugs(&["easel"]),
            custom_categories: vec!["Autumn unit".to_owned()],
            // Stated true on purpose: `None` is both the domain default and
            // what a dropped write leaves in the column, so a fixture holding
            // either of the other two states would read back equal to a write
            // path that never carried it.
            appropriate_for_country: Some(true),
        },
        standards: vec![StandardAlignment {
            framework: StandardsFramework::CommonCore,
            code: "CCSS.MATH.CONTENT.3.NF.A.2".to_owned(),
            tpt_node_id: Some(918_273),
        }],
        details: DetailGroup {
            teaching_duration: Some(TeachingDuration::from_wire_id(6).expect("1 Hour is a member")),
            pages_or_slides: Some(12),
            // Wire id 4, whose menu position is 3. The round trip is what
            // proves the two never get crossed on the way through a column.
            answer_key: Some(AnswerKey::IncludedWithRubric),
        },
        copyright: Some(CopyrightDeclaration::UsedCopyrightedMaterials),
        status: ListingStatus::Live,
    }
}

/// The record, over each state of the one field that has three.
///
/// The localisation flag is walked rather than fixed because 0050 is what made
/// its absence expressible, and the write that feeds the projection depends on
/// the absence surviving: the adapter's edit reads TPT's own flag back only
/// where the projection states nothing, so a `None` that read back as `false`
/// would repost a no the seller never gave.
#[sqlx::test(migrations = "./migrations")]
async fn every_field_the_form_collects_survives_the_round_trip(pool: PgPool) {
    provision(&pool).await;
    let repo = TptBaseRepo::new(pool);
    for stated in [Some(true), Some(false), None] {
        let mut written = full();
        written.categories.appropriate_for_country = stated;
        repo.upsert(ORG_A, PRODUCT_A, &written, AT)
            .await
            .expect("the sidecar writes");
        assert_eq!(
            repo.get(ORG_A, PRODUCT_A).await.expect("the sidecar reads"),
            Some(written),
            "every group the create form collects comes back as it went in, and a \
             localisation flag of {stated:?} keeps the three states apart: a seller who \
             answered nothing must not read back as a seller who answered no"
        );
    }
}

/// The mis-map this vocabulary has a history of, closed at the column.
#[sqlx::test(migrations = "./migrations")]
async fn the_answer_key_comes_back_as_the_id_that_went_in(pool: PgPool) {
    provision(&pool).await;
    let repo = TptBaseRepo::new(pool);
    for key in AnswerKey::ALL {
        let mut record = full();
        record.details.answer_key = Some(key);
        repo.upsert(ORG_A, PRODUCT_A, &record, AT)
            .await
            .expect("the sidecar writes");
        let read = repo
            .get(ORG_A, PRODUCT_A)
            .await
            .expect("the sidecar reads")
            .expect("a row was written");
        assert_eq!(
            read.details.answer_key,
            Some(key),
            "{key:?} has wire id {} and menu position {}, and the column stores the first",
            key.wire_id(),
            key.menu_index()
        );
    }
}

/// A create that carried no block writes no row, and the read says so rather
/// than answering with a product whose seller chose nothing.
#[sqlx::test(migrations = "./migrations")]
async fn a_product_with_no_sidecar_reads_as_absent_rather_than_empty(pool: PgPool) {
    provision(&pool).await;
    let repo = TptBaseRepo::new(pool);
    assert_eq!(
        repo.get(ORG_A, PRODUCT_A).await.expect("the read succeeds"),
        None,
        "no row is a product authored before this table or through a path that does not \
         carry these fields, which is a different thing from a row of nulls"
    );
}

/// The edit path. The row is replaced whole, so a control the seller cleared
/// is cleared rather than merged back in from what was there before.
#[sqlx::test(migrations = "./migrations")]
async fn a_second_write_replaces_the_row_rather_than_merging_into_it(pool: PgPool) {
    provision(&pool).await;
    let repo = TptBaseRepo::new(pool);
    repo.upsert(ORG_A, PRODUCT_A, &full(), AT)
        .await
        .expect("the first write lands");
    let mut cleared = full();
    cleared.tax_code = None;
    cleared.details.answer_key = None;
    cleared.categories.formats = vec![];
    cleared.standards = vec![];
    repo.upsert(ORG_A, PRODUCT_A, &cleared, AT)
        .await
        .expect("the second write lands");
    let read = repo
        .get(ORG_A, PRODUCT_A)
        .await
        .expect("the sidecar reads")
        .expect("a row exists");
    assert_eq!(
        (
            read.tax_code,
            read.details.answer_key,
            read.categories.formats.len(),
            read.standards.len()
        ),
        (None, None, 0, 0),
        "clearing a control is expressible, which a merge would make impossible"
    );
}

/// The tenant fence, stated over the relation rather than over the query: the
/// row-level-security policy is what refuses this, not a `WHERE` clause.
#[sqlx::test(migrations = "./migrations")]
async fn one_organisation_cannot_read_another_s_sidecar(pool: PgPool) {
    provision(&pool).await;
    let repo = TptBaseRepo::new(pool);
    repo.upsert(ORG_A, PRODUCT_A, &full(), AT)
        .await
        .expect("A's sidecar writes");
    assert_eq!(
        repo.get(ORG_B, PRODUCT_A).await.expect("the read succeeds"),
        None,
        "B pinned itself, so A's row matches no policy and is not there to be read"
    );
}

/// The one shape the form refuses and the database refuses too, so a client
/// that skipped the form cannot write it either.
#[sqlx::test(migrations = "./migrations")]
async fn thumbnails_under_a_mode_with_no_slots_are_refused_by_the_database(pool: PgPool) {
    provision(&pool).await;
    let repo = TptBaseRepo::new(pool);
    let mut deferred = full();
    deferred.thumbnail_mode = ThumbnailMode::UploadLater;
    assert!(
        repo.upsert(ORG_A, PRODUCT_A, &deferred, AT).await.is_err(),
        "the four slots are the conditional body of `upload thumbnails now`, and the CHECK \
         says so for anything that writes the row directly"
    );
}
