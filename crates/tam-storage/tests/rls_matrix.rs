//! The closed world: every table in the public schema must be classified
//! tenant or global, every tenant table must carry enabled AND forced
//! row-level security with a policy keyed on app.current_org, and a table
//! missing from both lists fails the test. Adding a table without a tenancy
//! decision is therefore a compile-against-reality error, not a review hope.

#![cfg(feature = "pg-tests")]

use std::collections::BTreeSet;

use sqlx::PgPool;

const TENANT_TABLES: [&str; 30] = [
    "binding_candidate",
    "blob",
    "connection",
    "connection_audit",
    "connection_secret",
    "election_item",
    "election_rule",
    "field_audit",
    "field_mismatch",
    "grade_declaration",
    "grade_declaration_path",
    "job",
    "job_event",
    "job_item",
    "listing_metric_snapshot",
    "mapping",
    "mapping_loss",
    "native_residue",
    "org_event_counter",
    "org_halt",
    "org_inventory_halt",
    "outbox_message",
    "product",
    "product_file",
    "product_term",
    "rate_budget",
    "reconciliation_item",
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
/// owns. The rest are genuinely global: reference data, the canonical
/// taxonomy, the fleet kill switch, and sqlx's migration bookkeeping.
const GLOBAL_TABLES: [&str; 10] = [
    "_sqlx_migrations",
    "app_user",
    "canonical_term",
    "inventory_halt",
    "marketplace_inventory",
    "organisation",
    "platform_operator",
    "projection_edge",
    "projection_no_counterpart",
    "user_session",
];

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
