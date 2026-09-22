//! Stripe's wire concerns: the webhook signature, the price map, the two
//! secrets, and the three outbound calls the checkout needs.
//!
//! The signature half is pure. Nothing in it opens a socket, reads a clock or
//! touches a database: the caller supplies the raw bytes, the header, the
//! secret and the current instant, so every branch is reachable from a unit
//! test with a constructed vector and no network.
//!
//! The scheme is Stripe's, documented at
//! <https://docs.stripe.com/webhooks#verify-manually>: the `Stripe-Signature`
//! header carries a `t` and one or more `v1` values, **comma** separated, and
//! each `v1` is HMAC-SHA256 over the timestamp, a full stop, and the raw
//! request body — the bytes as received, before any JSON parse, because a
//! re-serialised body is a different message.
//!
//! The client half is plain HTTP over `reqwest` rather than a vendor crate.
//! Three calls are needed and the official Rust binding is still a release
//! candidate; a form-encoded POST and a JSON read are less code than the
//! dependency and no less correct.

use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tam_limits::PriceKey;
use tam_types::Timestamp;

/// The header Stripe signs each event with.
pub const SIGNATURE_HEADER: &str = "Stripe-Signature";

/// How far an event's own timestamp may sit from our clock, in either
/// direction, before it is refused as a replay.
///
/// Stripe's own libraries default to five minutes, and that default is the
/// one adopted here rather than the tighter one the retired Paddle module
/// carried: this service runs in New Zealand, Stripe delivers from the
/// northern hemisphere, and a replayed body stays useless after five minutes
/// either way. The bound is symmetric because a timestamp ahead of our clock
/// is skew in the other direction, and treating it as acceptable would remove
/// the bound entirely for anyone able to set it.
pub const SIGNATURE_TOLERANCE_SECS: i64 = 300;

const MILLIS_PER_SEC: i64 = 1_000;

/// Where the outbound calls go. Overridden only by a test double.
const API_BASE: &str = "https://api.stripe.com";

/// Why one `Stripe-Signature` header did not authenticate its body.
///
/// The variants exist for tests and for a log line the operator reads; the
/// route answers the same refusal for all of them, because telling a caller
/// which check failed tells an attacker which one to work on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureRefusal {
    /// The header is not `t=<integer>,v1=<hex>[,v1=<hex>...]`.
    HeaderMalformed,
    /// The timestamp parses but sits further than [`SIGNATURE_TOLERANCE_SECS`]
    /// from the instant the caller supplied, in one direction or the other.
    TimestampOutOfTolerance,
    /// The header is well formed and timely, and no `v1` it carries equals
    /// the digest of these bytes under this secret.
    NoDigestMatched,
}

impl core::fmt::Display for SignatureRefusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::HeaderMalformed => "the Stripe-Signature header is malformed",
            Self::TimestampOutOfTolerance => "the Stripe-Signature timestamp is outside tolerance",
            Self::NoDigestMatched => "no Stripe-Signature digest matched the body",
        })
    }
}

/// The parsed header: the signed timestamp, and every digest offered for it.
///
/// More than one `v1` appears while an endpoint secret is being rotated, when
/// Stripe signs each event under both the old secret and the new one.
/// Accepting any of them is what makes a rotation a configuration change
/// rather than an outage. Schemes other than `v1` — `v0`, which Stripe uses
/// only for Connect test payloads — are ignored rather than refused, because
/// they are additional material rather than a malformation.
struct ParsedSignature<'a> {
    /// The timestamp exactly as it appeared, because it is signed material:
    /// re-rendering it from the parsed integer would change the message for
    /// any spelling that is not the canonical one.
    ts_raw: &'a str,
    ts_secs: i64,
    digests: Vec<&'a str>,
}

