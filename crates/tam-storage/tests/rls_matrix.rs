//! The closed world: every table in the public schema must be classified
//! tenant or global, every tenant table must carry enabled AND forced
//! row-level security with a policy keyed on app.current_org, and a table
//! missing from both lists fails the test. Adding a table without a tenancy
//! decision is therefore a compile-against-reality error, not a review hope.

#![cfg(feature = "pg-tests")]

use std::collections::BTreeSet;

use sqlx::PgPool;

const TENANT_TABLES: [&str; 58] = [
    "auto_publish_rule",
    "auto_publish_run",
    "billing_subscription",
    "binding_candidate",
    "blob",
    "collection",
    "collection_member",
    "connection",
    "connection_audit",
    "connection_secret",
    "device",
    "device_marketplace_session",
    "duplicate_evidence",
    "duplicate_verdict",
    "election_item",
    "election_rule",
    "entitlement_grant",
    "field_audit",
    "field_mismatch",
    "grade_declaration",
    "grade_declaration_path",
    "import_batch",
    "import_batch_row",
    "import_run",
    "import_run_item",
    "job",
    "job_event",
    "job_item",
    "label",
    "listing_metric_snapshot",
    "mapping",
    "mapping_loss",
    "marketplace_request",
    "marketplace_sync_setting",
    "native_residue",
    "notification",
    "org_event_counter",
    "org_halt",
    "org_inventory_halt",
    "outbox_message",
    "product",
    "product_file",
    "product_file_observation",
    "product_fingerprint",
    "product_label",
    "product_term",
    "product_tpt_base",
    "projection_override",
    "rate_budget",
    "reconciliation_item",
    "resource_template",
    "schedule",
    "schedule_marketplace",
    "schedule_product",
    "schedule_run",
    "sync_request",
    "sync_request_resource",
    "write_attempt",
];

/// organisation is the tenant root itself (it has no org_id column); the
/// auth milestone revisits whether it needs its own read fence. app_user,
/// user_session and platform_operator are the authentication root: a session
/// row must be readable before any tenant pin exists, the stored token digest
/// is the capability's verifier rather than tenant data, and an operator
/// marking is a platform fact about a human rather than a row any tenant
/// owns. standards_node is the mirrored standards catalogue: public data
/// shared by every tenant, carrying no organisation column, per
/// docs/notes/design/standards-ingestion.md. The rest are genuinely global:
/// reference data, the canonical taxonomy, the fleet kill switch, and sqlx's
/// migration bookkeeping.
const GLOBAL_TABLES: [&str; 11] = [
    "_sqlx_migrations",
    "app_user",
    "canonical_term",
    "inventory_halt",
    "marketplace_inventory",
    "organisation",
    "platform_operator",
    "projection_edge",
    "projection_no_counterpart",
    "standards_node",
    "user_session",
];

/// How far the operator role reads past the tenant pin, table by table.
///
/// A closed world for the same reason the classification above is one: a
/// backoffice policy added without a line here would widen that role's reach
/// silently, and the reach of the one role that reads every tenant is the last
/// thing that should move without a reviewer seeing it.
///
/// Every entry is `true` -- the whole table -- except `job_event` and
/// `outbox_message`. Migration 0060 opens exactly the rows recording an
/// import-drain measurement, because the operator console draws that series
/// and has no business with the rest of a tenant's ledger; migration 0067 opens
/// exactly the dead letters, because those are the rows nothing else reads.
/// That asymmetry is the point of listing quals here rather than table names.
/// `import_run` is whole and its four child tables are absent, which is the
/// same asymmetry stated as a choice of table rather than of qual: whether a
/// seller's import finished is a support question, and the resources it
/// described, the sketches of their files and the duplicate questions they
/// were asked are not.
const BACKOFFICE_READABLE: [(&str, &str); 14] = [
    ("billing_subscription", "true"),
    ("connection", "true"),
    ("entitlement_grant", "true"),
    ("import_run", "true"),
    ("job", "true"),
    ("job_event", "(kind = 'ImportDrainMeasured'::text)"),
    ("job_item", "true"),
    ("mapping", "true"),
    ("marketplace_request", "true"),
    ("org_halt", "true"),
    ("org_inventory_halt", "true"),
    ("outbox_message", "(state = 'dead'::text)"),
    ("product", "true"),
    ("write_attempt", "true"),
];

