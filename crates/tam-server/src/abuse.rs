//! Abuse prevention's two loops (`docs/notes/design/abuse-prevention.md`):
//!
//! - the nightly pass: copy card fingerprints out of the payments ledger,
//!   run the scorer over every cluster, and keep the retention promises
//!   (addresses and browsers after ninety days, bans after twenty-four
//!   months);
//! - the mail drainer: the warning or suspension an operator's decision
//!   queued on its flag (migration 0106), sent the way refund mail is
//!   (`refund_mail`), with the outcome written back to the flag the Abuse
//!   page shows.

use core::time::Duration;

use tam_api::AppState;
use tam_storage::{AbuseAction, AbuseMailOutcome, AbuseRepo, ClaimedAbuseMail, ABUSE_MAIL_ATTEMPTS};
use tam_types::Timestamp;
use tokio_util::sync::CancellationToken;

use crate::notify::{escaped, AddressResolver, Mail, Relay, RelayError, ResolveError};

/// How often the nightly pass runs. Once a day, the first a minute after
/// start-up so a restart does not skip a night.
const PASS_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const FIRST_PASS: Duration = Duration::from_secs(60);
/// How long the drainer waits when nothing is due.
const IDLE: Duration = Duration::from_secs(5);
const SEND_INTERVAL: Duration = Duration::from_millis(200);
const LEASE_MS: i64 = 120_000;
const RETRY_BASE_MS: i64 = 60_000;

/// The rule a suspension mail points at: the Terms' one-free-account
/// section, on the public site.
const RULE_URL: &str = "https://teachouse.io/terms/#one-account";

/// One nightly pass. Each step's failure is reported and the next still
/// runs: a sweep that could not read the payments ledger is no reason to
/// keep an address past its ninety days.
pub(crate) async fn pass(state: &AppState, now: Timestamp) {
    match tam_api::abuse::harvest_cards(state, None).await {
        Ok(0) => {}
        Ok(cards) => eprintln!("tam-server: abuse pass recorded {cards} card fingerprints"),
        Err(error) => eprintln!("tam-server: abuse pass could not read card fingerprints: {error}"),
    }
    let repo = AbuseRepo::new(state.pool.clone());
    match repo.score(None, now).await {
        Ok(scored) if scored.raised > 0 => {
            eprintln!("tam-server: abuse pass raised {} flags", scored.raised);
        }
        Ok(_) => {}
        Err(error) => eprintln!("tam-server: abuse scorer failed: {error}"),
    }
    match repo.prune(now).await {
        Ok(pruned) if pruned.signals + pruned.bans > 0 => eprintln!(
            "tam-server: abuse retention removed {} signals and {} expired bans",
            pruned.signals, pruned.bans
        ),
        Ok(_) => {}
        Err(error) => eprintln!("tam-server: abuse retention failed: {error}"),
    }
}

/// The nightly pass's loop.
#[expect(
    clippy::disallowed_methods,
    reason = "the abuse pass is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
pub(crate) fn spawn_pass(state: AppState, cancel: CancellationToken) {
    tokio::spawn(async move {
        let mut wait = FIRST_PASS;
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                () = tokio::time::sleep(wait) => {}
            }
            pass(&state, crate::wall_now()).await;
            wait = PASS_INTERVAL;
        }
    });
}

/// The warning or the suspension, in the founder's voice. Nothing about the
/// evidence is in it: the operator's reason stays on the Abuse page, and a
/// mail naming another account would tell one seller about another.
#[must_use]
pub(crate) fn compose(action: AbuseAction, org_name: &str, name: Option<&str>, console_url: &str) -> Mail {
    let greeting = match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("Kia ora {},", escaped(name)),
        None => "Kia ora,".to_owned(),
    };
    let (subject, lead, closing, href, button) = if action == AbuseAction::Ban {
        (
            "Your Teachouse account is suspended",
            "We've suspended your Teachouse account because it broke our one free account per person rule.",
            "If you think we've got this wrong, reply to this email or write to contact@teachouse.io and I'll look at it myself.",
            RULE_URL.to_owned(),
            "Read the rule",
        )
    } else {
        (
            "About your Teachouse account",
            "Your account looks linked to other Teachouse accounts. The free plan's five moves are once per shop and one free account per person.",
            "If these accounts are yours, please keep to one. If we've got this wrong, just reply and I'll sort it out.",
            console_url.trim_end_matches('/').to_owned(),
            "Open Teachouse",
        )
    };
    let html = format!(
        "<div style=\"font-family:system-ui,-apple-system,'Segoe UI',sans-serif;\
           background:#f6f4f1;padding:32px 16px\">\
           <div style=\"max-width:520px;margin:0 auto;background:#fdfdfc;border-radius:12px;\
             padding:28px 32px;color:#17231c\">\
           <p style=\"margin:0 0 16px;font-size:16px\">{greeting}</p>\
           <p style=\"margin:0 0 16px;font-size:16px\">{lead}</p>\
           <p style=\"margin:0 0 24px;font-size:15px;color:#5a6560\">This is about {org}. {closing}</p>\
           <a href=\"{href_html}\" style=\"display:inline-block;background:#1f4a38;color:#ffffff;\
             text-decoration:none;padding:10px 20px;border-radius:8px;font-weight:600\">\
             {button}</a>\
           </div></div>",
        org = escaped(org_name),
        href_html = escaped(&href),
    );
    Mail {
        subject: subject.to_owned(),
        html,
        href,
        reply_to: None,
    }
}