fn parse(header: &str) -> Result<ParsedSignature<'_>, SignatureRefusal> {
    let mut ts: Option<(&str, i64)> = None;
    let mut digests = Vec::new();
    for pair in header.split(',') {
        let (key, value) = pair
            .trim()
            .split_once('=')
            .ok_or(SignatureRefusal::HeaderMalformed)?;
        match key.trim() {
            "t" => {
                let raw = value.trim();
                let secs = raw
                    .parse::<i64>()
                    .map_err(|_| SignatureRefusal::HeaderMalformed)?;
                // A second `t` would leave which one was signed ambiguous,
                // and an attacker choosing the ambiguity is the whole attack.
                if ts.replace((raw, secs)).is_some() {
                    return Err(SignatureRefusal::HeaderMalformed);
                }
            }
            "v1" => digests.push(value.trim()),
            // Every other scheme Stripe may add is material this endpoint
            // does not read. Refusing it would turn Stripe widening its own
            // header into an outage here.
            _ => {}
        }
    }
    let (ts_raw, ts_secs) = ts.ok_or(SignatureRefusal::HeaderMalformed)?;
    if digests.is_empty() {
        return Err(SignatureRefusal::HeaderMalformed);
    }
    Ok(ParsedSignature {
        ts_raw,
        ts_secs,
        digests,
    })
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    let mut rendered = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // infallible on String; the Result is the trait's, not the writer's
        let _unused: core::fmt::Result = write!(rendered, "{byte:02x}");
    }
    rendered
}

/// Verifies one event's signature over the bytes as received.
///
/// `now` is the caller's instant rather than a clock read here, which is what
/// lets the tolerance test drive both edges of the window without waiting.
pub fn verify(
    header: &str,
    body: &[u8],
    secret: &str,
    now: Timestamp,
) -> Result<(), SignatureRefusal> {
    let signature = parse(header)?;
    let now_secs = now.0.div_euclid(MILLIS_PER_SEC);
    let skew = now_secs.saturating_sub(signature.ts_secs).saturating_abs();
    if skew > SIGNATURE_TOLERANCE_SECS {
        return Err(SignatureRefusal::TimestampOutOfTolerance);
    }

    let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .map_err(|_| SignatureRefusal::NoDigestMatched)?;
    mac.update(signature.ts_raw.as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = hex(&mac.finalize().into_bytes());

    // Every candidate is compared, and each comparison is length-checked and
    // then constant-time, so neither the number of digests offered nor how
    // far a wrong one agrees is observable in the time this takes.
    let matched = signature
        .digests
        .iter()
        .fold(subtle::Choice::from(0u8), |found, candidate| {
            found | candidate.as_bytes().ct_eq(expected.as_bytes())
        });
    if bool::from(matched) {
        Ok(())
    } else {
        Err(SignatureRefusal::NoDigestMatched)
    }
}

// ------------------------------------------------------------------ secrets

/// Stripe's endpoint signing secret (`whsec_…`), held so it cannot reach a log
/// line or a `Debug` render of the configuration that carries it.
#[derive(Clone, PartialEq, Eq)]
pub struct WebhookSecret(String);

impl WebhookSecret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The one legitimate use: keying the HMAC. Never logged, never echoed.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for WebhookSecret {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("WebhookSecret(redacted)")
    }
}

/// Stripe's restricted or secret API key (`sk_…`, `rk_…`), redacted for the
/// same reason and with the same shape.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey(String);

impl SecretKey {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SecretKey(redacted)")
    }
}

// ---------------------------------------------------------------- price map

/// Stripe's price identifiers mapped to what they sell.
///
/// Configuration rather than a constant, because a price identifier is minted
/// in Stripe's dashboard per environment: the test and live accounts name the
/// same product differently, and a table compiled in would make the binary
/// environment-specific. The server reads it from `--stripe-price-map`;
/// absent, the map is empty, checkout refuses and a completed session grants
/// nothing but says so on the log.
///
/// The value is a [`PriceKey`], not a plan and a rung: what a price sells is
/// named once in `tam-limits`, and the map only says which Stripe identifier
/// carries it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PriceMap(BTreeMap<String, PriceKey>);

