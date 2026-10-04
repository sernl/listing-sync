//! The sign-up agreement record (migration 0103): three rows per agreement,
//! read back per subject and as the operator's newest-per-subject summary,
//! and never rewritten or removed.

#![cfg(feature = "pg-tests")]

use std::net::{IpAddr, Ipv4Addr};

use sqlx::PgPool;
use tam_storage::{AccountConsentKind, AccountConsentRepo, NewAccountConsent};
use tam_types::{Timestamp, Uuid};

const ALICE: Uuid = Uuid([0xa1; 16]);
const BOB: Uuid = Uuid([0xb0; 16]);
const DAY: i64 = 24 * 60 * 60 * 1000;

fn agreement(subject: Uuid, version: &str, day: i64) -> NewAccountConsent<'_> {
    NewAccountConsent {
        subject,
        email: Some("alice@example.test"),
        document_version: version,
        accepted_at: Timestamp(1_790_000_000_000 + day * DAY),
        ip_address: Some(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7))),
        user_agent: Some("Mozilla/5.0 (test)"),
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn an_agreement_is_three_rows_read_back_newest_first(pool: PgPool) {
    let repo = AccountConsentRepo::new(pool.clone());
    repo.record(&agreement(ALICE, "2026-10-02", 0))
        .await
        .expect("the first agreement writes");
    repo.record(&agreement(ALICE, "2026-10-04", 2))
        .await
        .expect("the re-consent writes");

    let rows = repo.for_subject(ALICE).await.expect("the rows read");
    assert_eq!(rows.len(), 6, "three statements per agreement");
    assert!(rows
        .iter()
        .take(3)
        .all(|row| row.document_version == "2026-10-04"));
    let kinds: Vec<AccountConsentKind> = rows.iter().take(3).map(|row| row.kind).collect();
    assert!(kinds.contains(&AccountConsentKind::TermsPrivacy));
    assert!(kinds.contains(&AccountConsentKind::IpOwnership));
    assert!(kinds.contains(&AccountConsentKind::Age18));
    assert_eq!(rows[0].ip_address.as_deref(), Some("203.0.113.7"));
    assert_eq!(rows[0].user_agent.as_deref(), Some("Mozilla/5.0 (test)"));
    assert_eq!(rows[0].email.as_deref(), Some("alice@example.test"));

    assert!(repo.accepted(ALICE, "2026-10-04").await.expect("reads"));
    assert!(repo.accepted(ALICE, "2026-10-02").await.expect("reads"));
    assert!(!repo.accepted(BOB, "2026-10-04").await.expect("reads"));
    assert_eq!(
        repo.latest_version(ALICE).await.expect("reads").as_deref(),
        Some("2026-10-04")
    );
    assert_eq!(repo.latest_version(BOB).await.expect("reads"), None);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_summary_carries_each_subjects_newest_terms_acceptance(pool: PgPool) {
    let repo = AccountConsentRepo::new(pool.clone());
    repo.record(&agreement(ALICE, "2026-10-02", 0))
        .await
        .expect("writes");
    repo.record(&agreement(ALICE, "2026-10-04", 1))
        .await
        .expect("writes");
    repo.record(&agreement(BOB, "2026-10-02", 0))
        .await
        .expect("writes");

    let mut summaries = repo.summaries().await.expect("the summary reads");
    summaries.sort_by_key(|summary| summary.subject.0);
    assert_eq!(summaries.len(), 2, "one line per subject");
    let alice = summaries
        .iter()
        .find(|summary| summary.subject == ALICE)
        .expect("alice is summarised");
    assert_eq!(alice.document_version, "2026-10-04");
    assert_eq!(alice.accepted_at, Timestamp(1_790_000_000_000 + DAY));
}

#[sqlx::test(migrations = "./migrations")]
async fn an_agreement_is_never_rewritten_or_removed(pool: PgPool) {
    AccountConsentRepo::new(pool.clone())
        .record(&agreement(ALICE, "2026-10-04", 0))
        .await
        .expect("writes");
    for statement in [
        "UPDATE account_consent SET document_version = '2020-01-01'",
        "DELETE FROM account_consent",
    ] {
        let refused = sqlx::query(statement).execute(&pool).await;
        assert!(
            refused.is_err(),
            "the append-only trigger refuses even the table's owner: {statement}"
        );
    }
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM account_consent")
        .fetch_one(&pool)
        .await
        .expect("counts");
    assert_eq!(rows, 3);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_version_that_is_not_a_date_is_refused(pool: PgPool) {
    let refused = AccountConsentRepo::new(pool)
        .record(&agreement(ALICE, "latest", 0))
        .await;
    assert!(
        refused.is_err(),
        "the version names the terms' effective date"
    );
}
