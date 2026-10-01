//! The refund mail, sent: claim a queued refund, resolve the organisation's
//! people's addresses, compose `refund_issued`, send, record.
//!
//! The refund rows of migration 0102 are the outbox (see
//! `tam_api::payments`): the Payments page queues a mail by stamping
//! `mail_requested_at`, and the outcome lands back on the same row — sent
//! with its instant, which is what turns the page's button into "Sent on …",
//! or retried later, or failed with the reason. The addresses are held for
//! the length of one send and stored nowhere, as the completion mail's are
//! (`notify`).

use core::time::Duration;

use tam_api::payments::money;
use tam_storage::{ClaimedRefundMail, PaymentRepo, RefundMailOutcome, REFUND_MAIL_ATTEMPTS};
use tam_types::Timestamp;
use tokio_util::sync::CancellationToken;

use crate::notify::{compose_refund, AddressResolver, Relay, RelayError, ResolveError};

/// How long the drainer waits when nothing is due.
const IDLE: Duration = Duration::from_secs(5);
/// The least time between two claimed refunds.
const SEND_INTERVAL: Duration = Duration::from_millis(200);
/// How long a claim holds a row: long past one send's timeouts.
const LEASE_MS: i64 = 120_000;
/// The first retry's wait; each later one doubles it.
const RETRY_BASE_MS: i64 = 60_000;

fn retry_or_fail(claimed: &ClaimedRefundMail, error: String, now: Timestamp) -> RefundMailOutcome {
    if claimed.attempts >= REFUND_MAIL_ATTEMPTS {
        return RefundMailOutcome::Failed { error };
    }
    let doublings = u32::try_from(claimed.attempts.saturating_sub(1)).unwrap_or(0);
    let wait = RETRY_BASE_MS.saturating_mul(2_i64.saturating_pow(doublings));
    RefundMailOutcome::Retry {
        error,
        at: Timestamp(now.0.saturating_add(wait)),
    }
}

/// One claimed refund, mailed to every person in the organisation the
/// identity service vouches for an address of, and what became of it.
///
/// Every address is resolved before anything is sent, so an unreachable
/// identity service retries a refund that mailed nobody. Nobody reachable is
/// a failure the page shows rather than a retry: waiting will not give
/// anybody a verified address. A relay fault retries the whole mail, which
/// can re-mail someone an earlier pass reached — the accepted cost the
/// completion mail also pays.
pub(crate) async fn send_one<R: AddressResolver, S: Relay>(
    claimed: &ClaimedRefundMail,
    resolver: &R,
    relay: &S,
    console_url: &str,
    now: Timestamp,
) -> RefundMailOutcome {
    let mut recipients = Vec::with_capacity(claimed.subjects.len());
    for subject in &claimed.subjects {
        match resolver.address(*subject).await {
            Ok(address) if address.verified => recipients.push(address),
            Ok(_) | Err(ResolveError::NoAddress(_)) => {}
            Err(ResolveError::Unreachable(why)) => return retry_or_fail(claimed, why, now),
        }
    }
    if recipients.is_empty() {
        return RefundMailOutcome::Failed {
            error: "Nobody in this organisation has a verified email address.".to_owned(),
        };
    }
    let amount = money(claimed.amount_cents, &claimed.currency);
    for address in &recipients {
        let mail = compose_refund(
            &amount,
            &claimed.org_name,
            address.name.as_deref(),
            console_url,
        );
        match relay.send(&address.email, &mail).await {
            Ok(()) => {}
            Err(RelayError::Retryable(why)) => return retry_or_fail(claimed, why, now),
            Err(RelayError::Permanent(why)) => return RefundMailOutcome::Failed { error: why },
        }
    }
    RefundMailOutcome::Sent
}

/// The drainer's loop: claim, send, record, and space the sends.
#[expect(
    clippy::disallowed_methods,
    reason = "the refund mail loop is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