/// Why a `--stripe-price-map` file is not one.
#[derive(Debug)]
pub enum PriceMapError {
    /// The file is not a JSON object of string to string.
    Shape(serde_json::Error),
    /// A value is not one of the price keys `tam-limits` names.
    UnknownKey(String),
}

impl core::fmt::Display for PriceMapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Shape(error) => write!(f, "{error}"),
            Self::UnknownKey(raw) => write!(
                f,
                "{raw:?} is not a price key; the keys are the ones tam-limits names"
            ),
        }
    }
}

impl std::error::Error for PriceMapError {}

impl PriceMap {
    /// Reads the JSON object the `--stripe-price-map` file carries:
    /// `{ "<stripe_price_id>": "sync_monthly", ... }`.
    ///
    /// Keyed by Stripe's identifier rather than by ours because that is the
    /// direction a webhook reads it in, and a webhook that could not name
    /// what a seller bought is the failure that costs money.
    pub fn parse(raw: &str) -> Result<Self, PriceMapError> {
        let raw: BTreeMap<String, String> =
            serde_json::from_str(raw).map_err(PriceMapError::Shape)?;
        let mut mapped = BTreeMap::new();
        for (price, key) in raw {
            let key = PriceKey::parse(&key).ok_or(PriceMapError::UnknownKey(key))?;
            mapped.insert(price, key);
        }
        Ok(Self(mapped))
    }

    /// What a Stripe price identifier sells.
    #[must_use]
    pub fn key_for(&self, price: &str) -> Option<PriceKey> {
        self.0.get(price).copied()
    }

    /// Which Stripe price identifier sells one key, which is what the
    /// checkout route needs. A key mapped twice answers the first
    /// identifier in identifier order, deterministically: two identifiers for
    /// one key is a misconfiguration, and answering it stably is what makes
    /// it diagnosable.
    #[must_use]
    pub fn price_for(&self, key: PriceKey) -> Option<&str> {
        self.0
            .iter()
            .find(|(_price, held)| **held == key)
            .map(|(price, _held)| price.as_str())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

// ------------------------------------------------------------------- client

/// Why an outbound Stripe call did not answer.
#[derive(Debug)]
pub enum StripeError {
    /// The request never reached a reply: DNS, TLS, connect or read.
    Transport(String),
    /// Stripe answered, and answered a refusal. The message is Stripe's own,
    /// which is safe to log and never safe to echo to a browser: it can name
    /// an account, a customer or a price.
    Api { status: u16, message: String },
    /// Stripe answered 2xx with a body this code could not read as the object
    /// it asked for.
    Malformed(String),
}

impl core::fmt::Display for StripeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Transport(detail) => write!(f, "the Stripe API was unreachable: {detail}"),
            Self::Api { status, message } => {
                write!(f, "the Stripe API answered {status}: {message}")
            }
            Self::Malformed(detail) => {
                write!(f, "the Stripe API answered a body we cannot read: {detail}")
            }
        }
    }
}

impl std::error::Error for StripeError {}

/// Which kind of Checkout Session to open. A recurring price needs
/// `subscription` and a one-off price needs `payment`; naming the wrong one is
/// the error Stripe answers rather than the charge it makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutMode {
    Payment,
    Subscription,
}

impl CheckoutMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Payment => "payment",
            Self::Subscription => "subscription",
        }
    }
}

/// One checkout to open. The organisation travels twice — as
/// `client_reference_id` and in `metadata` — because the two survive
/// different things: `client_reference_id` is on the session and nothing
/// else, and the metadata copy is what the subscription and the payment
/// intent inherit and therefore what a later `customer.subscription.updated`
/// still carries.
#[derive(Debug, Clone)]
pub struct CheckoutRequest<'a> {
    pub price: &'a str,
    pub mode: CheckoutMode,
    /// The organisation, hyphenated.
    pub org: &'a str,
    /// An existing Stripe customer to bill, where this tenant already has
    /// one. Absent, Stripe makes one and the webhook records it.
    pub customer: Option<&'a str>,
    pub success_url: &'a str,
    pub cancel_url: &'a str,
}