/// The operator role's reach, asserted as a set rather than a sample.
#[sqlx::test(migrations = "./migrations")]
async fn the_backoffice_role_reads_exactly_what_is_declared(pool: PgPool) {
    let observed: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT tablename, qual FROM pg_policies \
         WHERE schemaname = 'public' AND 'tam_backoffice' = ANY(roles) \
         ORDER BY tablename",
    )
    .fetch_all(&pool)
    .await
    .expect("the policy query runs");

    let observed: Vec<(String, String)> = observed
        .into_iter()
        .map(|(table, qual)| (table, qual.unwrap_or_default()))
        .collect();
    let declared: Vec<(String, String)> = BACKOFFICE_READABLE
        .iter()
        .map(|(table, qual)| ((*table).to_owned(), (*qual).to_owned()))
        .collect();
    assert_eq!(
        observed, declared,
        "the operator role's cross-tenant reach changed; a policy added, \
         removed or widened here needs a line in BACKOFFICE_READABLE"
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn every_table_is_classified_and_every_tenant_table_is_fenced(pool: PgPool) {
    let tables: Vec<(String, bool, bool)> = sqlx::query_as(
        "SELECT c.relname, c.relrowsecurity, c.relforcerowsecurity \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'public' AND c.relkind = 'r'",
    )
    .fetch_all(&pool)
    .await
    .expect("the catalogue query runs");

    let observed: BTreeSet<&str> = tables.iter().map(|(name, _, _)| name.as_str()).collect();
    let classified: BTreeSet<&str> = TENANT_TABLES
        .iter()
        .chain(GLOBAL_TABLES.iter())
        .copied()
        .collect();
    assert_eq!(
        observed, classified,
        "every public table must appear in exactly one classification list; \
         a new table needs a tenancy decision here"
    );

    let policies: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT tablename, qual FROM pg_policies WHERE schemaname = 'public'")
            .fetch_all(&pool)
            .await
            .expect("the policy query runs");

    for tenant in TENANT_TABLES {
        let (_, enabled, forced) = tables
            .iter()
            .find(|(name, _, _)| name == tenant)
            .expect("classified tenant tables exist in the catalogue");
        assert!(enabled, "{tenant} must have row-level security enabled");
        assert!(
            forced,
            "{tenant} must FORCE row-level security onto its owner"
        );
        let keyed = policies.iter().any(|(table, qual)| {
            table == tenant
                && qual
                    .as_deref()
                    .is_some_and(|q| q.contains("app.current_org"))
        });
        assert!(
            keyed,
            "{tenant} must carry a policy keyed on app.current_org"
        );
    }
}

/// The override layer's tenancy, proven by driving it rather than by reading
/// the catalogue: the matrix above shows the policy exists, and this shows it
/// holds. One seller's mapping decision is invisible to every other seller,
/// which is the whole reason an override is tenant data while the edge
/// relation it overrides is global.
#[sqlx::test(migrations = "./migrations")]
async fn one_orgs_override_is_invisible_to_another(pool: PgPool) {
    use tam_domain::equivalence::{NewProjectionOverride, OverrideKind, ProjectionOverride};
    use tam_domain::{CanonicalTerm, Decider, TermKind, VocabularyId, VocabularyPath};
    use tam_storage::overrides::OverrideRepo;
    use tam_storage::TaxonomyRepo;
    use tam_types::{CanonicalTermId, InventoryId, OrgId, Timestamp, Uuid};

    let org_a = OrgId(Uuid([0xa1; 16]));
    let org_b = OrgId(Uuid([0xb2; 16]));
    for (org, name) in [(org_a, "org-a"), (org_b, "org-b")] {
        sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, $2, now())")
            .bind(uuid::Uuid::from_bytes(org.0 .0))
            .bind(name)
            .execute(&pool)
            .await
            .expect("the organisation inserts");
    }

    let term = CanonicalTermId(Uuid([0xc3; 16]));
    TaxonomyRepo::new(pool.clone())
        .seed(
            &[CanonicalTerm {
                id: term,
                kind: TermKind::Subject,
                parent: None,
                label: "Mathematics".to_owned(),
            }],
            &[],
        )
        .await
        .expect("the canonical term seeds");

    let repo = OverrideRepo::new(pool.clone());
    let entry = ProjectionOverride::new(NewProjectionOverride {
        org: org_a,
        inventory: InventoryId::Tes,
        axis: TermKind::Subject,
        from: term,
        to: VocabularyPath {
            vocabulary: VocabularyId(InventoryId::Tes, TermKind::Subject),
            segments: vec!["Primary science".to_owned()],
            native_id: Some("1000928".to_owned()),
        },
        kind: OverrideKind::Exact,
        decided_by: Decider::Imported {
            source: "test".to_owned(),
        },
        decided_at: Timestamp(0),
    })
    .expect("a subject override is constructible");
    repo.upsert(&entry).await.expect("org A records it");

    let a = repo.for_org(org_a).await.expect("org A reads its own");
    assert_eq!(a, vec![entry.clone()], "org A sees the decision it made");

    let b = repo.for_org(org_b).await.expect("org B reads its own");
    assert!(
        b.is_empty(),
        "org B must not see org A's decision; the forced policy is what stops it"
    );

    assert!(
        !repo
            .remove(org_b, InventoryId::Tes, TermKind::Subject, term)
            .await
            .expect("org B may attempt a withdrawal"),
        "org B must not be able to withdraw org A's decision either"
    );
    assert_eq!(
        repo.for_org(org_a).await.expect("org A reads again"),
        vec![entry],
        "and org A's decision survives the attempt"
    );
}
