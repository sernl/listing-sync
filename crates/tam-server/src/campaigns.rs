//! The operators' campaigns, sent: claim a due recipient row, resolve the
//! address, render, send, record.
//!
//! The recipient rows of migration 0097 are the outbox (see
//! `tam_api::mail_campaigns`). One row is claimed at a time and sends are
//! spaced at least [`SEND_INTERVAL`] apart, so this process never asks the
//! relay for more than ten a second. Each outcome lands on the row it came
//! from: sent with the relay's id, failed with its answer, skipped with the
//! reason, or put back with a later due time.
//!
//! The address is held for the length of one send and stored nowhere, as the
//! completion mail's is (`notify`).

use core::time::Duration;

use tam_api::mail_campaigns::{long_date, render, Letter, Personal, UNSUBSCRIBE_PATH};
use tam_storage::{ClaimedMail, MailCampaignRepo, MailOutcome};
use tam_types::Timestamp;
use tokio_util::sync::CancellationToken;

use crate::notify::{Address, AddressResolver, RelayError, ResendRelay, ResolveError};

/// The least time between two sends: ten a second at most.
pub(crate) const SEND_INTERVAL: Duration = Duration::from_millis(100);
/// How long the drainer waits when nothing is due.
const IDLE: Duration = Duration::from_secs(5);
/// How long a claim holds a row: long past one send's timeouts.
const LEASE_MS: i64 = 120_000;
/// A retryable fault is tried this many times in all before the row fails.
pub(crate) const MAX_ATTEMPTS: i32 = 5;
/// The first retry's wait; each later one doubles it.
const RETRY_BASE_MS: i64 = 60_000;

/// One composed campaign mail, as the relay posts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Outgoing {
    pub(crate) subject: String,
    pub(crate) html: String,
    pub(crate) text: String,
    pub(crate) unsubscribe_url: String,
}

/// The relay as the campaign path needs it: a send that answers an id.
pub(crate) trait CampaignRelay: Send + Sync {
    fn send(
        &self,
        to: &str,
        mail: &Outgoing,
    ) -> impl core::future::Future<Output = Result<String, RelayError>> + Send;
}

impl CampaignRelay for ResendRelay {
    async fn send(&self, to: &str, mail: &Outgoing) -> Result<String, RelayError> {
        self.send_campaign(to, mail).await
    }
}

/// Where a retryable fault leaves a row: due again later, or failed once the
/// attempts are spent.
fn retry_or_fail(claimed: &ClaimedMail, error: String, now: Timestamp) -> MailOutcome {
    if claimed.attempts >= MAX_ATTEMPTS {
        return MailOutcome::Failed { error };
    }
    let doublings = u32::try_from(claimed.attempts.saturating_sub(1)).unwrap_or(0);
    let wait = RETRY_BASE_MS.saturating_mul(2_i64.saturating_pow(doublings));
    MailOutcome::Retry {
        error,
        at: Timestamp(now.0.saturating_add(wait)),
    }
}

