//! The Postgres side of the driver's two write capabilities.
//!
//! Each opens a transaction around a single write, which is what the
//! interpreter's move off the raw pool had to leave unchanged: the journal's
//! per-organisation sequence is allocated inside its transaction, and the
//! outbox row is deduped by the storage layer within its own.

use sqlx::PgPool;
use tam_storage::{append_event, EventScope, NewOutboxMessage, OutboxRepo, StorageError};
use tam_types::{JobEventPayload, Stamp};

use crate::driver::{JournalPort, NotifyPort};

/// The event journal against the engine's own pool.
pub struct PgJournal {
    pool: PgPool,
}

impl PgJournal {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl JournalPort for PgJournal {
    async fn record(
        &self,
        scope: &EventScope,
        payload: &JobEventPayload,
        stamp: Stamp,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        append_event(&mut tx, scope, payload, stamp).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// The seller-notification outbox against the engine's own pool.
pub struct PgOutbox {
    pool: PgPool,
}

impl PgOutbox {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl NotifyPort for PgOutbox {
    async fn append(&self, message: &NewOutboxMessage) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await?;
        OutboxRepo::append(&mut tx, message).await?;
        tx.commit().await?;
        Ok(())
    }
}
