//! The mails this process sends: the seller's completion mail, the
//! operators' word of a new marketplace request, and the refund mail the
//! Payments page queues (`refund_mail`); who each goes to, what each says,
//! and the relay they go out through.
//!
//! Three seams, and the reason for each. The address comes from the identity
//! service rather than from this database, because `app_user.email` holds
//! `{subject}@subject.invalid` for every self-serve signup and the real
//! address is `tam-auth`'s; it is held for the length of one send and stored
//! nowhere, so the domain database keeps its property of not knowing any
//! seller's address. The relay is a seam so that a test proves the mail
//! without a network. And each template is a pure function of its payload, so
//! what a reader reads is asserted against a literal rather than eyeballed.
//!
//! Nothing here carries a resource title, a listing body, or any marketplace
//! credential or session. The completion mail's counts and the marketplace's
//! name are the whole of what it lets leave this process, and the run's own
//! page is where the titles are. The request mail carries what the seller
//! typed into the request form for us to read, and nothing else of theirs.

use std::future::Future;

use tam_engine::outbox::{Deliverer, DeliveryError};
use tam_storage::{
    NotificationRepo, OperatorRepo, OutboxMessage, Recipient, JOB_SETTLED_TOPIC,
    MARKETPLACE_REQUESTED_TOPIC,
};
use tam_types::{
    JobSettledNotice, Marketplace, MarketplaceRequestedNotice, NotificationCounts,
    NotificationKind, OrgId, Uuid,
};

/// Resend's send endpoint, which is one authenticated JSON POST.
const RESEND_ENDPOINT: &str = "https://api.resend.com/emails";

/// The header the internal address route is fenced by.
pub(crate) const INTERNAL_SECRET_HEADER: &str = "x-tam-internal-secret";

/// Both hops are to a neighbour or to one well-known relay, so these are
/// short. Named at all because the lint table refuses a client built without
/// them: an untimed fetch would hold a drain pass open indefinitely.
const HTTP_TIMEOUT_SECS: u64 = 10;
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 3;

/// One person's address as the identity service holds it, and the name it
/// holds beside it where there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Address {
    pub(crate) email: String,
    pub(crate) verified: bool,
    pub(crate) name: Option<String>,
}

/// Why an address could not be had. The split is the whole of the retry
/// decision: a service that is down will answer later and is retried, and a
/// subject that has no address it will vouch for will not, so that message
/// completes with nothing sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolveError {
    /// The identity service could not be reached or answered a fault.
    Unreachable(String),
    /// It answered, and this subject has no address it will vouch for.
    NoAddress(String),
}

/// Where a platform subject's address comes from.
pub(crate) trait AddressResolver: Send + Sync {
    fn address(&self, subject: Uuid) -> impl Future<Output = Result<Address, ResolveError>> + Send;
}

/// One composed mail. Held as data rather than posted directly so the template
/// is a pure function and the relay is a seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Mail {
    pub(crate) subject: String,
    pub(crate) html: String,
    /// The page the mail's one button opens: the run's own page, which is
    /// where the resource titles live, or the requesting organisation's.
    pub(crate) href: String,
    /// Where a reply goes, where that is not the sender: the request mail's
    /// is the seller who asked, so an operator answers them by replying.
    pub(crate) reply_to: Option<String>,
}

/// Why a relay refused. Split on the same retry question the resolver is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RelayError {
    Retryable(String),
    Permanent(String),
}

/// The mail relay.
pub(crate) trait Relay: Send + Sync {
    fn send(&self, to: &str, mail: &Mail) -> impl Future<Output = Result<(), RelayError>> + Send;
}

// ------------------------------------------------------------- configuration

/// Everything the mail path needs, as one decision rather than five.
///
/// All of it or none of it: a relay key with no sender address would compose a
/// mail nothing accepts, and a console origin missing would put a button in
/// front of a seller that goes nowhere. With none of it, [`select`] answers
/// [`Selected::Logging`] and the drainer behaves exactly as it did before this
/// module existed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MailConfig {
    pub(crate) resend_api_key: String,
    pub(crate) email_from: String,
    pub(crate) console_url: String,
    pub(crate) auth_internal_url: String,
    pub(crate) auth_internal_secret: String,
    /// The operations inbox a marketplace request goes to when no operator
    /// has an address the identity service vouches for. Optional, unlike the
    /// five above: without it such a request is on the operator listing and
    /// in a log line, and in nobody's inbox.
    pub(crate) ops_email: Option<String>,
    /// The sender the operators' campaigns go out from
    /// (`--marketing-email-from`), a second address so a campaign's
    /// reputation is not the one a seller's own completion mail rides on.
    pub(crate) marketing_email_from: String,
}

/// Which deliverer a deployment gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Selected {
    /// No mail configuration: log and succeed, which is what development and
    /// CI have always done.
    Logging,
    Email,
}

/// The choice, as a function of the configuration alone, so a test can assert
/// that an unconfigured deployment still selects the logging deliverer without
/// starting a process.
pub(crate) const fn select(mail: Option<&MailConfig>) -> Selected {
    match mail {
        None => Selected::Logging,
        Some(_) => Selected::Email,
    }
}

// -------------------------------------------------------------- the resolver

/// The identity service's internal address route.
pub(crate) struct AuthAddresses {
    client: reqwest::Client,
    base: String,
    secret: String,
}

impl AuthAddresses {
    pub(crate) fn new(base: &str, secret: &str) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: http_client()?,
            base: base.trim_end_matches('/').to_owned(),
            secret: secret.to_owned(),
        })
    }
}

impl AddressResolver for AuthAddresses {
    async fn address(&self, subject: Uuid) -> Result<Address, ResolveError> {
        let url = format!("{}/internal/address/{}", self.base, uuid_text(subject));
        let answer = self
            .client
            .get(&url)
            .header(INTERNAL_SECRET_HEADER, &self.secret)
            .send()
            .await
            .map_err(|error| {
                ResolveError::Unreachable(format!("the identity service is unreachable: {error}"))
            })?;
        let status = answer.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ResolveError::NoAddress(
                "the identity service holds no such subject".to_owned(),
            ));
        }
        if !status.is_success() {
            return Err(ResolveError::Unreachable(format!(
                "the identity service answered {status}"
            )));
        }
        let body: serde_json::Value = answer.json().await.map_err(|error| {
            ResolveError::Unreachable(format!(
                "the identity service's answer did not parse: {error}"
            ))
        })?;
        let email = body
            .get("email")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ResolveError::NoAddress("that subject has no address".to_owned()))?;
        Ok(Address {
            email: email.to_owned(),
            verified: body
                .get("emailVerified")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            name: body
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
        })
    }
}

// ----------------------------------------------------------------- the relay

pub(crate) struct ResendRelay {
    client: reqwest::Client,
    api_key: String,
    from: String,
}

impl ResendRelay {
    pub(crate) fn new(api_key: &str, from: &str) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: http_client()?,
            api_key: api_key.to_owned(),
            from: from.to_owned(),
        })
    }

    /// A second relay on the same account with another sender: the
    /// operators' campaigns go out from their own address.
    pub(crate) fn with_sender(&self, from: &str) -> Self {
        Self {
            client: self.client.clone(),
            api_key: self.api_key.clone(),
            from: from.to_owned(),
        }
    }

    /// One campaign mail, answering the relay's message id.
    ///
    /// Carries a plain-text part beside the HTML and the one-click
    /// unsubscribe headers (RFC 2369 and RFC 8058) that mailbox providers
    /// require of bulk senders.
    pub(crate) async fn send_campaign(
        &self,
        to: &str,
        mail: &crate::campaigns::Outgoing,
    ) -> Result<String, RelayError> {
        let body = serde_json::json!({
            "from": self.from,
            "to": [to],
            "subject": mail.subject,
            "html": mail.html,
            "text": mail.text,
            "headers": {
                "List-Unsubscribe": format!("<{}>", mail.unsubscribe_url),
                "List-Unsubscribe-Post": "List-Unsubscribe=One-Click",
            },
        });
        let answer = self
            .client
            .post(RESEND_ENDPOINT)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                RelayError::Retryable(format!("the mail relay is unreachable: {error}"))
            })?;
        relay_verdict(answer.status())?;
        // Accepted is accepted: a 2xx whose body carries no id still sent the
        // mail, and reporting it as a failure would invite a second copy.
        let receipt: serde_json::Value = answer.json().await.unwrap_or_default();
        Ok(receipt
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map_or_else(|| "(accepted, no id)".to_owned(), str::to_owned))
    }
}