pub(crate) fn spawn<R, S>(
    repo: PaymentRepo,
    resolver: R,
    relay: S,
    console_url: String,
    cancel: CancellationToken,
) where
    R: AddressResolver + 'static,
    S: Relay + 'static,
{
    tokio::spawn(async move {
        loop {
            let now = crate::wall_now();
            let claimed = match repo
                .claim_mail(now, Timestamp(now.0.saturating_add(LEASE_MS)))
                .await
            {
                Ok(claimed) => claimed,
                Err(error) => {
                    eprintln!("tam-server: could not claim a refund mail: {error}");
                    None
                }
            };
            let pause = if let Some(claimed) = claimed {
                let outcome = send_one(&claimed, &resolver, &relay, &console_url, now).await;
                if let RefundMailOutcome::Failed { error } = &outcome {
                    eprintln!(
                        "tam-server: the mail for refund {} was not sent: {error}",
                        claimed.refund.to_hyphenated()
                    );
                }
                if let Err(error) = repo
                    .settle_mail(claimed.refund, &outcome, crate::wall_now())
                    .await
                {
                    eprintln!("tam-server: could not record a refund mail's outcome: {error}");
                }
                SEND_INTERVAL
            } else {
                IDLE
            };
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(pause) => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use tokio::sync::Mutex;

    use tam_storage::{ClaimedRefundMail, RefundMailOutcome, REFUND_MAIL_ATTEMPTS};
    use tam_types::{Timestamp, Uuid};

    use super::send_one;
    use crate::notify::{Address, AddressResolver, Mail, Relay, RelayError, ResolveError};

    const NOW: Timestamp = Timestamp(1_790_640_000_000);
    const CONSOLE: &str = "https://app.teachouse.test";

    struct Resolver(Result<Address, ResolveError>);

    impl AddressResolver for Resolver {
        async fn address(&self, _subject: Uuid) -> Result<Address, ResolveError> {
            self.0.clone()
        }
    }

    struct Sent {
        answer: Result<(), RelayError>,
        mails: Mutex<Vec<(String, Mail)>>,
    }

    impl Relay for Sent {
        async fn send(&self, to: &str, mail: &Mail) -> Result<(), RelayError> {
            self.mails.lock().await.push((to.to_owned(), mail.clone()));
            self.answer.clone()
        }
    }

    fn relay(answer: Result<(), RelayError>) -> Sent {
        Sent {
            answer,
            mails: Mutex::new(Vec::new()),
        }
    }

    fn claimed(attempts: i32) -> ClaimedRefundMail {
        ClaimedRefundMail {
            refund: Uuid([0x11; 16]),
            org_name: "Kauri Room".to_owned(),
            amount_cents: 1_200,
            currency: "usd".to_owned(),
            attempts,
            subjects: vec![Uuid([0x22; 16])],
        }
    }

    fn vouched() -> Address {
        Address {
            email: "aroha@example.test".to_owned(),
            verified: true,
            name: Some("Aroha".to_owned()),
        }
    }

    #[tokio::test]
    async fn the_customer_is_told_the_amount_and_the_wait() {
        let relay = relay(Ok(()));
        let outcome = send_one(&claimed(1), &Resolver(Ok(vouched())), &relay, CONSOLE, NOW).await;
        assert_eq!(outcome, RefundMailOutcome::Sent);
        let mails = relay.mails.lock().await.clone();
        assert_eq!(mails.len(), 1);
        let (to, mail) = mails.first().expect("one mail went");
        assert_eq!(to, "aroha@example.test");
        assert_eq!(mail.subject, "We've refunded $12.00");
        assert!(mail.html.contains(
            "We've refunded $12.00 to your card. \
             It can take 5–10 business days to show."
        ));
        assert!(mail.html.contains("Kia ora Aroha,"));
        assert_eq!(mail.href, "https://app.teachouse.test/settings/billing");
    }

    #[tokio::test]
    async fn an_unverified_address_is_not_mailed_and_the_page_is_told_why() {
        let relay = relay(Ok(()));
        let unverified = Ok(Address {
            email: "maybe@example.test".to_owned(),
            verified: false,
            name: None,
        });
        let outcome = send_one(&claimed(1), &Resolver(unverified), &relay, CONSOLE, NOW).await;
        assert!(matches!(outcome, RefundMailOutcome::Failed { .. }));
        assert!(relay.mails.lock().await.is_empty());
    }

    #[tokio::test]
    async fn an_unreachable_identity_service_retries_until_the_attempts_run_out() {
        let relay = relay(Ok(()));
        let down = Resolver(Err(ResolveError::Unreachable("down".to_owned())));
        assert!(matches!(
            send_one(&claimed(1), &down, &relay, CONSOLE, NOW).await,
            RefundMailOutcome::Retry { at, .. } if at == Timestamp(NOW.0 + 60_000)
        ));
        assert!(matches!(
            send_one(&claimed(REFUND_MAIL_ATTEMPTS), &down, &relay, CONSOLE, NOW).await,
            RefundMailOutcome::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn a_relay_refusal_fails_and_a_relay_fault_retries() {
        let refused = relay(Err(RelayError::Permanent("bad address".to_owned())));
        assert_eq!(
            send_one(
                &claimed(1),
                &Resolver(Ok(vouched())),
                &refused,
                CONSOLE,
                NOW
            )
            .await,
            RefundMailOutcome::Failed {
                error: "bad address".to_owned()
            }
        );
        let busy = relay(Err(RelayError::Retryable("429".to_owned())));
        assert!(matches!(
            send_one(&claimed(2), &Resolver(Ok(vouched())), &busy, CONSOLE, NOW).await,
            RefundMailOutcome::Retry { at, .. } if at == Timestamp(NOW.0 + 120_000)
        ));
    }
}