/// Whether the campaign asked for vouched-for addresses only. A missing or
/// unreadable flag reads as yes, the careful answer.
fn verified_only(claimed: &ClaimedMail) -> bool {
    claimed
        .audience
        .get("verified_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

/// The unsubscribe link a recipient's mail carries.
pub(crate) fn unsubscribe_url(console_url: &str, token: tam_types::Uuid) -> String {
    format!(
        "{}{UNSUBSCRIBE_PATH}?t={}",
        console_url.trim_end_matches('/'),
        token.to_hyphenated()
    )
}

/// One claimed row, sent or not, and what became of it.
///
/// Separate from the claim and the write-back so a test drives the whole
/// decision with no database and no network.
pub(crate) async fn send_one<R: AddressResolver, S: CampaignRelay>(
    claimed: &ClaimedMail,
    resolver: &R,
    relay: &S,
    console_url: &str,
    now: Timestamp,
) -> MailOutcome {
    if claimed.opted_out {
        return MailOutcome::Skipped {
            reason: "unsubscribed".to_owned(),
        };
    }
    let address = match resolver.address(claimed.auth_subject).await {
        Ok(address) => address,
        Err(ResolveError::Unreachable(why)) => return retry_or_fail(claimed, why, now),
        Err(ResolveError::NoAddress(_)) => {
            return MailOutcome::Skipped {
                reason: "no email address".to_owned(),
            }
        }
    };
    let Address {
        email,
        verified,
        name,
    } = address;
    if verified_only(claimed) && !verified {
        return MailOutcome::Skipped {
            reason: "email address not verified".to_owned(),
        };
    }
    let unsubscribe = unsubscribe_url(console_url, claimed.marketing_token);
    let date = long_date(now);
    let rendered = render(
        &Letter {
            subject: &claimed.subject,
            body_html: &claimed.body_html,
            link_url: claimed.link_url.as_deref(),
            link_label: claimed.link_label.as_deref(),
        },
        &Personal {
            name: name.as_deref(),
            email: &email,
            org: &claimed.org_name,
            plan: &claimed.plan,
            date: &date,
            unsubscribe_url: &unsubscribe,
        },
        console_url,
    );
    let mail = Outgoing {
        subject: rendered.subject,
        html: rendered.html,
        text: rendered.text,
        unsubscribe_url: unsubscribe,
    };
    match relay.send(&email, &mail).await {
        Ok(provider_id) => MailOutcome::Sent { provider_id },
        Err(RelayError::Retryable(why)) => retry_or_fail(claimed, why, now),
        Err(RelayError::Permanent(why)) => MailOutcome::Failed { error: why },
    }
}

/// The drainer's loop: claim, send, record, and space the sends.
#[expect(
    clippy::disallowed_methods,
    reason = "the campaign loop is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
pub(crate) fn spawn<R, S>(
    repo: MailCampaignRepo,
    resolver: R,
    relay: S,
    console_url: String,
    cancel: CancellationToken,
) where
    R: AddressResolver + 'static,
    S: CampaignRelay + 'static,
{
    tokio::spawn(async move {
        loop {
            let now = crate::wall_now();
            let claimed = match repo
                .claim(now, Timestamp(now.0.saturating_add(LEASE_MS)))
                .await
            {
                Ok(claimed) => claimed,
                Err(error) => {
                    eprintln!("tam-server: could not claim a campaign mail: {error}");
                    None
                }
            };
            let pause = if let Some(claimed) = claimed {
                let outcome = send_one(&claimed, &resolver, &relay, &console_url, now).await;
                if let MailOutcome::Failed { error } = &outcome {
                    eprintln!(
                        "tam-server: campaign {} to user {} failed: {error}",
                        claimed.campaign.to_hyphenated(),
                        claimed.user.0.to_hyphenated()
                    );
                }
                if let Err(error) = repo
                    .settle(claimed.campaign, claimed.user, &outcome, crate::wall_now())
                    .await
                {
                    eprintln!("tam-server: could not record a campaign mail's outcome: {error}");
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

    use tam_storage::{ClaimedMail, MailOutcome};
    use tam_types::{Timestamp, UserId, Uuid};

    use super::{send_one, CampaignRelay, Outgoing, MAX_ATTEMPTS};
    use crate::notify::{Address, AddressResolver, RelayError, ResolveError};

    const NOW: Timestamp = Timestamp(1_790_640_000_000);

    struct Resolver(Result<Address, ResolveError>);

    impl AddressResolver for Resolver {
        async fn address(&self, _subject: Uuid) -> Result<Address, ResolveError> {
            self.0.clone()
        }
    }

    struct Relay {
        answer: Result<String, RelayError>,
        sent: Mutex<Vec<(String, Outgoing)>>,
    }

    impl Relay {
        fn answering(answer: Result<String, RelayError>) -> Self {
            Self {
                answer,
                sent: Mutex::new(Vec::new()),
            }
        }

        async fn sent(&self) -> Vec<(String, Outgoing)> {
            self.sent.lock().await.clone()
        }
    }

    impl CampaignRelay for Relay {
        async fn send(&self, to: &str, mail: &Outgoing) -> Result<String, RelayError> {
            self.sent.lock().await.push((to.to_owned(), mail.clone()));
            self.answer.clone()
        }
    }

    fn claimed(opted_out: bool, verified_only: bool, attempts: i32) -> ClaimedMail {
        ClaimedMail {
            campaign: Uuid([1; 16]),
            user: UserId(Uuid([2; 16])),
            auth_subject: Uuid([3; 16]),
            org_name: "Ruiz & Co".to_owned(),
            plan: "Sync".to_owned(),
            attempts,
            subject: "News for @first_name".to_owned(),
            body_html: "<p>Hi @name at @org</p><p>@link</p>".to_owned(),
            link_url: Some("https://teachouse.io/new".to_owned()),
            link_label: None,
            audience: serde_json::json!({
                "segment": "all", "exclude_operators": true, "verified_only": verified_only
            }),
            opted_out,
            marketing_token: Uuid([0xAB; 16]),
        }
    }

    fn address(verified: bool) -> Address {
        Address {
            email: "ana@example.test".to_owned(),
            verified,
            name: Some("Ana Ruiz".to_owned()),
        }
    }

    #[tokio::test]
    async fn an_opted_out_seller_is_skipped_and_nothing_is_sent() {
        let relay = Relay::answering(Ok("id".to_owned()));
        let outcome = send_one(
            &claimed(true, true, 1),
            &Resolver(Ok(address(true))),
            &relay,
            "https://app.example.test",
            NOW,
        )
        .await;
        assert_eq!(
            outcome,
            MailOutcome::Skipped {
                reason: "unsubscribed".to_owned()
            }
        );
        assert!(relay.sent().await.is_empty());
    }

    #[tokio::test]
    async fn a_sent_mail_is_personal_and_carries_its_unsubscribe_link() {
        let relay = Relay::answering(Ok("re_123".to_owned()));
        let outcome = send_one(
            &claimed(false, true, 1),
            &Resolver(Ok(address(true))),
            &relay,
            "https://app.example.test/",
            NOW,
        )
        .await;
        assert_eq!(
            outcome,
            MailOutcome::Sent {
                provider_id: "re_123".to_owned()
            }
        );
        let sent = relay.sent().await;
        let [(to, mail)] = sent.as_slice() else {
            panic!("exactly one mail is sent, got {}", sent.len());
        };
        assert_eq!(to, "ana@example.test");
        assert_eq!(mail.subject, "News for Ana");
        assert!(mail.html.contains("Hi Ana Ruiz at Ruiz &amp; Co"));
        assert_eq!(
            mail.unsubscribe_url,
            "https://app.example.test/v1/mail/unsubscribe?t=abababab-abab-abab-abab-abababababab"
        );
        assert!(mail.html.contains(&mail.unsubscribe_url));
        assert!(mail.text.contains(&mail.unsubscribe_url));
    }

    #[tokio::test]
    async fn an_unverified_address_is_skipped_only_when_the_campaign_asked() {
        let relay = Relay::answering(Ok("id".to_owned()));
        let strict = send_one(
            &claimed(false, true, 1),
            &Resolver(Ok(address(false))),
            &relay,
            "https://app.example.test",
            NOW,
        )
        .await;
        assert!(matches!(strict, MailOutcome::Skipped { .. }));
        assert!(relay.sent().await.is_empty());
        let lenient = send_one(
            &claimed(false, false, 1),
            &Resolver(Ok(address(false))),
            &relay,
            "https://app.example.test",
            NOW,
        )
        .await;
        assert!(matches!(lenient, MailOutcome::Sent { .. }));
    }

    #[tokio::test]
    async fn a_retryable_fault_waits_longer_each_time_then_fails() {
        let relay = Relay::answering(Err(RelayError::Retryable("429".to_owned())));
        let resolver = Resolver(Ok(address(true)));
        let first = send_one(
            &claimed(false, true, 1),
            &resolver,
            &relay,
            "https://a.test",
            NOW,
        )
        .await;
        let second = send_one(
            &claimed(false, true, 2),
            &resolver,
            &relay,
            "https://a.test",
            NOW,
        )
        .await;
        let (MailOutcome::Retry { at: a, .. }, MailOutcome::Retry { at: b, .. }) = (first, second)
        else {
            panic!("both are retried");
        };
        assert_eq!(a.0 - NOW.0, 60_000);
        assert_eq!(b.0 - NOW.0, 120_000);
        let last = send_one(
            &claimed(false, true, MAX_ATTEMPTS),
            &resolver,
            &relay,
            "https://a.test",
            NOW,
        )
        .await;
        assert!(matches!(last, MailOutcome::Failed { .. }));
        let refused = send_one(
            &claimed(false, true, 1),
            &resolver,
            &Relay::answering(Err(RelayError::Permanent("422".to_owned()))),
            "https://a.test",
            NOW,
        )
        .await;
        assert_eq!(
            refused,
            MailOutcome::Failed {
                error: "422".to_owned()
            }
        );
        let unreachable = send_one(
            &claimed(false, true, 1),
            &Resolver(Err(ResolveError::Unreachable("down".to_owned()))),
            &relay,
            "https://a.test",
            NOW,
        )
        .await;
        assert!(matches!(unreachable, MailOutcome::Retry { .. }));
    }
}