impl Relay for ResendRelay {
    async fn send(&self, to: &str, mail: &Mail) -> Result<(), RelayError> {
        let mut body = serde_json::json!({
            "from": self.from,
            "to": [to],
            "subject": mail.subject,
            "html": mail.html,
        });
        if let Some(reply_to) = &mail.reply_to {
            body["reply_to"] = serde_json::Value::String(reply_to.clone());
        }
        let answer = self
            .client
            .post(RESEND_ENDPOINT)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                RelayError::Retryable(format!("the mail relay is unreachable: {error}"))
            })?;
        relay_verdict(answer.status())
    }
}

/// What a relay's status means for the attempt budget.
///
/// Pure and separate from the request so the classification is testable
/// without a network, which is the whole of why it is not inline: this is the
/// rule that decides whether a seller's mail is retried twelve times or
/// dead-lettered on the first pass, and it had no test while it lived inside
/// the `send` above.
///
/// Two 4xx statuses are retryable and the rest are not. RFC 9110 defines 429
/// as "try again later" — it is the status a healthy relay returns most often,
/// because it carries the per-second send limit — and 408 as the server giving
/// up on a request that may well succeed on the next one. Treating either as
/// permanent spends the entire twelve-attempt budget in a single drain pass on
/// the one answer that most deserves a wait. Every other 4xx is a refusal we
/// caused — a bad key, a sender the account does not own, an address the relay
/// rejects — and will refuse identically on every retry.
fn relay_verdict(status: reqwest::StatusCode) -> Result<(), RelayError> {
    if status.is_success() {
        return Ok(());
    }
    if matches!(
        status,
        reqwest::StatusCode::TOO_MANY_REQUESTS | reqwest::StatusCode::REQUEST_TIMEOUT
    ) {
        return Err(RelayError::Retryable(format!(
            "the mail relay asked us to wait: {status}"
        )));
    }
    if status.is_client_error() {
        return Err(RelayError::Permanent(format!(
            "the mail relay refused: {status}"
        )));
    }
    Err(RelayError::Retryable(format!(
        "the mail relay answered {status}"
    )))
}

pub(crate) fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(core::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .connect_timeout(core::time::Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .build()
}

// -------------------------------------------------------------- the template

/// Where a run's own page lives, which is the button's target and the console
/// row's link both.
#[must_use]
pub(crate) fn run_path(kind: NotificationKind, subject: Uuid) -> String {
    let id = uuid_text(subject);
    match kind {
        NotificationKind::Sync | NotificationKind::Migration => format!("/sync/requests/{id}"),
        NotificationKind::Import => format!("/imports/{id}"),
    }
}

/// What a seller calls the marketplace, matching the acronyms
/// `web/src/lib/platforms.ts` renders rather than the enum's own spelling.
const fn marketplace_name(marketplace: Marketplace) -> &'static str {
    match marketplace {
        Marketplace::Tes => "Tes",
        Marketplace::Etsy => "Etsy",
        Marketplace::Tpt => "TPT",
    }
}

/// What the run is called in a sentence.
const fn run_noun(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::Sync => "sync",
        NotificationKind::Migration => "migration",
        NotificationKind::Import => "import",
    }
}

