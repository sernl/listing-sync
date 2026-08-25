//! The outbox drainer: claim what is due, deliver through the seam, back off
//! exponentially on refusal, and quarantine by attempt count alone — a
//! message that crashes a deliverer is dead-lettered like any other, which
//! is the whole of the poison-message handling.

use tam_storage::{OutboxMessage, OutboxRepo, StorageError};
use tam_types::Timestamp;

/// Configuration defaults from the design's operational table; each bounds
/// only this drainer, so they are ordinary constants beside their caller.
pub const OUTBOX_BACKOFF_BASE_MS: i64 = 30_000;
pub const OUTBOX_BACKOFF_MAX_MS: i64 = 3_600_000;
pub const OUTBOX_ATTEMPTS_MAX: i32 = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryError {
    /// The relay refused or was unreachable; the message backs off.
    Retryable(String),
    /// The message itself cannot be delivered; it dead-letters immediately.
    Poison(String),
}

/// One delivery destination. The real relays (email, push, billing) arrive
/// with their milestones; the seam is what M1d ships.
pub trait Deliverer: Send + Sync {
    fn deliver(
        &self,
        message: &OutboxMessage,
    ) -> impl core::future::Future<Output = Result<(), DeliveryError>> + Send;
}

/// Logs and succeeds: the deliverer the worker runs until a relay exists,
/// so drained messages are visible rather than silently accumulating.
pub struct LoggingDeliverer;

impl Deliverer for LoggingDeliverer {
    fn deliver(
        &self,
        message: &OutboxMessage,
    ) -> impl core::future::Future<Output = Result<(), DeliveryError>> + Send {
        eprintln!(
            "outbox deliver (logging only): topic={} org={:02x?}",
            message.topic, message.org.0 .0
        );
        core::future::ready(Ok(()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DrainReport {
    pub delivered: u32,
    pub retried: u32,
    pub dead: u32,
}

/// The exponential backoff, capped: attempts are the exponent, so the first
/// refusal waits the base and the curve stops at the ceiling.
#[must_use]
pub fn backoff_ms(attempts: i32) -> i64 {
    let exponent = u32::try_from(attempts).unwrap_or(0).min(24);
    OUTBOX_BACKOFF_BASE_MS
        .saturating_mul(1_i64 << exponent)
        .min(OUTBOX_BACKOFF_MAX_MS)
}

pub async fn drain(
    outbox: &OutboxRepo,
    deliverer: &impl Deliverer,
    now: Timestamp,
    batch: i64,
) -> Result<DrainReport, StorageError> {
    let mut report = DrainReport::default();
    for message in outbox.claim_due(now, batch).await? {
        let reference = message.reference();
        match deliverer.deliver(&message).await {
            Ok(()) => {
                if outbox.mark_delivered(&reference, now).await? {
                    report.delivered += 1;
                }
            }
            Err(DeliveryError::Retryable(error)) => {
                if message.attempts + 1 >= OUTBOX_ATTEMPTS_MAX {
                    if outbox.mark_dead(&reference, &error).await? {
                        report.dead += 1;
                    }
                } else {
                    let next = Timestamp(now.0 + backoff_ms(message.attempts));
                    if outbox.retry_later(&reference, next, &error).await? {
                        report.retried += 1;
                    }
                }
            }
            Err(DeliveryError::Poison(error)) => {
                if outbox.mark_dead(&reference, &error).await? {
                    report.dead += 1;
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::{backoff_ms, OUTBOX_BACKOFF_BASE_MS, OUTBOX_BACKOFF_MAX_MS};

    #[test]
    fn the_backoff_doubles_from_the_base_and_caps() {
        assert_eq!(
            backoff_ms(0),
            OUTBOX_BACKOFF_BASE_MS,
            "first refusal waits the base"
        );
        assert_eq!(backoff_ms(1), OUTBOX_BACKOFF_BASE_MS * 2, "then doubles");
        assert_eq!(
            backoff_ms(30),
            OUTBOX_BACKOFF_MAX_MS,
            "the curve stops at the ceiling and cannot overflow"
        );
    }
}