fn retry_or_fail(claimed: &ClaimedAbuseMail, error: String, now: Timestamp) -> AbuseMailOutcome {
    if claimed.attempts >= ABUSE_MAIL_ATTEMPTS {
        return AbuseMailOutcome::Failed { error };
    }
    let doublings = u32::try_from(claimed.attempts.saturating_sub(1)).unwrap_or(0);
    let wait = RETRY_BASE_MS.saturating_mul(2_i64.saturating_pow(doublings));
    AbuseMailOutcome::Retry {
        error,
        at: Timestamp(now.0.saturating_add(wait)),
    }
}

/// One claimed mail, sent to every person in the organisation with a
/// verified address. The refund mail's rules: every address resolved first,
/// nobody reachable is a failure the page shows, a relay fault retries.
pub(crate) async fn send_one<R: AddressResolver, S: Relay>(
    claimed: &ClaimedAbuseMail,
    resolver: &R,
    relay: &S,
    console_url: &str,
    now: Timestamp,
) -> AbuseMailOutcome {
    let mut recipients = Vec::with_capacity(claimed.subjects.len());
    for subject in &claimed.subjects {
        match resolver.address(*subject).await {
            Ok(address) if address.verified => recipients.push(address),
            Ok(_) | Err(ResolveError::NoAddress(_)) => {}
            Err(ResolveError::Unreachable(why)) => return retry_or_fail(claimed, why, now),
        }
    }
    if recipients.is_empty() {
        return AbuseMailOutcome::Failed {
            error: "Nobody in this organisation has a verified email address.".to_owned(),
        };
    }
    for address in &recipients {
        let mail = compose(
            claimed.action,
            &claimed.org_name,
            address.name.as_deref(),
            console_url,
        );
        match relay.send(&address.email, &mail).await {
            Ok(()) => {}
            Err(RelayError::Retryable(why)) => return retry_or_fail(claimed, why, now),
            Err(RelayError::Permanent(why)) => return AbuseMailOutcome::Failed { error: why },
        }
    }
    AbuseMailOutcome::Sent
}

/// The drainer's loop.
#[expect(
    clippy::disallowed_methods,
    reason = "the abuse mail loop is owned by the serving process and stopped by its cancellation token, not a fire-and-forget spawn"
)]
pub(crate) fn spawn_mail<R, S>(
    repo: AbuseRepo,
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
                    eprintln!("tam-server: could not claim an abuse mail: {error}");
                    None
                }
            };
            let pause = if let Some(claimed) = claimed {
                let outcome = send_one(&claimed, &resolver, &relay, &console_url, now).await;
                if let AbuseMailOutcome::Failed { error } = &outcome {
                    eprintln!(
                        "tam-server: the mail for abuse flag {} was not sent: {error}",
                        claimed.flag.to_hyphenated()
                    );
                }
                if let Err(error) = repo
                    .settle_mail(claimed.flag, &outcome, crate::wall_now())
                    .await
                {
                    eprintln!("tam-server: could not record an abuse mail's outcome: {error}");
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
    use tam_storage::AbuseAction;

    use super::compose;

    #[test]
    fn the_warning_and_the_suspension_say_different_things_and_name_no_evidence() {
        let warn = compose(AbuseAction::Warn, "Kiwi <Maths>", Some("Ana"), "https://dash.teachouse.io/");
        assert_eq!(warn.subject, "About your Teachouse account");
        assert!(warn.html.contains("Kia ora Ana,"));
        assert!(warn.html.contains("Kiwi &lt;Maths&gt;"), "the name is escaped");
        assert_eq!(warn.href, "https://dash.teachouse.io");

        let ban = compose(AbuseAction::Ban, "Kiwi Maths", None, "https://dash.teachouse.io");
        assert_eq!(ban.subject, "Your Teachouse account is suspended");
        assert!(ban.html.contains("Kia ora,"));
        assert!(ban.html.contains("contact@teachouse.io"));
        assert_eq!(ban.href, "https://teachouse.io/terms/#one-account");
    }
}
