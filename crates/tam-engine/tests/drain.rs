//! The drain loop end to end against the live database: a refusing relay
//! backs the message off, the elapsed backoff redelivers, and the report
//! counts what actually happened.

#![cfg(feature = "pg-tests")]

use std::sync::atomic::{AtomicU32, Ordering};

use tam_engine::outbox::{backoff_ms, drain, Deliverer, DeliveryError};
use tam_storage::{NewOutboxMessage, OutboxMessage, OutboxRepo};
use tam_types::{OrgId, Timestamp, Uuid};

const T0: Timestamp = Timestamp(1_756_000_000_000);
const ORG: OrgId = OrgId(Uuid([0xAA; 16]));

/// Refuses the first `refusals` deliveries, then accepts.
struct FlakyRelay {
    refusals: u32,
    seen: AtomicU32,
}

impl Deliverer for FlakyRelay {
    fn deliver(
        &self,
        _message: &OutboxMessage,
    ) -> impl core::future::Future<Output = Result<(), DeliveryError>> + Send {
        let attempt = self.seen.fetch_add(1, Ordering::SeqCst);
        core::future::ready(if attempt < self.refusals {
            Err(DeliveryError::Retryable("relay refused".to_owned()))
        } else {
            Ok(())
        })
    }
}

#[expect(
    clippy::expect_used,
    reason = "allow-expect-in-tests reaches #[test] functions, not a free helper in an integration-test crate; a broken fixture should panic"
)]
async fn engine_pool(app: &sqlx::PgPool) -> sqlx::PgPool {
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(app)
        .await
        .expect("the database name is readable");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&format!(
            "postgres://tam_engine:tam_engine_dev@127.0.0.1:5433/{database}"
        ))
        .await
        .expect("the engine role connects")
}

#[sqlx::test(migrations = "../tam-storage/migrations")]
async fn a_refused_message_backs_off_then_delivers(app: sqlx::PgPool) {
    sqlx::query("INSERT INTO organisation (id, name, created_at) VALUES ($1, 'org-a', now())")
        .bind(uuid::Uuid::from_bytes(ORG.0 .0))
        .execute(&app)
        .await
        .expect("the fixture org inserts");
    let engine = engine_pool(&app).await;
    let outbox = OutboxRepo::new(engine.clone());
    let mut tx = engine.begin().await.expect("transaction begins");
    OutboxRepo::append(
        &mut tx,
        &NewOutboxMessage {
            org: ORG,
            id: Uuid([0x61; 16]),
            topic: "email.job_settled".to_owned(),
            dedupe_key: "job-1".to_owned(),
            payload: serde_json::json!({"job": "1"}),
            at: T0,
        },
    )
    .await
    .expect("the append runs");
    tx.commit().await.expect("the append commits");

    let relay = FlakyRelay {
        refusals: 1,
        seen: AtomicU32::new(0),
    };
    let first = drain(&outbox, &relay, T0, 10)
        .await
        .expect("the drain runs");
    assert_eq!(
        (first.delivered, first.retried, first.dead),
        (0, 1, 0),
        "the refused message backs off"
    );
    let too_soon = drain(&outbox, &relay, Timestamp(T0.0 + 1), 10)
        .await
        .expect("the drain runs");
    assert_eq!(
        (too_soon.delivered, too_soon.retried, too_soon.dead),
        (0, 0, 0),
        "nothing is due inside the backoff"
    );
    let after = drain(&outbox, &relay, Timestamp(T0.0 + backoff_ms(0) + 1), 10)
        .await
        .expect("the drain runs");
    assert_eq!(
        (after.delivered, after.retried, after.dead),
        (1, 0, 0),
        "the elapsed backoff redelivers and the relay accepts"
    );
}