/// The counts in the console's own outcome words, zeros dropped.
///
/// Dropping them is the point: a table of six rows five of which say nothing
/// is harder to read than the one row that does.
fn stated_counts(counts: NotificationCounts) -> Vec<(&'static str, u32)> {
    [
        ("succeeded", counts.succeeded),
        ("degraded", counts.degraded),
        ("failed", counts.failed),
        ("ambiguous", counts.ambiguous),
        ("skipped", counts.skipped),
        ("blocked", counts.blocked),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .collect()
}

/// Escapes the little that reaches the mail's markup at all.
///
/// Every interpolated value here is either a number or a string this process
/// chose from a closed set, so nothing a seller or a marketplace wrote can
/// reach it. This runs anyway, because the day one of those becomes a stored
/// value the escape must already be in the path rather than remembered.
fn escaped(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for character in raw.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// The mail, as a pure function of the payload and the console's origin.
///
/// Drawn in the Pounamu palette rather than the identity service's retired
/// Kauri one, because this is a new artefact and not a recolour of the two
/// identity mails. Mail clients strip `<style>` and resolve no custom
/// property, so every colour is inlined; each is a literal from
/// `web/src/lib/styles/tokens.css`, and the five are named here so a reader
/// can check them against the console rather than trust that they match:
/// `#f6f4f1` is `--ground`, `#fdfdfc` is `--surface`, `#17231c` is `--text`,
/// `#5a6560` is `--muted`, and `#1f4a38` is `--primary` with `#ffffff` its
/// `--on-fill`.
#[must_use]
pub(crate) fn compose(notice: &JobSettledNotice, console_url: &str) -> Mail {
    let place = notice.marketplace.map_or("catalogue", marketplace_name);
    let noun = run_noun(notice.kind);
    let total = notice.counts.total();
    let done = notice.counts.succeeded;
    let word = if total == 1 { "resource" } else { "resources" };
    // A run where everything succeeded states one figure; a run where anything
    // did not states both. `total` sums all six outcomes, so a run of nine
    // succeeded and two failed would otherwise subject itself "11 resources
    // updated", which reads as an achievement and is not one.
    let subject = if done == total {
        format!("Your {place} {noun} finished — {total} {word} updated")
    } else {
        format!("Your {place} {noun} finished — {done} of {total} {word} updated")
    };
    let href = format!(
        "{}{}",
        console_url.trim_end_matches('/'),
        run_path(notice.kind, notice.subject_id)
    );
    let rows =
        stated_counts(notice.counts)
            .into_iter()
            .fold(String::new(), |mut rows, (label, count)| {
                use core::fmt::Write as _;
                // infallible on String; the Result is the trait's, not the writer's
                let _unused: core::fmt::Result = write!(
                    rows,
                    "<tr><td style=\"padding:6px 16px 6px 0;color:#5a6560\">{}</td>\
                 <td style=\"padding:6px 0;font-weight:600;color:#17231c\">{count}</td></tr>",
                    escaped(label)
                );
                rows
            });
    let html = format!(
        "<div style=\"font-family:system-ui,-apple-system,'Segoe UI',sans-serif;\
           background:#f6f4f1;padding:32px 16px\">\
           <div style=\"max-width:520px;margin:0 auto;background:#fdfdfc;border-radius:12px;\
             padding:28px 32px;color:#17231c\">\
           <p style=\"margin:0 0 16px;font-size:16px\">Your {place} {noun} has finished.</p>\
           <table style=\"border-collapse:collapse;margin:0 0 24px;font-size:15px\">{rows}</table>\
           <a href=\"{href}\" style=\"display:inline-block;background:#1f4a38;color:#ffffff;\
             text-decoration:none;padding:10px 20px;border-radius:8px;font-weight:600\">\
             Open the run</a>\
           </div></div>",
        place = escaped(place),
        noun = escaped(noun),
        rows = rows,
        href = escaped(&href),
    );
    Mail {
        subject,
        html,
        href,
        reply_to: None,
    }
}

/// Where an organisation's page lives in the operator console.
#[must_use]
pub(crate) fn org_admin_path(org: OrgId) -> String {
    format!("/admin/orgs/{}", uuid_text(org.0))
}

/// Who asked for a marketplace, in the words the request mail names them by.
///
/// Decided by the deliverer from the identity service's answer, so the
/// template decides nothing about identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Requester {
    /// The name the identity service holds, where it holds one.
    pub(crate) name: Option<String>,
    /// How to reach them: their address, marked where the identity service
    /// does not vouch for it; their subject, which the operator console can
    /// look up, where it holds none; or a sentence saying there is neither.
    pub(crate) contact: String,
    /// The address a reply goes to, and only a vouched-for one: replying to
    /// an address nobody proved is how a reply reaches a stranger.
    pub(crate) reply_to: Option<String>,
}

/// The operators' mail about one new marketplace request, as a pure function
/// of the payload, who asked, and the console's origin.
///
/// Every value the seller typed, their name, and the organisation's name they
/// chose is escaped here: unlike the completion mail's, these are stored
/// strings a seller controls. The note keeps its line breaks, because a seller
/// who wrote two paragraphs meant two. A reply goes to the seller, so an
/// operator answers a request by replying to the mail about it.
///
/// In the completion mail's palette and for the same reasons; its comment
/// names each literal's token.
#[must_use]
pub(crate) fn compose_request(
    notice: &MarketplaceRequestedNotice,
    requester: &Requester,
    console_url: &str,
) -> Mail {
    let subject = format!("Marketplace request: {}", notice.marketplace_name);
    let href = format!(
        "{}{}",
        console_url.trim_end_matches('/'),
        org_admin_path(notice.org)
    );
    let row = |label: &str, value: &str| {
        format!(
            "<tr><td style=\"padding:6px 16px 6px 0;color:#5a6560;vertical-align:top\">{label}</td>\
             <td style=\"padding:6px 0;color:#17231c;white-space:pre-wrap\">{value}</td></tr>"
        )
    };
    let rows = [
        row(
            "Name",
            &escaped(requester.name.as_deref().unwrap_or("Not given")),
        ),
        row("Email", &escaped(&requester.contact)),
        row(
            "Organisation",
            &format!(
                "{} <span style=\"color:#5a6560\">({})</span>",
                escaped(&notice.org_name),
                uuid_text(notice.org.0)
            ),
        ),
        row("Marketplace", &escaped(&notice.marketplace_name)),
        row("Their note", &escaped(&notice.note)),
    ]
    .concat();
    // Promised only where it is true: without a vouched-for address a reply
    // goes back to the sender rather than to the seller.
    let lead = if requester.reply_to.is_some() {
        "A seller asked us to support a marketplace. Reply to this mail to answer them."
    } else {
        "A seller asked us to support a marketplace."
    };
    let html = format!(
        "<div style=\"font-family:system-ui,-apple-system,'Segoe UI',sans-serif;\
           background:#f6f4f1;padding:32px 16px\">\
           <div style=\"max-width:520px;margin:0 auto;background:#fdfdfc;border-radius:12px;\
             padding:28px 32px;color:#17231c\">\
           <p style=\"margin:0 0 16px;font-size:16px\">{lead}</p>\
           <table style=\"border-collapse:collapse;margin:0 0 24px;font-size:15px\">{rows}</table>\
           <a href=\"{href}\" style=\"display:inline-block;background:#1f4a38;color:#ffffff;\
             text-decoration:none;padding:10px 20px;border-radius:8px;font-weight:600\">\
             Open the organisation in admin</a>\
           </div></div>",
        href = escaped(&href),
    );
    Mail {
        subject,
        html,
        href,
        reply_to: requester.reply_to.clone(),
    }
}

/// Where a seller reads their own bills, which the refund mail's button
/// opens.
pub(crate) const BILLING_PATH: &str = "/settings/billing";

/// The `refund_issued` mail: one refund, told to the organisation that paid,
/// in the founder's voice.
///
/// The amount is the whole of what it says about the money; the charge, the
/// reason and the operator's note are ours and stay on the Payments page.
/// `name` is the recipient's own, where the identity service holds one.
/// In the completion mail's palette and for the same reasons; its comment
/// names each literal's token.
#[must_use]
pub(crate) fn compose_refund(
    amount: &str,
    org_name: &str,
    name: Option<&str>,
    console_url: &str,
) -> Mail {
    let href = format!("{}{BILLING_PATH}", console_url.trim_end_matches('/'));
    let greeting = match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("Kia ora {},", escaped(name)),
        None => "Kia ora,".to_owned(),
    };
    let html = format!(
        "<div style=\"font-family:system-ui,-apple-system,'Segoe UI',sans-serif;\
           background:#f6f4f1;padding:32px 16px\">\
           <div style=\"max-width:520px;margin:0 auto;background:#fdfdfc;border-radius:12px;\
             padding:28px 32px;color:#17231c\">\
           <p style=\"margin:0 0 16px;font-size:16px\">{greeting}</p>\
           <p style=\"margin:0 0 16px;font-size:16px\">We've refunded {amount} to your card. \
             It can take 5–10 business days to show.</p>\
           <p style=\"margin:0 0 24px;font-size:15px;color:#5a6560\">This is for {org}. \
             If anything looks wrong, reply to this email and I'll sort it out.</p>\
           <a href=\"{href}\" style=\"display:inline-block;background:#1f4a38;color:#ffffff;\
             text-decoration:none;padding:10px 20px;border-radius:8px;font-weight:600\">\
             See your bills</a>\
           </div></div>",
        amount = escaped(amount),
        org = escaped(org_name),
        href = escaped(&href),
    );
    Mail {
        subject: format!("We've refunded {amount}"),
        html,
        href,
        reply_to: None,
    }
}

/// Where the goodbye mail's button goes: signing up again.
pub(crate) const SIGNUP_PATH: &str = "/signup";

/// The goodbye a seller gets when they delete their own account, in the
/// founder's voice, sent before the identity service forgets the address.
///
/// It says what went and what stayed, the way the console's danger zone
/// said it before they pressed the button, and gives them one way to say "this
/// was not me": a reply. Nothing about the account itself is in it, because
/// by the time it lands there is no account. `name` is the recipient's own,
/// where the identity service held one. The palette is the refund mail's.
#[must_use]
pub(crate) fn compose_goodbye(name: Option<&str>, console_url: &str) -> Mail {
    let href = format!("{}{SIGNUP_PATH}", console_url.trim_end_matches('/'));
    let greeting = match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("Kia ora {},", escaped(name)),
        None => "Kia ora,".to_owned(),
    };
    let html = format!(
        "<div style=\"font-family:system-ui,-apple-system,'Segoe UI',sans-serif;\
           background:#f6f4f1;padding:32px 16px\">\
           <div style=\"max-width:520px;margin:0 auto;background:#fdfdfc;border-radius:12px;\
             padding:28px 32px;color:#17231c\">\
           <p style=\"margin:0 0 16px;font-size:16px\">{greeting}</p>\
           <p style=\"margin:0 0 16px;font-size:16px\">Your Teachouse account is deleted, \
             along with your catalogue and your device registrations. If you had a plan, \
             it's cancelled and you won't be charged again.</p>\
           <p style=\"margin:0 0 16px;font-size:15px;color:#5a6560\">Your listings on TPT \
             and Tes stay as they are, and the files on your devices are untouched.</p>\
           <p style=\"margin:0 0 24px;font-size:15px;color:#5a6560\">Thank you for teaching \
             with us. If you didn't ask for this, reply to this email straight away.</p>\
           <a href=\"{href}\" style=\"display:inline-block;background:#1f4a38;color:#ffffff;\
             text-decoration:none;padding:10px 20px;border-radius:8px;font-weight:600\">\
             Start again any time</a>\
           </div></div>",
        href = escaped(&href),
    );
    Mail {
        subject: "Your Teachouse account is deleted".to_owned(),
        html,
        href,
        reply_to: None,
    }
}

