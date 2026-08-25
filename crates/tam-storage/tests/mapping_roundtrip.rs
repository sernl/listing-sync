//! The mapping codec is proven by property: an arbitrary well-formed
//! `Mapping` inserted and re-read is equal, including `Mismatched`
//! verifications with their ordered child rows and ambiguous creates with
//! their candidate identifiers. Generation is deterministic and bounded;
//! shrinking is traded away because each case runs a live database round
//! trip, and the failing value prints whole in the assertion message.

#![cfg(feature = "pg-tests")]

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;
use sqlx::PgPool;
use tam_domain::{
    Binding, CanonicalProduct, DeclarationSource, FieldPolicies, FieldPolicy, GradeDeclaration,
    Mapping, PublishMode, SeverCause, Verification,
};
use tam_marketplace::{CorrelationMarker, RemoteLifecycle, RemoteListingId};
use tam_storage::{MappingRepo, ProductRepo};
use tam_types::{
    AttemptId, ContentHash, FieldKey, FieldMismatch, FileId, FileKind, FileRole, InventoryId,
    ListingCopy, MappingId, Marketplace, MismatchClass, Money, OrgId, PayloadSet, PriceIntent,
    PriceRule, ProductFile, ProductId, Rounding, ScanOutcome, Timestamp, Title, Uuid,
};

const ORG_A: OrgId = OrgId(Uuid([0xAA; 16]));
const PRODUCT_1: ProductId = ProductId(Uuid([0x01; 16]));

