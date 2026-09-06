//! The completion mail: who to send it to, what it says, and the relay it
//! goes out through.
//!
//! Three seams, and the reason for each. The address comes from the identity
//! service rather than from this database, because `app_user.email` holds
//! `{subject}@subject.invalid` for every self-serve signup and the real
//! address is `tam-auth`'s; it is held for the length of one send and stored
//! nowhere, so the domain database keeps its property of not knowing any
//! seller's address. The relay is a seam so that a test proves the mail
//! without a network. And the template is a pure function of the payload, so
//! what a seller reads is asserted against a literal rather than eyeballed.
//!
//! Nothing here carries a resource title, a listing body, or any marketplace
//! credential or session. The counts and the marketplace's name are the whole
//! of what leaves this process, and the run's own page is where the titles are.

use std::future::Future;

use tam_engine::outbox::{Deliverer, DeliveryError};
use tam_storage::{NotificationRepo, OutboxMessage, Recipient, JOB_SETTLED_TOPIC};
use tam_types::{JobSettledNotice, Marketplace, NotificationCounts, NotificationKind, OrgId, Uuid};

/// Resend's send endpoint, which is one authenticated JSON POST.
const RESEND_ENDPOINT: &str = "https://api.resend.com/emails";

/// The header the internal address route is fenced by.
pub(crate) const INTERNAL_SECRET_HEADER: &str = "x-tam-internal-secret";

/// Both hops are to a neighbour or to one well-known relay, so these are
/// short. Named at all because the lint table refuses a client built without
/// them: an untimed fetch would hold a drain pass open indefinitely.
const HTTP_TIMEOUT_SECS: u64 = 10;
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 3;

/// One seller's address as the identity service holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Address {
    pub(crate) email: String,
    pub(crate) verified: bool,
}

/// Why an address could not be had. The split is the whole of the retry
/// decision: a service that is down will answer later, and a subject that has
/// no verified address will not.
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
    /// The run's own page, which is the button's target and the one place the
    /// resource titles live.
    pub(crate) href: String,
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
}