// --------------------------------------------------------------- the drainer

/// The real deliverer: the address, the mail and the relay.
///
/// Two repositories because two roles hold the reads: who in an organisation
/// wants a completion mail is the engine pool's column grant, and who is an
/// operator is the application pool's alone, the engine role being granted
/// nothing on `platform_operator`.
pub(crate) struct EmailDeliverer<R, S> {
    notifications: NotificationRepo,
    operators: OperatorRepo,
    resolver: R,
    relay: S,
    console_url: String,
    ops_email: Option<String>,
}

/// Where the mail points and where it falls back to: the console the links
/// open, and the operations inbox a marketplace request reaches when no
/// operator has a verified address.
#[derive(Clone, Copy)]
pub(crate) struct Sending<'a> {
    pub(crate) console_url: &'a str,
    pub(crate) ops_email: Option<&'a str>,
}

impl<R: AddressResolver, S: Relay> EmailDeliverer<R, S> {
    pub(crate) fn new(
        notifications: NotificationRepo,
        operators: OperatorRepo,
        resolver: R,
        relay: S,
        sending: Sending<'_>,
    ) -> Self {
        Self {
            notifications,
            operators,
            resolver,
            relay,
            console_url: sending.console_url.to_owned(),
            ops_email: sending.ops_email.map(str::to_owned),
        }
    }

    async fn deliver_notice(
        &self,
        org: OrgId,
        notice: &JobSettledNotice,
    ) -> Result<(), DeliveryError> {
        let recipients = self
            .notifications
            .recipients(org)
            .await
            .map_err(|error| DeliveryError::Retryable(error.to_string()))?;
        self.mail_to(&recipients, notice).await
    }

    /// The address a recipient may be mailed at, or `None` where there is
    /// none the identity service will vouch for.
    ///
    /// Unreachable is the one retryable answer, and it is logged with the
    /// subject it was asked about and carried into the outbox's own record of
    /// the attempt, so a dead letter says whose address could not be had. A
    /// subject with no address, or one it will not vouch for, is skipped with
    /// a logged line naming the subject and `mail` — which mail was not sent —
    /// and the caller carries on (founder decision A1, 2026-09-07). Retrying
    /// cannot change either answer, and a dead letter nothing reads is a worse
    /// record than the log line: under the earlier rule every seller whose
    /// provider never asserted verification simply stopped receiving mail. The
    /// subject is a platform id and not an address, so naming it costs nothing
    /// a log should not hold.
    async fn verified_address(
        &self,
        subject: Uuid,
        mail: &str,
    ) -> Result<Option<String>, DeliveryError> {
        let address = match self.resolver.address(subject).await {
            Ok(address) => address,
            Err(ResolveError::Unreachable(why)) => {
                let reason = format!("the address of subject {}: {why}", uuid_text(subject));
                eprintln!("tam-server: could not read {reason}; the {mail} is retried");
                return Err(DeliveryError::Retryable(reason));
            }
            Err(ResolveError::NoAddress(why)) => {
                eprintln!(
                    "tam-server: subject {} has no address to mail ({why}); its {mail} is not sent",
                    uuid_text(subject)
                );
                return Ok(None);
            }
        };
        if !address.verified {
            eprintln!(
                "tam-server: subject {} has an address the identity service does not vouch for; its {mail} is not sent",
                uuid_text(subject)
            );
            return Ok(None);
        }
        Ok(Some(address.email))
    }

    /// The mail itself, given who wants it.
    ///
    /// Separate from the read above so that a test drives the whole retry
    /// decision with no database at all, and so the read's own failure is
    /// unambiguously retryable rather than mixed into the resolver's answers.
    ///
    /// First error wins, and the whole message is retried or dead-lettered on
    /// it. With one user per organisation that is exactly the rule; with
    /// several, a retry can re-mail a recipient the earlier pass reached, which
    /// is the accepted cost of a single message per run. A recipient without
    /// a vouched-for address is skipped, as [`Self::verified_address`] says.
    async fn mail_to(
        &self,
        recipients: &[Recipient],
        notice: &JobSettledNotice,
    ) -> Result<(), DeliveryError> {
        let mail = compose(notice, &self.console_url);
        for recipient in recipients {
            let Some(address) = self
                .verified_address(recipient.subject, "completion mail")
                .await?
            else {
                continue;
            };
            self.relay
                .send(&address, &mail)
                .await
                .map_err(|error| match error {
                    RelayError::Retryable(why) => DeliveryError::Retryable(why),
                    RelayError::Permanent(why) => DeliveryError::Poison(why),
                })?;
        }
        Ok(())
    }

    async fn deliver_request(
        &self,
        notice: &MarketplaceRequestedNotice,
    ) -> Result<(), DeliveryError> {
        let operators = self
            .operators
            .mail_recipients()
            .await
            .map_err(|error| DeliveryError::Retryable(error.to_string()))?;
        self.mail_operators(&operators, notice).await
    }

    /// Who asked, as the request mail names them.
    ///
    /// Their name and address where the identity service holds them, the
    /// address marked when it does not vouch for it — an operator replying
    /// needs to know the reply may go nowhere — and their subject where it
    /// holds none, which the operator console can still look up. Unreachable
    /// is retried and logged, as it is for every address this process asks
    /// for.
    async fn requester(
        &self,
        notice: &MarketplaceRequestedNotice,
    ) -> Result<Requester, DeliveryError> {
        let Some(subject) = notice.requester_subject else {
            return Ok(Requester {
                name: None,
                contact: "a user with no sign-in identity".to_owned(),
                reply_to: None,
            });
        };
        match self.resolver.address(subject).await {
            Ok(Address {
                email,
                verified: true,
                name,
            }) => Ok(Requester {
                name,
                contact: email.clone(),
                reply_to: Some(email),
            }),
            Ok(Address {
                email,
                verified: false,
                name,
            }) => Ok(Requester {
                name,
                contact: format!("{email} (not verified)"),
                reply_to: None,
            }),
            Err(ResolveError::Unreachable(why)) => {
                let reason = format!(
                    "the address of requester subject {}: {why}",
                    uuid_text(subject)
                );
                eprintln!(
                    "tam-server: could not read {reason}; the marketplace request mail is retried"
                );
                Err(DeliveryError::Retryable(reason))
            }
            Err(ResolveError::NoAddress(_)) => Ok(Requester {
                name: None,
                contact: format!("subject {}", uuid_text(subject)),
                reply_to: None,
            }),
        }
    }