/// A Checkout Session as this code reads it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CheckoutSession {
    pub id: String,
    /// Where to send the browser. Absent on a session already completed,
    /// which is why the checkout route refuses rather than redirects to
    /// nothing.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    /// `paid`, `unpaid` or `no_payment_required`. Stripe's fulfilment
    /// guidance is to act on anything that is not `unpaid`.
    #[serde(default)]
    pub payment_status: Option<String>,
    #[serde(default)]
    pub client_reference_id: Option<String>,
    #[serde(default, deserialize_with = "expandable")]
    pub customer: Option<String>,
    #[serde(default, deserialize_with = "expandable")]
    pub subscription: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub line_items: Option<LineItems>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LineItems {
    #[serde(default)]
    pub data: Vec<LineItem>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LineItem {
    #[serde(default)]
    pub price: Option<PriceRef>,
    #[serde(default)]
    pub quantity: Option<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PriceRef {
    #[serde(default)]
    pub id: Option<String>,
}

impl CheckoutSession {
    /// Every price identifier this session's line items name, with the
    /// quantity each was bought at.
    #[must_use]
    pub fn purchased(&self) -> Vec<(&str, i64)> {
        self.line_items
            .as_ref()
            .map(|items| {
                items
                    .data
                    .iter()
                    .filter_map(|item| {
                        let price = item.price.as_ref()?.id.as_deref()?;
                        Some((price, item.quantity.unwrap_or(1)))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Reads a Stripe field that is either an identifier string or, when
/// expanded, the whole object carrying that identifier.
///
/// Both shapes arrive on the same field depending on what the request asked
/// to expand, and a reader that accepted only one would fail on a payload
/// Stripe considers equivalent.
fn expandable<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Either {
        Id(String),
        Object {
            #[serde(default)]
            id: Option<String>,
        },
        Absent,
    }
    Ok(match Option::<Either>::deserialize(deserializer)? {
        Some(Either::Id(id)) => Some(id),
        Some(Either::Object { id }) => id,
        Some(Either::Absent) | None => None,
    })
}

#[derive(Debug, Deserialize)]
struct PortalSession {
    url: String,
}

#[derive(Debug, Deserialize)]
struct ApiRefusal {
    #[serde(default)]
    error: Option<ApiRefusalBody>,
}

#[derive(Debug, Deserialize)]
struct ApiRefusalBody {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

/// The three outbound calls the checkout needs, and nothing else.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    key: SecretKey,
    base: String,
}

impl core::fmt::Debug for Client {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("stripe::Client")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Client {
    /// Two clients are the same client when they carry the same key against
    /// the same base. `Config` derives equality and a `reqwest::Client` has
    /// none; the key and the base are what a test is actually comparing.
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.base == other.base
    }
}

impl Eq for Client {}

/// How long one Stripe call may take end to end, and how long it may spend
/// reaching a socket.
///
/// A checkout call sits inside a request a teacher is waiting on, and the
/// session re-read sits inside a webhook Stripe will retry, so an outbound
/// call that never returns is worse in both directions than one that fails:
/// the teacher waits on a button that never answers, and the webhook holds a
/// connection Stripe has already given up on. Ten seconds is longer than any
/// of the three calls takes and short enough to be a fault rather than a
/// hang.
const CALL_TIMEOUT_SECS: u64 = 10;
const CONNECT_TIMEOUT_SECS: u64 = 5;

/// The one HTTP client this module makes, built with both bounds because
/// `reqwest::Client::new` has neither and the lint table says so.
///
/// A failed build is a client that refuses every call rather than a panic at
/// start-up: the only ways `build` fails are a TLS backend that did not
/// initialise and a system proxy that did not parse, and both are deployment
/// faults the billing routes should report as 503 rather than take the
/// process down for.
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(core::time::Duration::from_secs(CALL_TIMEOUT_SECS))
        .connect_timeout(core::time::Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .build()
        .unwrap_or_default()
}

impl Client {
    #[must_use]
    pub fn new(key: SecretKey) -> Self {
        Self {
            http: http_client(),
            key,
            base: API_BASE.to_owned(),
        }
    }

    /// The same client pointed at a local double. Test-only in practice, and
    /// public because the integration tests live outside this crate.
    #[must_use]
    pub fn with_base(key: SecretKey, base: String) -> Self {
        Self {
            http: http_client(),
            key,
            base,
        }
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        form: &[(String, String)],
    ) -> Result<T, StripeError> {
        let response = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(self.key.expose())
            .form(form)
            .send()
            .await
            .map_err(|error| StripeError::Transport(error.to_string()))?;
        Self::read(response).await
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, StripeError> {
        let response = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(self.key.expose())
            .send()
            .await
            .map_err(|error| StripeError::Transport(error.to_string()))?;
        Self::read(response).await
    }

    async fn read<T: serde::de::DeserializeOwned>(
        response: reqwest::Response,
    ) -> Result<T, StripeError> {
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| StripeError::Transport(error.to_string()))?;
        if !(200..300).contains(&status) {
            let message = serde_json::from_str::<ApiRefusal>(&body)
                .ok()
                .and_then(|refusal| refusal.error)
                .map(|error| match (error.message, error.code) {
                    (Some(message), Some(code)) => format!("{message} ({code})"),
                    (Some(message), None) => message,
                    (None, Some(code)) => code,
                    (None, None) => String::new(),
                })
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(StripeError::Api { status, message });
        }
        serde_json::from_str(&body).map_err(|error| StripeError::Malformed(error.to_string()))
    }

    /// Opens one Checkout Session and answers it, `url` included.
    ///
    /// `automatic_tax` is on because Teachouse is the seller of record under
    /// Stripe Payments and therefore owns the tax calculation; Stripe
    /// calculates against the registrations added in the dashboard and
    /// charges nothing where none applies.
    pub async fn create_checkout_session(
        &self,
        request: &CheckoutRequest<'_>,
    ) -> Result<CheckoutSession, StripeError> {
        let mut form = vec![
            ("mode".to_owned(), request.mode.as_str().to_owned()),
            ("line_items[0][price]".to_owned(), request.price.to_owned()),
            ("line_items[0][quantity]".to_owned(), "1".to_owned()),
            ("success_url".to_owned(), request.success_url.to_owned()),
            ("cancel_url".to_owned(), request.cancel_url.to_owned()),
            ("client_reference_id".to_owned(), request.org.to_owned()),
            ("metadata[org]".to_owned(), request.org.to_owned()),
            ("automatic_tax[enabled]".to_owned(), "true".to_owned()),
        ];
        match request.mode {
            // The organisation has to reach the object the later events carry,
            // and for a subscription that object is the subscription rather
            // than the session.
            CheckoutMode::Subscription => form.push((
                "subscription_data[metadata][org]".to_owned(),
                request.org.to_owned(),
            )),
            CheckoutMode::Payment => form.push((
                "payment_intent_data[metadata][org]".to_owned(),
                request.org.to_owned(),
            )),
        }
        if let Some(customer) = request.customer {
            form.push(("customer".to_owned(), customer.to_owned()));
            form.push(("customer_update[address]".to_owned(), "auto".to_owned()));
        } else if request.mode == CheckoutMode::Payment {
            // Without a customer Stripe still needs somewhere to put the
            // address `automatic_tax` resolves from, and `always` is what
            // makes one for a session that would otherwise be guest checkout.
            // Subscription mode always creates one and refuses the parameter.
            form.push(("customer_creation".to_owned(), "always".to_owned()));
        }
        self.post("/v1/checkout/sessions", &form).await
    }

    /// Opens a Billing Portal session for one customer, answering its URL.
    ///
    /// The URL is short-lived by Stripe's design — unused it expires in five
    /// minutes — so it is minted per click and never stored.
    pub async fn create_portal_session(
        &self,
        customer: &str,
        return_url: &str,
    ) -> Result<String, StripeError> {
        let form = vec![
            ("customer".to_owned(), customer.to_owned()),
            ("return_url".to_owned(), return_url.to_owned()),
        ];
        let session: PortalSession = self.post("/v1/billing_portal/sessions", &form).await?;
        Ok(session.url)
    }

    /// Re-reads one Checkout Session with its line items expanded.
    ///
    /// The webhook's own copy of the session carries no line items, and the
    /// line items are what name the price a seller chose. Stripe's fulfilment
    /// guidance is explicit that this read is the step that makes fulfilment
    /// correct rather than guessed from an amount.
    pub async fn retrieve_checkout_session(
        &self,
        id: &str,
    ) -> Result<CheckoutSession, StripeError> {
        self.get(&format!("/v1/checkout/sessions/{id}?expand[]=line_items"))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::{verify, SignatureRefusal, SIGNATURE_TOLERANCE_SECS};
    use hmac::{Hmac, Mac};
    use tam_types::Timestamp;

    const SECRET: &str = "whsec_the_development_endpoint_secret";
    const BODY: &[u8] = br#"{"id":"evt_01","type":"checkout.session.completed"}"#;
    const TS: i64 = 1_800_000_000;

    fn digest(ts: i64, body: &[u8], secret: &str) -> String {
        let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes())
            .expect("HMAC accepts a key of any length");
        mac.update(ts.to_string().as_bytes());
        mac.update(b".");
        mac.update(body);
        super::hex(&mac.finalize().into_bytes())
    }

    fn header(ts: i64, body: &[u8], secret: &str) -> String {
        format!("t={ts},v1={}", digest(ts, body, secret))
    }

    fn at(secs: i64) -> Timestamp {
        Timestamp(secs * 1_000)
    }

    #[test]
    fn stripe_a_correctly_signed_body_verifies() {
        assert_eq!(
            verify(&header(TS, BODY, SECRET), BODY, SECRET, at(TS)),
            Ok(()),
            "the vector this module constructs is the vector it accepts"
        );
    }

    #[test]
    fn stripe_the_signed_material_is_the_timestamp_a_full_stop_and_the_body() {
        // The one byte that separates this scheme from the retired Paddle
        // one. A colon here would verify nothing Stripe ever sends.
        let mut mac = <Hmac<sha2::Sha256> as Mac>::new_from_slice(SECRET.as_bytes())
            .expect("HMAC accepts a key of any length");
        mac.update(TS.to_string().as_bytes());
        mac.update(b":");
        mac.update(BODY);
        let colon = super::hex(&mac.finalize().into_bytes());
        assert_eq!(
            verify(&format!("t={TS},v1={colon}"), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::NoDigestMatched),
            "the separator is a full stop, and a colon must not authenticate"
        );
    }

    #[test]
    fn stripe_a_signature_under_another_secret_is_refused() {
        assert_eq!(
            verify(&header(TS, BODY, "another-secret"), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::NoDigestMatched),
            "only the configured secret authenticates an event"
        );
    }

    #[test]
    fn stripe_a_tampered_body_is_refused() {
        let signed = header(TS, BODY, SECRET);
        let tampered = br#"{"id":"evt_02","type":"checkout.session.completed"}"#;
        assert_eq!(
            verify(&signed, tampered, SECRET, at(TS)),
            Err(SignatureRefusal::NoDigestMatched),
            "the digest covers the body, so one changed byte refuses"
        );
    }

    #[test]
    fn stripe_a_restamped_header_is_refused() {
        let signed = header(TS, BODY, SECRET);
        let restamped = signed.replace(&TS.to_string(), &(TS + 1).to_string());
        assert_eq!(
            verify(&restamped, BODY, SECRET, at(TS + 1)),
            Err(SignatureRefusal::NoDigestMatched),
            "the timestamp is signed material, so moving it invalidates the digest"
        );
    }

    #[test]
    fn stripe_a_stale_timestamp_is_refused_beyond_the_tolerance() {
        let stale = TS - SIGNATURE_TOLERANCE_SECS - 1;
        assert_eq!(
            verify(&header(stale, BODY, SECRET), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::TimestampOutOfTolerance),
            "a captured signature stops being useful once the window closes"
        );
    }

    #[test]
    fn stripe_a_future_timestamp_is_refused_beyond_the_tolerance() {
        let ahead = TS + SIGNATURE_TOLERANCE_SECS + 1;
        assert_eq!(
            verify(&header(ahead, BODY, SECRET), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::TimestampOutOfTolerance),
            "the window is symmetric; skew forward is skew"
        );
    }

    #[test]
    fn stripe_the_tolerance_edges_are_inside_the_window() {
        for edge in [TS - SIGNATURE_TOLERANCE_SECS, TS + SIGNATURE_TOLERANCE_SECS] {
            assert_eq!(
                verify(&header(edge, BODY, SECRET), BODY, SECRET, at(TS)),
                Ok(()),
                "the bound is inclusive, so a Pacific crossing of exactly \
                 {SIGNATURE_TOLERANCE_SECS}s is accepted"
            );
        }
    }

    #[test]
    fn stripe_any_one_of_several_digests_authenticates_during_rotation() {
        let old = digest(TS, BODY, "the-retiring-secret");
        let new = digest(TS, BODY, SECRET);
        assert_eq!(
            verify(&format!("t={TS},v1={old},v1={new}"), BODY, SECRET, at(TS)),
            Ok(()),
            "the new secret's digest is accepted though it is not the first offered"
        );
        assert_eq!(
            verify(&format!("t={TS},v1={new},v1={old}"), BODY, SECRET, at(TS)),
            Ok(()),
            "and accepted when it is first, so order does not decide"
        );
    }

    #[test]
    fn stripe_several_wrong_digests_are_still_refused() {
        let a = digest(TS, BODY, "wrong-one");
        let b = digest(TS, BODY, "wrong-two");
        assert_eq!(
            verify(&format!("t={TS},v1={a},v1={b}"), BODY, SECRET, at(TS)),
            Err(SignatureRefusal::NoDigestMatched),
            "accepting any digest is not accepting every digest"
        );
    }

    #[test]
    fn stripe_an_unread_scheme_alongside_v1_does_not_refuse() {
        let good = digest(TS, BODY, SECRET);
        assert_eq!(
            verify(
                &format!("t={TS},v0=deadbeef,v1={good}"),
                BODY,
                SECRET,
                at(TS)
            ),
            Ok(()),
            "Stripe widening its own header is not an outage here"
        );
    }

    #[test]
    fn stripe_malformed_header_shapes_are_refused() {
        let good = digest(TS, BODY, SECRET);
        for (shape, why) in [
            (String::new(), "the empty header"),
            (format!("v1={good}"), "no timestamp at all"),
            (format!("t={TS}"), "a timestamp with no digest"),
            (format!("t=notanumber,v1={good}"), "an unparsable timestamp"),
            (format!("t={TS},v1={good},t={TS}"), "two timestamps"),
            (
                format!("t={TS};v1={good}"),
                "semicolon separators, which were Paddle's scheme rather than Stripe's",
            ),
            (format!("t={TS},{good}"), "a pair carrying no '='"),
        ] {
            assert_eq!(
                verify(&shape, BODY, SECRET, at(TS)),
                Err(SignatureRefusal::HeaderMalformed),
                "{why} is malformed"
            );
        }
    }

    #[test]
    fn stripe_whitespace_around_the_pairs_is_tolerated() {
        let good = digest(TS, BODY, SECRET);
        assert_eq!(
            verify(&format!(" t={TS} , v1={good} "), BODY, SECRET, at(TS)),
            Ok(()),
            "padding around the separators does not change the signed material"
        );
    }
}