impl Relay for ResendRelay {
    async fn send(&self, to: &str, mail: &Mail) -> Result<(), RelayError> {
        let body = serde_json::json!({
            "from": self.from,
            "to": [to],
            "subject": mail.subject,
            "html": mail.html,
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

fn http_client() -> Result<reqwest::Client, reqwest::Error> {
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
    }
}

// --------------------------------------------------------------- the drainer

/// The real deliverer: the address, the mail and the relay.
pub(crate) struct EmailDeliverer<R, S> {
    notifications: NotificationRepo,
    resolver: R,
    relay: S,
    console_url: String,
}

impl<R: AddressResolver, S: Relay> EmailDeliverer<R, S> {
    pub(crate) fn new(
        notifications: NotificationRepo,
        resolver: R,
        relay: S,
        console_url: &str,
    ) -> Self {
        Self {
            notifications,
            resolver,
            relay,
            console_url: console_url.to_owned(),
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

    /// The mail itself, given who wants it.
    ///
    /// Separate from the read above so that a test drives the whole retry
    /// decision with no database at all, and so the read's own failure is
    /// unambiguously retryable rather than mixed into the resolver's answers.
    ///
    /// First error wins, and the whole message is retried or dead-lettered on
    /// it. With one user per organisation that is exactly the rule; with
    /// several, a retry can re-mail a recipient the earlier pass reached, which
    /// is the accepted cost of a single message per run.
    async fn mail_to(
        &self,
        recipients: &[Recipient],
        notice: &JobSettledNotice,
    ) -> Result<(), DeliveryError> {
        let mail = compose(notice, &self.console_url);
        for recipient in recipients {
            let address = match self.resolver.address(recipient.subject).await {
                Ok(address) => address,
                Err(ResolveError::Unreachable(why)) => return Err(DeliveryError::Retryable(why)),
                Err(ResolveError::NoAddress(why)) => return Err(DeliveryError::Poison(why)),
            };
            if !address.verified {
                // Logged as well as returned, because the row this dead-letters
                // is written `state = 'dead'` and nothing reads dead letters
                // yet (R2). Without this line a seller whose identity provider
                // never asserted verification simply stops receiving mail, with
                // no record anywhere that they were dropped. The subject is a
                // platform id and not an address, so naming it costs nothing a
                // log should not hold.
                eprintln!(
                    "tam-server: subject {} has no verified address; its completion mail is dead-lettered",
                    uuid_text(recipient.subject)
                );
                return Err(DeliveryError::Poison(
                    "that subject's address is not verified, and retrying cannot verify it"
                        .to_owned(),
                ));
            }
            self.relay
                .send(&address.email, &mail)
                .await
                .map_err(|error| match error {
                    RelayError::Retryable(why) => DeliveryError::Retryable(why),
                    RelayError::Permanent(why) => DeliveryError::Poison(why),
                })?;
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
        compose, relay_verdict, run_path, select, Address, AddressResolver, EmailDeliverer, Mail,
        MailConfig, Relay, RelayError, ResolveError, Selected,
    };
    use std::future::Future;
    use std::sync::OnceLock;
    use tam_engine::outbox::{Deliverer, DeliveryError};
    use tam_storage::OutboxMessage;
    use tam_types::{
        InventoryId, JobSettledNotice, Marketplace, NotificationCounts, NotificationKind, OrgId,
        Timestamp, Uuid,
    };

    fn notice(counts: NotificationCounts) -> JobSettledNotice {
        JobSettledNotice {
            kind: NotificationKind::Sync,
            subject_id: Uuid([0x2b; 16]),
            inventory: Some(InventoryId::TesGb),
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

    /// Answers whatever it was scripted with and records the first subject it
    /// was asked about.
    ///
    /// `OnceLock` rather than a lock around a growing list: these cases mail one
    /// recipient, so the first call is the whole record, and the lint table bans
    /// `std::sync::Mutex` for a reason that applies to production code and would
    /// otherwise pull a runtime primitive into a recorder that needs none.
    struct RecordingResolver {
        answer: Result<Address, ResolveError>,
        asked: OnceLock<Uuid>,
    }

    impl AddressResolver for RecordingResolver {
        fn address(
            &self,
            subject: Uuid,
        ) -> impl Future<Output = Result<Address, ResolveError>> + Send {
            let _unused = self.asked.set(subject);
            core::future::ready(self.answer.clone())
        }
    }

    /// Records the first mail it was handed and sends none.
    struct RecordingRelay {
        answer: Result<(), RelayError>,
        sent: OnceLock<(String, Mail)>,
    }

    impl Relay for RecordingRelay {
        fn send(
            &self,
            to: &str,
            mail: &Mail,
        ) -> impl Future<Output = Result<(), RelayError>> + Send {
            let _unused = self.sent.set((to.to_owned(), mail.clone()));
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
    /// every case below refuses before the recipient read. `connect_lazy` is
    /// what makes that possible without a database.
    fn deliverer(
        resolver: RecordingResolver,
        relay: RecordingRelay,
    ) -> EmailDeliverer<RecordingResolver, RecordingRelay> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://tam_engine@127.0.0.1/unreachable")
            .expect("a lazy pool is built without connecting");
        EmailDeliverer::new(
            tam_storage::NotificationRepo::new(pool),
            resolver,
            relay,
            "https://app.example.test",
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
                }),
                asked: OnceLock::new(),
            },
            RecordingRelay {
                answer: Ok(()),
                sent: OnceLock::new(),
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
    async fn an_unreachable_address_service_is_retried_and_a_refused_one_is_not() {
        for (answer, expected) in [
            (
                Err(ResolveError::Unreachable("down".to_owned())),
                DeliveryError::Retryable("down".to_owned()),
            ),
            (
                Err(ResolveError::NoAddress("no such subject".to_owned())),
                DeliveryError::Poison("no such subject".to_owned()),
            ),
        ] {
            let resolver = RecordingResolver {
                answer,
                asked: OnceLock::new(),
            };
            let relay = RecordingRelay {
                answer: Ok(()),
                sent: OnceLock::new(),
            };
            let outcome = deliverer(resolver, relay)
                .mail_to(&[one_recipient()], &notice(twelve_succeeded()))
                .await;
            assert_eq!(
                outcome,
                Err(expected),
                "the retry decision follows the reason, not the failure"
            );
        }
    }

    #[tokio::test]
    async fn an_unverified_address_dead_letters_because_retrying_cannot_verify_it() {
        let resolver = RecordingResolver {
            answer: Ok(Address {
                email: "sam@example.test".to_owned(),
                verified: false,
            }),
            asked: OnceLock::new(),
        };
        let relay = RecordingRelay {
            answer: Ok(()),
            sent: OnceLock::new(),
        };
        let outcome = deliverer(resolver, relay)
            .mail_to(&[one_recipient()], &notice(twelve_succeeded()))
            .await;
        assert!(
            matches!(outcome, Err(DeliveryError::Poison(_))),
            "an unverified address is not a fault that a later attempt fixes"
        );
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
                sent: OnceLock::new(),
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
            .sent
            .get()
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
}