    /// Where the request mail goes: every operator with a vouched-for address,
    /// or the operations inbox where none has one.
    ///
    /// The fallback exists for the deployment where the operator marking and
    /// the identity service disagree — no operator granted yet, an operator
    /// who never verified, or an address route answering 404 because the
    /// shared secret is wrong — so that a seller's request still reaches a
    /// person. It is not added beside operators who were reached, because
    /// they are the people the operations inbox would forward it to.
    ///
    /// Every operator is resolved before anything is sent, so an unreachable
    /// identity service retries a message that mailed nobody.
    async fn request_recipients(
        &self,
        operators: &[Recipient],
    ) -> Result<Vec<(String, String)>, DeliveryError> {
        let mut recipients = Vec::with_capacity(operators.len());
        for operator in operators {
            if let Some(address) = self
                .verified_address(operator.subject, "marketplace request mail")
                .await?
            {
                recipients.push((
                    format!("operator subject {}", uuid_text(operator.subject)),
                    address,
                ));
            }
        }
        if recipients.is_empty() {
            if let Some(inbox) = &self.ops_email {
                eprintln!(
                    "tam-server: no operator has a verified address; the marketplace request mail goes to the operations inbox"
                );
                recipients.push(("the operations inbox".to_owned(), inbox.clone()));
            }
        }
        Ok(recipients)
    }

    /// One mail per recipient [`Self::request_recipients`] answers.
    ///
    /// Separate from the operator read for the reason [`Self::mail_to`] is.
    /// Nobody to mail — no operator with a vouched-for address and no
    /// operations inbox configured — completes with nothing sent and a logged
    /// line: the request is stored and listed whatever happens here, and
    /// retrying will not grant anybody the marking.
    ///
    /// The retry rule is the completion mail's with one difference. An
    /// unreachable identity service or a relay fault retries the whole
    /// message; a relay fault can re-mail a recipient an earlier pass reached,
    /// which is the accepted cost of one message per request. A relay's
    /// permanent refusal skips that one recipient with a logged line and
    /// carries on, and the message completes, rather than dead-lettering: a
    /// refusal is about one address, and poisoning on it would keep the
    /// request from everyone after it in the list.
    async fn mail_operators(
        &self,
        operators: &[Recipient],
        notice: &MarketplaceRequestedNotice,
    ) -> Result<(), DeliveryError> {
        let recipients = self.request_recipients(operators).await?;
        if recipients.is_empty() {
            eprintln!(
                "tam-server: no operator with a verified address and no operations inbox to tell about a marketplace request from organisation {}; it is on the operator listing",
                uuid_text(notice.org.0)
            );
            return Ok(());
        }
        let requester = self.requester(notice).await?;
        let mail = compose_request(notice, &requester, &self.console_url);
        for (who, address) in &recipients {
            match self.relay.send(address, &mail).await {
                Ok(()) => {}
                Err(RelayError::Retryable(why)) => {
                    eprintln!(
                        "tam-server: the relay faulted on the marketplace request mail to {who} ({why}); retried"
                    );
                    return Err(DeliveryError::Retryable(why));
                }
                Err(RelayError::Permanent(why)) => eprintln!(
                    "tam-server: the relay refused the marketplace request mail to {who} ({why}); not retried"
                ),
            }
        }
        Ok(())
    }
}

impl<R: AddressResolver, S: Relay> Deliverer for EmailDeliverer<R, S> {
    async fn deliver(&self, message: &OutboxMessage) -> Result<(), DeliveryError> {
        // A topic this build does not relay, and a payload it cannot read, both
        // succeed rather than dead-letter: delivery is unordered, and one
        // message the drainer does not understand must not stall the ones
        // behind it.
        if message.topic == MARKETPLACE_REQUESTED_TOPIC {
            let Ok(notice) =
                serde_json::from_value::<MarketplaceRequestedNotice>(message.payload.clone())
            else {
                eprintln!(
                    "tam-server: outbox message on {} carries no request summary; logged and dropped",
                    message.topic
                );
                return Ok(());
            };
            return self.deliver_request(&notice).await;
        }
        if message.topic != JOB_SETTLED_TOPIC {
            eprintln!(
                "tam-server: outbox topic {} has no relay yet; logged and dropped",
                message.topic
            );
            return Ok(());
        }
        let Ok(notice) = serde_json::from_value::<JobSettledNotice>(message.payload.clone()) else {
            eprintln!(
                "tam-server: outbox message on {} carries no run summary; logged and dropped",
                message.topic
            );
            return Ok(());
        };
        self.deliver_notice(message.org, &notice).await
    }
}