fn minimal_product() -> CanonicalProduct {
    CanonicalProduct {
        id: PRODUCT_1,
        org: ORG_A,
        title: Title("Fixture product".to_owned()),
        body: ListingCopy {
            body: "Fixture body.".to_owned(),
        },
        payload: PayloadSet::new(
            ProductFile {
                id: FileId(Uuid([0x21; 16])),
                role: FileRole::Payload,
                kind: FileKind::Pdf,
                hash: ContentHash([0x51; 32]),
                byte_len: 4,
                scan: ScanOutcome::Pending,
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
    }
}

async fn seed_fixture(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
        .execute(pool)
        .await?;
    Ok(())
}

fn arb_timestamp() -> impl Strategy<Value = Timestamp> {
    (0i64..4_000_000_000_000i64).prop_map(Timestamp)
}

fn arb_uuid() -> impl Strategy<Value = Uuid> {
    any::<[u8; 16]>().prop_map(Uuid)
}

/// Remote identifiers drawn from ONE marketplace, because a mapping holding
/// an identifier minted by a different marketplace is the invariant
/// `Mapping::check` refuses and `mapping_remote_id_marketplace` enforces;
/// the generator must respect what the domain forbids.
fn arb_remote_id_for(marketplace: Marketplace) -> BoxedStrategy<RemoteListingId> {
    match marketplace {
        Marketplace::Tes => "[a-z0-9-]{1,40}"
            .prop_map(|slug| RemoteListingId::Tes {
                url: format!("https://www.tes.com/teaching-resource/{slug}"),
            })
            .boxed(),
        Marketplace::Tpt => (0u64..9_000_000_000_000_000_000u64)
            .prop_map(|product_id| RemoteListingId::Tpt { product_id })
            .boxed(),
        Marketplace::Etsy => (0u64..9_000_000_000_000_000_000u64)
            .prop_map(|listing_id| RemoteListingId::Etsy { listing_id })
            .boxed(),
    }
}

fn arb_field() -> impl Strategy<Value = FieldKey> {
    prop_oneof![
        Just(FieldKey::Title),
        Just(FieldKey::Description),
        Just(FieldKey::Price),
        Just(FieldKey::Taxonomy),
        Just(FieldKey::Grades),
        Just(FieldKey::Files),
    ]
}

fn arb_class() -> impl Strategy<Value = MismatchClass> {
    prop_oneof![
        Just(MismatchClass::Normalised),
        (0usize..100_000usize)
            .prop_map(|limit_observed| MismatchClass::Truncated { limit_observed }),
        Just(MismatchClass::Missing),
        arb_field().prop_map(|observed_in| MismatchClass::WrongField { observed_in }),
        Just(MismatchClass::Unexpected),
    ]
}

/// Distinct fields, because `field_mismatch` is keyed per field.
fn arb_mismatches() -> impl Strategy<Value = (FieldMismatch, Vec<FieldMismatch>)> {
    const FIELDS: [FieldKey; 6] = [
        FieldKey::Title,
        FieldKey::Description,
        FieldKey::Price,
        FieldKey::Taxonomy,
        FieldKey::Grades,
        FieldKey::Files,
    ];
    proptest::sample::subsequence(FIELDS.to_vec(), 1..=6)
        .prop_flat_map(|fields| {
            let classes = proptest::collection::vec(arb_class(), fields.len());
            (Just(fields), classes)
        })
        .prop_map(|(fields, classes)| {
            let mut mismatches = fields
                .into_iter()
                .zip(classes)
                .map(|(field, class)| FieldMismatch { field, class });
            let first = mismatches.next();
            let rest: Vec<_> = mismatches.collect();
            (
                first.unwrap_or(FieldMismatch {
                    field: FieldKey::Title,
                    class: MismatchClass::Normalised,
                }),
                rest,
            )
        })
}

fn arb_verification() -> impl Strategy<Value = Verification> {
    prop_oneof![
        arb_timestamp().prop_map(|since| Verification::Stale { since }),
        arb_timestamp().prop_map(|at| Verification::Clean { at }),
        (arb_timestamp(), arb_mismatches())
            .prop_map(|(at, (first, rest))| { Verification::Mismatched { at, first, rest } }),
    ]
}

fn arb_binding_for(marketplace: Marketplace) -> BoxedStrategy<Binding> {
    prop_oneof![
        Just(Binding::Unbound),
        (
            arb_uuid(),
            proptest::option::of("[ -~]{1,30}".prop_map(CorrelationMarker))
        )
            .prop_map(|(attempt, marker)| Binding::Creating {
                attempt: AttemptId(attempt),
                marker,
            }),
        (
            arb_remote_id_for(marketplace),
            arb_timestamp(),
            arb_verification()
        )
            .prop_map(|(id, first_seen, verified)| Binding::Bound {
                id,
                first_seen,
                verified,
            }),
        (
            arb_uuid(),
            proptest::collection::vec(arb_remote_id_for(marketplace), 0..3),
            arb_timestamp()
        )
            .prop_map(|(attempt, candidates, since)| Binding::AmbiguousCreate {
                attempt: AttemptId(attempt),
                candidates,
                since,
            }),
        (
            arb_remote_id_for(marketplace),
            arb_timestamp(),
            prop_oneof![
                Just(SeverCause::RemovedByMarketplace),
                Just(SeverCause::RemovedBySeller),
                Just(SeverCause::NotFoundOnVerify),
            ]
        )
            .prop_map(|(was, noticed, cause)| Binding::Severed {
                was,
                noticed,
                cause,
            }),
    ]
    .boxed()
}

fn arb_policy() -> impl Strategy<Value = FieldPolicy> {
    prop_oneof![
        Just(FieldPolicy::Managed),
        Just(FieldPolicy::Frozen),
        Just(FieldPolicy::Propose),
    ]
}

fn arb_policies() -> impl Strategy<Value = FieldPolicies> {
    (
        arb_policy(),
        arb_policy(),
        arb_policy(),
        arb_policy(),
        arb_policy(),
        arb_policy(),
    )
        .prop_map(
            |(title, description, price, taxonomy, grades, files)| FieldPolicies {
                title,
                description,
                price,
                taxonomy,
                grades,
                files,
            },
        )
}

fn arb_price_rule() -> impl Strategy<Value = PriceRule> {
    let currency = prop_oneof![
        Just(tam_types::Currency::Gbp),
        Just(tam_types::Currency::Usd)
    ];
    prop_oneof![
        (
            any::<i64>(),
            prop_oneof![Just(Rounding::Nearest), Just(Rounding::UpToCharm)]
        )
            .prop_map(|(rate_micros, rounding)| PriceRule::Converted {
                rate_micros,
                rounding,
            }),
        Just(PriceRule::Explicit(PriceIntent::Free)),
        (1i64..i64::MAX, currency).prop_filter_map(
            "a positive amount is a price",
            |(units, cur)| {
                Money::new(units, cur)
                    .ok()
                    .map(|money| PriceRule::Explicit(PriceIntent::Paid(money)))
            }
        ),
    ]
}

fn arb_lifecycle() -> impl Strategy<Value = RemoteLifecycle> {
    prop_oneof![
        Just(RemoteLifecycle::Absent),
        Just(RemoteLifecycle::Draft),
        arb_timestamp().prop_map(|at| RemoteLifecycle::Submitted { at }),
        arb_timestamp().prop_map(|since| RemoteLifecycle::InReview { since }),
        arb_timestamp().prop_map(|since| RemoteLifecycle::Live { since }),
        (
            arb_timestamp(),
            proptest::option::of("[ -~]{1,40}".prop_map(String::from))
        )
            .prop_map(|(at, reason)| RemoteLifecycle::Rejected { at, reason }),
        arb_timestamp().prop_map(|at| RemoteLifecycle::Withdrawn { at }),
    ]
}

fn arb_inventory() -> impl Strategy<Value = InventoryId> {
    prop_oneof![
        Just(InventoryId::TesGb),
        Just(InventoryId::TesUs),
        Just(InventoryId::Etsy),
        Just(InventoryId::Tpt),
    ]
}

fn arb_mapping() -> impl Strategy<Value = Mapping> {
    arb_inventory().prop_flat_map(|inventory| {
        (
            arb_uuid(),
            arb_binding_for(inventory.marketplace()),
            arb_policies(),
            arb_price_rule(),
            prop_oneof![
                Just(PublishMode::DryRun),
                Just(PublishMode::Propose),
                Just(PublishMode::Publish),
            ],
            arb_lifecycle(),
        )
            .prop_map(
                move |(id, binding, policies, price_rule, publish, lifecycle)| Mapping {
                    id: MappingId(id),
                    org: ORG_A,
                    product: PRODUCT_1,
                    inventory,
                    binding,
                    policies,
                    price_rule,
                    publish,
                    lifecycle,
                },
            )
    })
}

#[sqlx::test(migrations = "./migrations")]
async fn an_arbitrary_mapping_round_trips(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), Timestamp(1))
        .await
        .expect("the fixture product inserts");
    let repo = MappingRepo::new(pool.clone());

    let mut runner = TestRunner::deterministic();
    let strategy = arb_mapping();
    for case in 0..24 {
        let mapping = strategy
            .new_tree(&mut runner)
            .expect("the strategy generates")
            .current();
        assert!(
            mapping.check().is_ok(),
            "generator sanity: case {case} must satisfy the domain invariant"
        );
        repo.insert(ORG_A, &mapping, 3, Timestamp(1_000 + case))
            .await
            .expect("an arbitrary well-formed mapping encodes and inserts");
        let record = repo
            .get(ORG_A, mapping.id)
            .await
            .expect("the mapping reads back")
            .expect("the mapping exists");
        assert_eq!(
            record.mapping, mapping,
            "case {case}: the mapping must survive the round trip unchanged"
        );
        assert_eq!(
            record.normaliser_version, 3,
            "case {case}: the record metadata survives"
        );

        let mut tx = pool.begin().await.expect("cleanup transaction begins");
        sqlx::query("SELECT set_config('app.current_org', $1, true)")
            .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
            .execute(&mut *tx)
            .await
            .expect("tenant pin applies");
        sqlx::query("DELETE FROM mapping WHERE org_id = $1 AND id = $2")
            .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
            .bind(uuid::Uuid::from_bytes(mapping.id.0 .0))
            .execute(&mut *tx)
            .await
            .expect("the case row deletes");
        tx.commit().await.expect("cleanup commits");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_bound_row_without_a_remote_id_is_refused(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), Timestamp(1))
        .await
        .expect("the fixture product inserts");

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    let refused = sqlx::query(
        "INSERT INTO mapping \
         (org_id, id, product_id, inventory, marketplace, binding_state, \
          first_seen_at, verify_state, verify_stale_since, normaliser_version, \
          policy_title, policy_description, policy_price, policy_taxonomy, \
          policy_grades, policy_files, price_rule_kind, price_explicit_kind, \
          publish_mode, lifecycle_state, created_at, updated_at) \
         VALUES ($1, $2, $3, 'tes_gb', 'tes', 'bound', now(), 'stale', now(), 0, \
                 'managed', 'managed', 'managed', 'managed', 'managed', 'managed', \
                 'explicit', 'free', 'publish', 'absent', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0x77; 16]))
    .bind(uuid::Uuid::from_bytes(PRODUCT_1.0 .0))
    .execute(&mut *tx)
    .await;
    assert!(
        refused.is_err(),
        "mapping_binding_total must refuse a bound row with no remote identifier"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn a_mismatch_claim_without_a_named_field_cannot_commit(pool: PgPool) {
    seed_fixture(&pool).await.expect("fixture rows insert");
    let products = ProductRepo::new(pool.clone());
    products
        .insert(ORG_A, &minimal_product(), Timestamp(1))
        .await
        .expect("the fixture product inserts");

    let mut tx = pool.begin().await.expect("transaction begins");
    sqlx::query("SELECT set_config('app.current_org', $1, true)")
        .bind(uuid::Uuid::from_bytes(ORG_A.0 .0).to_string())
        .execute(&mut *tx)
        .await
        .expect("tenant pin applies");
    sqlx::query(
        "INSERT INTO mapping \
         (org_id, id, product_id, inventory, marketplace, binding_state, \
          remote_id_kind, remote_url, first_seen_at, verify_state, verified_at, \
          normaliser_version, policy_title, policy_description, policy_price, \
          policy_taxonomy, policy_grades, policy_files, price_rule_kind, \
          price_explicit_kind, publish_mode, lifecycle_state, created_at, updated_at) \
         VALUES ($1, $2, $3, 'tes_gb', 'tes', 'bound', 'tes', 'https://example', \
                 now(), 'mismatched', now(), 0, 'managed', 'managed', 'managed', \
                 'managed', 'managed', 'managed', 'explicit', 'free', 'publish', \
                 'absent', now(), now())",
    )
    .bind(uuid::Uuid::from_bytes(ORG_A.0 .0))
    .bind(uuid::Uuid::from_bytes([0x78; 16]))
    .bind(uuid::Uuid::from_bytes(PRODUCT_1.0 .0))
    .execute(&mut *tx)
    .await
    .expect("the row inserts; the deferred trigger has not run yet");
    let refused = tx.commit().await;
    assert!(
        refused.is_err(),
        "the deferred trigger must refuse a mismatch claim with no field_mismatch row"
    );
}