/// The 8-4-4-4-12 rendering, which is what the identity service's route keys
/// on.
fn uuid_text(id: Uuid) -> String {
    use core::fmt::Write as _;
    let mut out = String::with_capacity(36);
    for (index, byte) in id.0.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        compose, compose_request, relay_verdict, run_path, select, Address, AddressResolver,
        EmailDeliverer, Mail, MailConfig, Relay, RelayError, Requester, ResolveError, Selected,
    };
    use std::future::Future;
    use std::sync::{OnceLock, RwLock};
    use tam_engine::outbox::{Deliverer, DeliveryError};
    use tam_storage::{OutboxMessage, Recipient};
    use tam_types::{
        InventoryId, JobSettledNotice, Marketplace, MarketplaceRequestedNotice, NotificationCounts,
        NotificationKind, OrgId, Timestamp, Uuid,
    };

    fn notice(counts: NotificationCounts) -> JobSettledNotice {
        JobSettledNotice {
            kind: NotificationKind::Sync,
            subject_id: Uuid([0x2b; 16]),
            inventory: Some(InventoryId::Tes),
            marketplace: Some(Marketplace::Tes),
            counts,
            settled_at: Timestamp(1_757_164_800_000),
        }
    }

    fn twelve_succeeded() -> NotificationCounts {
        NotificationCounts {
            succeeded: 12,
            ..NotificationCounts::default()
        }
    }

    /// Answers whatever it was scripted with, per subject where a case names
    /// one and `answer` otherwise, and records the first subject it was asked
    /// about.
    ///
    /// `OnceLock` rather than a lock around a growing list: every case that
    /// reads it asks about one subject first, and that call is the record.
    struct RecordingResolver {
        answer: Result<Address, ResolveError>,
        asked: OnceLock<Uuid>,
        by_subject: Vec<(Uuid, Result<Address, ResolveError>)>,
    }

    impl AddressResolver for RecordingResolver {
        fn address(
            &self,
            subject: Uuid,
        ) -> impl Future<Output = Result<Address, ResolveError>> + Send {
            let _unused = self.asked.set(subject);
            let answer = self
                .by_subject
                .iter()
                .find(|(scripted, _)| *scripted == subject)
                .map_or_else(|| self.answer.clone(), |(_, answer)| answer.clone());
            core::future::ready(answer)
        }
    }

    /// Records every mail it was handed and sends none.
    ///
    /// A list, because the request mail goes to several operators and which
    /// of them received it is the assertion. `RwLock` rather than the banned
    /// `std::sync::Mutex`: a poisoned lock here fails the one test that
    /// panicked, which is what a test wants anyway.
    struct RecordingRelay {
        answer: Result<(), RelayError>,
        record: RwLock<Vec<(String, Mail)>>,
    }

    impl RecordingRelay {
        fn sent(&self) -> Vec<(String, Mail)> {
            self.record.read().expect("the record is readable").clone()
        }
    }

    impl Relay for RecordingRelay {
        fn send(
            &self,
            to: &str,
            mail: &Mail,
        ) -> impl Future<Output = Result<(), RelayError>> + Send {
            self.record
                .write()
                .expect("the record is writable")
                .push((to.to_owned(), mail.clone()));
            core::future::ready(self.answer.clone())
        }
    }

    #[test]
    fn an_unconfigured_deployment_keeps_the_logging_deliverer() {
        assert_eq!(
            select(None),
            Selected::Logging,
            "development and CI must behave exactly as they did before mail existed"
        );
        let configured = MailConfig {
            resend_api_key: "k".to_owned(),
            email_from: "runs@example.test".to_owned(),
            console_url: "https://app.example.test".to_owned(),
            auth_internal_url: "http://127.0.0.1:8081".to_owned(),
            auth_internal_secret: "s".to_owned(),
            ops_email: None,
            marketing_email_from: "no-reply@marketing.example.test".to_owned(),
        };
        assert_eq!(select(Some(&configured)), Selected::Email);
    }

    #[test]
    fn the_subject_line_states_the_marketplace_and_the_total() {
        let mail = compose(&notice(twelve_succeeded()), "https://app.example.test");
        assert_eq!(
            mail.subject, "Your Tes sync finished — 12 resources updated",
            "the subject names the marketplace, the run and what it did"
        );
    }

    /// A run that did not wholly succeed says so in the subject, rather than
    /// summing every outcome into one figure that reads as work done.
    #[test]
    fn a_mixed_run_states_what_succeeded_and_what_it_was_out_of() {
        let mail = compose(
            &notice(NotificationCounts {
                succeeded: 9,
                failed: 2,
                ..NotificationCounts::default()
            }),
            "https://app.example.test",
        );
        assert_eq!(
            mail.subject,
            "Your Tes sync finished — 9 of 11 resources updated"
        );
    }

    #[test]
    fn one_resource_is_not_pluralised() {
        let mail = compose(
            &notice(NotificationCounts {
                succeeded: 1,
                ..NotificationCounts::default()
            }),
            "https://app.example.test",
        );
        assert_eq!(mail.subject, "Your Tes sync finished — 1 resource updated");
    }

    #[test]
    fn the_body_states_only_the_counts_that_are_not_zero() {
        let mail = compose(
            &notice(NotificationCounts {
                succeeded: 9,
                failed: 2,
                ..NotificationCounts::default()
            }),
            "https://app.example.test",
        );
        assert!(mail.html.contains("succeeded"), "a stated count appears");
        assert!(mail.html.contains("failed"), "so does the other one");
        for absent in ["degraded", "ambiguous", "skipped", "blocked"] {
            assert!(
                !mail.html.contains(absent),
                "{absent} is zero and must not take a row"
            );
        }
    }

    #[test]
    fn the_button_points_at_the_runs_own_page() {
        let mail = compose(&notice(twelve_succeeded()), "https://app.example.test/");
        assert_eq!(
            mail.href,
            "https://app.example.test/sync/requests/2b2b2b2b-2b2b-2b2b-2b2b-2b2b2b2b2b2b",
            "the trailing slash is absorbed and the path is the run's own"
        );
        assert!(mail.html.contains(&mail.href), "the button carries it");
    }

    #[test]
    fn every_kind_has_a_page_to_open() {
        let subject = Uuid([0x11; 16]);
        assert_eq!(
            run_path(NotificationKind::Sync, subject),
            "/sync/requests/11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(
            run_path(NotificationKind::Migration, subject),
            "/sync/requests/11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(
            run_path(NotificationKind::Import, subject),
            "/imports/11111111-1111-1111-1111-111111111111"
        );
    }

    /// The privacy rule as a test rather than a habit: a payload carrying a
    /// title in a field the notice does not name renders nothing from it,
    /// because the notice is parsed into a closed shape before anything is
    /// composed.
    #[test]
    fn a_title_smuggled_into_the_payload_reaches_no_reader() {
        let mut payload =
            serde_json::to_value(notice(twelve_succeeded())).expect("a notice serialises");
        payload["title"] = serde_json::Value::String("Year 4 fractions pack".to_owned());
        payload["resources"] =
            serde_json::json!([{ "title": "Year 4 fractions pack", "url": "https://tes.test/x" }]);
        let parsed: JobSettledNotice =
            serde_json::from_value(payload).expect("the extra fields are ignored");
        let mail = compose(&parsed, "https://app.example.test");
        assert!(
            !mail.html.contains("fractions") && !mail.subject.contains("fractions"),
            "no resource title may reach a third party's mail logs"
        );
    }

    fn message(payload: serde_json::Value, topic: &str) -> OutboxMessage {
        OutboxMessage {
            org: OrgId(Uuid([0x77; 16])),
            id: Uuid([0x78; 16]),
            topic: topic.to_owned(),
            payload,
            attempts: 0,
        }
    }

    /// A pool is needed to build the deliverer and never connected to, because
    /// every case below refuses before the recipient and operator reads.
    /// `connect_lazy` is what makes that possible without a database.
    fn deliverer(
        resolver: RecordingResolver,
        relay: RecordingRelay,
    ) -> EmailDeliverer<RecordingResolver, RecordingRelay> {
        deliverer_with_ops(resolver, relay, None)
    }

    /// The same, with an operations inbox configured or not.
    fn deliverer_with_ops(
        resolver: RecordingResolver,
        relay: RecordingRelay,
        ops_email: Option<&str>,
    ) -> EmailDeliverer<RecordingResolver, RecordingRelay> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://tam_engine@127.0.0.1/unreachable")
            .expect("a lazy pool is built without connecting");
        EmailDeliverer::new(
            tam_storage::NotificationRepo::new(pool.clone()),
            tam_storage::OperatorRepo::new(pool),
            resolver,
            relay,
            super::Sending {
                console_url: "https://app.example.test",
                ops_email,
            },
        )
    }

    fn one_recipient() -> tam_storage::Recipient {
        tam_storage::Recipient {
            user: tam_types::UserId(Uuid([0x55; 16])),
            subject: Uuid([0x56; 16]),
        }
    }

    fn recorders() -> (RecordingResolver, RecordingRelay) {
        (
            RecordingResolver {
                answer: Ok(Address {
                    email: "sam@example.test".to_owned(),
                    verified: true,
                    name: None,
                }),
                asked: OnceLock::new(),
                by_subject: Vec::new(),
            },
            RecordingRelay {
                answer: Ok(()),
                record: RwLock::default(),
            },
        )
    }

    #[tokio::test]
    async fn a_topic_without_a_relay_succeeds_rather_than_stalling_the_queue() {
        let (resolver, relay) = recorders();
        let sent = deliverer(resolver, relay)
            .deliver(&message(serde_json::json!({}), "push.job_settled"))
            .await;
        assert_eq!(
            sent,
            Ok(()),
            "an unrelayed topic must not dead-letter the messages behind it"
        );
    }

    #[tokio::test]
    async fn a_payload_without_a_run_summary_succeeds_rather_than_stalling_the_queue() {
        let (resolver, relay) = recorders();
        let sent = deliverer(resolver, relay)
            .deliver(&message(
                serde_json::json!({ "event": "JobSettled" }),
                "email.job_settled",
            ))
            .await;
        assert_eq!(sent, Ok(()), "the pre-summary payload shape is not a fault");
    }

    #[tokio::test]
    async fn an_unreachable_address_service_is_retried() {
        let resolver = RecordingResolver {
            answer: Err(ResolveError::Unreachable("down".to_owned())),
            asked: OnceLock::new(),
            by_subject: Vec::new(),
        };
        let relay = RecordingRelay {
            answer: Ok(()),
            record: RwLock::default(),
        };
        let deliverer = deliverer(resolver, relay);
        let outcome = deliverer
            .mail_to(&[one_recipient()], &notice(twelve_succeeded()))
            .await;
        assert_eq!(
            outcome,
            Err(DeliveryError::Retryable(
                "the address of subject 56565656-5656-5656-5656-565656565656: down".to_owned()
            )),
            "a service that is down will answer later, so the attempt budget is kept"
        );
        assert!(
            deliverer.relay.sent().is_empty(),
            "nothing is handed to the relay without an address"
        );
    }

    /// Founder decision A1 (2026-09-07): an address the identity service does
    /// not vouch for, or a subject it holds no address for, completes the
    /// message with nothing sent rather than dead-lettering it. Retrying
    /// cannot change either answer, and a dead letter nothing reads was how a
    /// whole sign-in route's sellers silently stopped receiving mail.
    #[tokio::test]
    async fn an_unverified_or_absent_address_completes_with_nothing_sent() {
        for answer in [
            Ok(Address {
                email: "sam@example.test".to_owned(),
                verified: false,
                name: None,
            }),
            Err(ResolveError::NoAddress("no such subject".to_owned())),
        ] {
            let resolver = RecordingResolver {
                answer,
                asked: OnceLock::new(),
                by_subject: Vec::new(),
            };
            let relay = RecordingRelay {
                answer: Ok(()),
                record: RwLock::default(),
            };
            let deliverer = deliverer(resolver, relay);
            let outcome = deliverer
                .mail_to(&[one_recipient()], &notice(twelve_succeeded()))
                .await;
            assert_eq!(
                outcome,
                Ok(()),
                "the message completes: neither answer is a fault a later attempt fixes"
            );
            assert!(
                deliverer.relay.sent().is_empty(),
                "and nothing reaches the relay, because there is no address to send to"
            );
        }
    }

    #[tokio::test]
    async fn a_relays_refusal_is_permanent_and_its_fault_is_retried() {
        for (answer, permanent) in [
            (RelayError::Permanent("422".to_owned()), true),
            (RelayError::Retryable("503".to_owned()), false),
        ] {
            let (resolver, _unused) = recorders();
            let relay = RecordingRelay {
                answer: Err(answer),
                record: RwLock::default(),
            };
            let outcome = deliverer(resolver, relay)
                .mail_to(&[one_recipient()], &notice(twelve_succeeded()))
                .await;
            assert_eq!(
                matches!(outcome, Err(DeliveryError::Poison(_))),
                permanent,
                "the relay's own answer decides whether the budget is spent"
            );
        }
    }

    #[tokio::test]
    async fn the_composed_mail_reaches_the_address_the_identity_service_gave() {
        let (resolver, relay) = recorders();
        let deliverer = deliverer(resolver, relay);
        deliverer
            .mail_to(&[one_recipient()], &notice(twelve_succeeded()))
            .await
            .expect("the recorders accept it");
        assert_eq!(
            deliverer.resolver.asked.get(),
            Some(&Uuid([0x56; 16])),
            "the address is asked for by the recipient's own platform subject"
        );
        let (to, mail) = deliverer
            .relay
            .sent()
            .first()
            .cloned()
            .expect("one mail was handed over");
        assert_eq!(to, "sam@example.test", "and it goes where that answer said");
        assert_eq!(
            mail.subject,
            "Your Tes sync finished — 12 resources updated"
        );
    }

    /// The status-to-verdict rule, over literals rather than through a relay.
    ///
    /// 429 is the case this exists for: it is the one 4xx a healthy relay
    /// returns in normal traffic, and classifying it permanent dead-letters a
    /// seller's mail on attempt zero with no backoff entered and nothing
    /// reading `state = 'dead'` to say so.
    #[test]
    fn a_relay_status_decides_whether_the_budget_is_spent() {
        use reqwest::StatusCode;
        assert_eq!(relay_verdict(StatusCode::OK), Ok(()));
        assert_eq!(relay_verdict(StatusCode::ACCEPTED), Ok(()));
        for retryable in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(
                matches!(relay_verdict(retryable), Err(RelayError::Retryable(_))),
                "{retryable} is transient and must keep its attempts"
            );
        }
        for permanent in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::UNPROCESSABLE_ENTITY,
        ] {
            assert!(
                matches!(relay_verdict(permanent), Err(RelayError::Permanent(_))),
                "{permanent} refuses identically on every retry"
            );
        }
    }

    #[test]
    fn a_uuid_renders_as_the_identity_service_keys_on_it() {
        assert_eq!(
            super::uuid_text(Uuid([
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef
            ])),
            "01234567-89ab-cdef-0123-456789abcdef"
        );
    }

    const REQUESTER: Uuid = Uuid([0x61; 16]);

    fn request_notice() -> MarketplaceRequestedNotice {
        MarketplaceRequestedNotice {
            requester_subject: Some(REQUESTER),
            org: OrgId(Uuid([0x0c; 16])),
            org_name: "O'Brien & Daughters".to_owned(),
            marketplace_name: "Amped <Up> Learning".to_owned(),
            note: "Science units.\n<script>alert(\"x\")</script>".to_owned(),
        }
    }

    fn operator(byte: u8) -> Recipient {
        Recipient {
            user: tam_types::UserId(Uuid([byte; 16])),
            subject: Uuid([byte.wrapping_add(1); 16]),
        }
    }

    fn verified(email: &str) -> Address {
        Address {
            email: email.to_owned(),
            verified: true,
            name: None,
        }
    }

    fn seller() -> Requester {
        Requester {
            name: Some("Sam <O'Neill>".to_owned()),
            contact: "seller@example.test".to_owned(),
            reply_to: Some("seller@example.test".to_owned()),
        }
    }

    /// The whole of what an operator needs to answer a request: who asked,
    /// how to reach them, from which organisation, for which marketplace and
    /// why — and none of what a seller typed reaches the markup as markup:
    /// the note is a place a seller can write anything, and an operator's mail
    /// client is where it would run.
    #[test]
    fn the_request_mail_names_who_asked_where_and_why_escaped() {
        let mail = compose_request(&request_notice(), &seller(), "https://app.example.test/");
        assert_eq!(
            mail.subject, "Marketplace request: Amped <Up> Learning",
            "the subject names what was asked for"
        );
        for stated in [
            "Sam &lt;O&#39;Neill&gt;",
            "seller@example.test",
            "O&#39;Brien &amp; Daughters",
            "0c0c0c0c-0c0c-0c0c-0c0c-0c0c0c0c0c0c",
            "Amped &lt;Up&gt; Learning",
            "Science units.\n&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;",
            "Reply to this mail to answer them.",
            "Open the organisation in admin",
        ] {
            assert!(mail.html.contains(stated), "the mail states {stated:?}");
        }
        assert!(
            !mail.html.contains("<script>") && !mail.html.contains("<Up>"),
            "nothing a seller typed reaches the markup unescaped"
        );
        assert_eq!(
            mail.href, "https://app.example.test/admin/orgs/0c0c0c0c-0c0c-0c0c-0c0c-0c0c0c0c0c0c",
            "the button opens the asking organisation in the operator console"
        );
        assert!(
            mail.html.contains(&mail.href),
            "and the button carries that link"
        );
        assert_eq!(
            mail.reply_to.as_deref(),
            Some("seller@example.test"),
            "a reply reaches the seller who asked"
        );
    }

    /// A seller the identity service does not vouch for is named, and not
    /// replied to: the mail neither sets a reply address nor promises one.
    #[test]
    fn an_unverified_requester_is_not_the_reply_address() {
        let mail = compose_request(
            &request_notice(),
            &Requester {
                name: None,
                contact: "seller@example.test (not verified)".to_owned(),
                reply_to: None,
            },
            "https://app.example.test",
        );
        assert_eq!(mail.reply_to, None);
        assert!(mail.html.contains("seller@example.test (not verified)"));
        assert!(mail.html.contains("Not given"), "a missing name is said");
        assert!(
            !mail.html.contains("Reply to this mail"),
            "no reply is promised that would go back to the sender"
        );
    }

    #[tokio::test]
    async fn only_operators_with_a_vouched_for_address_are_told() {
        let (ana, ben, cai) = (operator(0x10), operator(0x20), operator(0x30));
        let resolver = RecordingResolver {
            answer: Err(ResolveError::NoAddress("unscripted".to_owned())),
            asked: OnceLock::new(),
            by_subject: vec![
                (
                    REQUESTER,
                    Ok(Address {
                        email: "seller@example.test".to_owned(),
                        verified: true,
                        name: Some("Sam Seller".to_owned()),
                    }),
                ),
                (ana.subject, Ok(verified("ana@example.test"))),
                (
                    ben.subject,
                    Ok(Address {
                        email: "ben@example.test".to_owned(),
                        verified: false,
                        name: None,
                    }),
                ),
                (cai.subject, Err(ResolveError::NoAddress("gone".to_owned()))),
            ],
        };
        let relay = RecordingRelay {
            answer: Ok(()),
            record: RwLock::default(),
        };
        let deliverer = deliverer(resolver, relay);
        let outcome = deliverer
            .mail_operators(&[ana, ben, cai], &request_notice())
            .await;
        assert_eq!(outcome, Ok(()), "skipping an operator is not a fault");
        let sent = deliverer.relay.sent();
        let recipients: Vec<&str> = sent.iter().map(|(to, _)| to.as_str()).collect();
        assert_eq!(
            recipients,
            ["ana@example.test"],
            "an unverified or absent address is not mailed"
        );
        assert!(
            sent[0].1.html.contains("seller@example.test") && sent[0].1.html.contains("Sam Seller"),
            "the requester is named by the name and address the identity service gave"
        );
        assert_eq!(
            sent[0].1.reply_to.as_deref(),
            Some("seller@example.test"),
            "and the operator's reply goes to them"
        );
    }

    /// No operator the identity service will vouch for — none granted, or
    /// every one unverified or unknown to it — sends the mail to the
    /// operations inbox instead, so a seller's request still reaches a person.
    #[tokio::test]
    async fn the_operations_inbox_is_told_when_no_operator_can_be() {
        let unknown = operator(0x20);
        let unknown_subject = unknown.subject;
        for operators in [Vec::new(), vec![unknown]] {
            let (resolver, relay) = recorders();
            let resolver = RecordingResolver {
                by_subject: vec![(
                    unknown_subject,
                    Err(ResolveError::NoAddress("gone".to_owned())),
                )],
                ..resolver
            };
            let deliverer = deliverer_with_ops(resolver, relay, Some("ops@example.test"));
            let outcome = deliverer
                .mail_operators(&operators, &request_notice())
                .await;
            assert_eq!(outcome, Ok(()));
            let recipients: Vec<String> = deliverer
                .relay
                .sent()
                .into_iter()
                .map(|(to, _)| to)
                .collect();
            assert_eq!(
                recipients,
                ["ops@example.test"],
                "with {} operator(s) and none reachable, the inbox is mailed once",
                operators.len()
            );
        }
    }

    /// The inbox is a fallback and not a copy: where an operator was mailed,
    /// it is not mailed as well.
    #[tokio::test]
    async fn the_operations_inbox_is_not_copied_when_an_operator_is_told() {
        let (resolver, relay) = recorders();
        let deliverer = deliverer_with_ops(resolver, relay, Some("ops@example.test"));
        deliverer
            .mail_operators(&[operator(0x10)], &request_notice())
            .await
            .expect("the recorders accept it");
        let recipients: Vec<String> = deliverer
            .relay
            .sent()
            .into_iter()
            .map(|(to, _)| to)
            .collect();
        assert_eq!(recipients, ["sam@example.test"]);
    }

    /// A requester the identity service holds no address for is still named,
    /// by the subject the operator console can look up.
    #[tokio::test]
    async fn a_requester_without_an_address_is_named_by_subject() {
        let resolver = RecordingResolver {
            answer: Ok(verified("ana@example.test")),
            asked: OnceLock::new(),
            by_subject: vec![(REQUESTER, Err(ResolveError::NoAddress("none".to_owned())))],
        };
        let relay = RecordingRelay {
            answer: Ok(()),
            record: RwLock::default(),
        };
        let deliverer = deliverer(resolver, relay);
        deliverer
            .mail_operators(&[operator(0x10)], &request_notice())
            .await
            .expect("the recorders accept it");
        let sent = deliverer.relay.sent();
        assert!(
            sent[0]
                .1
                .html
                .contains("subject 61616161-6161-6161-6161-616161616161"),
            "the mail names the subject instead"
        );
    }

    #[tokio::test]
    async fn no_operators_completes_with_nothing_sent() {
        let (resolver, relay) = recorders();
        let deliverer = deliverer(resolver, relay);
        let outcome = deliverer.mail_operators(&[], &request_notice()).await;
        assert_eq!(
            outcome,
            Ok(()),
            "retrying will not grant anybody the marking"
        );
        assert!(deliverer.relay.sent().is_empty(), "and nobody is mailed");
        assert!(
            deliverer.resolver.asked.get().is_none(),
            "and nobody's address is asked for"
        );
    }

    #[tokio::test]
    async fn an_unreachable_address_service_retries_the_request_mail() {
        let resolver = RecordingResolver {
            answer: Err(ResolveError::Unreachable("down".to_owned())),
            asked: OnceLock::new(),
            by_subject: Vec::new(),
        };
        let relay = RecordingRelay {
            answer: Ok(()),
            record: RwLock::default(),
        };
        let deliverer = deliverer(resolver, relay);
        let outcome = deliverer
            .mail_operators(&[operator(0x10)], &request_notice())
            .await;
        assert_eq!(
            outcome,
            Err(DeliveryError::Retryable(
                "the address of subject 11111111-1111-1111-1111-111111111111: down".to_owned()
            )),
            "the retried attempt records whose address could not be had"
        );
        assert!(deliverer.relay.sent().is_empty());
    }

    /// The one place the request mail's rule departs from the completion
    /// mail's: a relay refusing one operator's address must not keep the
    /// request from the operators after them, so it completes rather than
    /// dead-letters, while a relay fault still keeps its attempts.
    #[tokio::test]
    async fn a_relay_refusal_skips_one_operator_and_a_fault_is_retried() {
        for (answer, expected) in [
            (RelayError::Permanent("422".to_owned()), Ok(())),
            (
                RelayError::Retryable("503".to_owned()),
                Err(DeliveryError::Retryable("503".to_owned())),
            ),
        ] {
            let (resolver, _unused) = recorders();
            let relay = RecordingRelay {
                answer: Err(answer),
                record: RwLock::default(),
            };
            let deliverer = deliverer(resolver, relay);
            let outcome = deliverer
                .mail_operators(&[operator(0x10), operator(0x20)], &request_notice())
                .await;
            let attempted = deliverer.relay.sent().len();
            assert_eq!(outcome, expected);
            assert_eq!(
                attempted,
                if expected.is_ok() { 2 } else { 1 },
                "a refusal moves on to the next operator; a fault stops the pass"
            );
        }
    }

    /// The goodbye says what went and what stayed, greets by an escaped
    /// name, and its one button is signing up again.
    #[test]
    fn the_goodbye_mail_says_what_went_and_what_stayed() {
        let mail = super::compose_goodbye(Some("Aroha <b>"), "https://dash.example.test/");
        assert_eq!(mail.subject, "Your Teachouse account is deleted");
        assert_eq!(mail.href, "https://dash.example.test/signup");
        assert!(
            mail.html.contains("Kia ora Aroha &lt;b&gt;,"),
            "{}",
            mail.html
        );
        for said in [
            "Your Teachouse account is deleted",
            "your catalogue and your device registrations",
            "won't be charged again",
            "Your listings on TPT and Tes stay as they are",
            "the files on your devices are untouched",
            "reply to this email",
        ] {
            assert!(mail.html.contains(said), "missing {said:?}");
        }
        assert!(mail.reply_to.is_none());
        let anonymous = super::compose_goodbye(None, "https://dash.example.test");
        assert!(anonymous.html.contains("Kia ora,"));
    }
}
