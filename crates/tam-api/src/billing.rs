//! Billing state: the Paddle webhook that records it, and the org-scoped read
//! that answers it.
//!
//! State tracking only. Nothing here gates a feature, meters a quota or
//! decides an entitlement; the table this writes is a record of what Paddle
//! said, and every other surface still serves every tenant exactly as before.
//!
//! The two routes are authenticated in two different ways, deliberately. The
//! read is [`OrgContext`] like every other tenant route, so a caller can only
//! ever see its own organisation's row. The webhook has no session and no
//! organisation extractor at all, because Paddle calls it and Paddle holds no
//! session: its signature over the raw bytes *is* the authentication, and the
//! organisation arrives inside the signed payload rather than from the
//! caller. That is why the handler takes the body as [`axum::body::Bytes`]
//! and verifies before it parses — a JSON value re-serialised for hashing is
//! a different byte string, and verifying that one proves nothing about what
//! arrived.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use tam_storage::{BillingRepo, SubscriptionState};
use tam_types::{OrgId, Timestamp, Uuid};

use crate::error::{APIError, APIErrorEntry, APIErrorKind};
use crate::paddle;
use crate::version::APIVersion;
use crate::{AppState, OrgContext};

/// The header Paddle signs each notification with.
const SIGNATURE_HEADER: &str = "Paddle-Signature";

/// The key our checkout puts the organisation identifier under in Paddle's
/// `custom_data`, and therefore the only place a webhook may learn which
/// tenant an event belongs to.
///
/// Named here rather than at the checkout, because this is the side that
/// cannot be changed unilaterally: a checkout that stopped setting it would
/// produce events this route ignores, silently, forever. One constant, two
/// readers.
pub const ORG_CUSTOM_DATA_KEY: &str = "org";

/// The three subscription events this milestone acts on. Every other event
/// type Paddle sends is acknowledged and ignored.
const SUBSCRIPTION_CREATED: &str = "subscription.created";
const SUBSCRIPTION_UPDATED: &str = "subscription.updated";
const SUBSCRIPTION_CANCELED: &str = "subscription.canceled";

/// The Paddle notification-webhook secret, held so it cannot reach a log line
/// or a `Debug` render of the configuration that carries it.
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

/// What a tenant's billing page reads. `subscription` is absent for an
/// organisation that has never reached checkout, which is a different fact
/// from a cancelled subscription — that one is present, carrying Paddle's
/// cancelled status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillingView {
    pub subscription: Option<SubscriptionView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionView {
    pub paddle_subscription_id: String,
    pub paddle_customer_id: String,
    /// Paddle's own vocabulary, passed through rather than translated. The
    /// client renders what it recognises; a value it does not is displayed
    /// rather than swallowed, and widening the set needs no release here.
    pub status: String,
    pub current_period_end: Option<Timestamp>,
    pub occurred_at: Timestamp,
}

impl SubscriptionView {
    fn of(state: SubscriptionState) -> Self {
        Self {
            paddle_subscription_id: state.paddle_subscription_id,
            paddle_customer_id: state.paddle_customer_id,
            status: state.status,
            current_period_end: state.current_period_end,
            occurred_at: state.occurred_at,
        }
    }
}

pub(crate) async fn billing_view(
    _version: APIVersion,
    State(state): State<AppState>,
    context: OrgContext,
) -> Result<Json<BillingView>, APIError> {
    let stored = BillingRepo::new(state.pool.clone())
        .get(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(BillingView {
        subscription: stored.map(SubscriptionView::of),
    }))
}

// ------------------------------------------------------------------ webhook

/// One notification's envelope, parsed only after its signature has been
/// verified over the bytes it arrived as.
///
/// Every field below `event_type` and `occurred_at` is optional because
/// Paddle's payload shape varies by event, and this route is reached by every
/// event type the notification setting subscribes to — not only the three it
/// acts on. Requiring a subscription's fields on an unrelated event would
/// turn an event we mean to ignore into one we refuse.
#[derive(Debug, Deserialize)]
struct Notification {
    event_type: String,
    occurred_at: String,
    #[serde(default)]
    data: NotificationData,
}

#[derive(Debug, Default, Deserialize)]
struct NotificationData {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    customer_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    current_billing_period: Option<BillingPeriod>,
    #[serde(default)]
    custom_data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct BillingPeriod {
    #[serde(default)]
    ends_at: Option<String>,
}

/// The organisation this notification speaks for, if it names one we can read.
///
/// Absent covers every way that can fail — no `custom_data`, no `org` key, a
/// value that is not a UUID — because the route's answer is the same for all
/// of them and distinguishing them here would only invite a log line keyed on
/// something a payload controls.
fn org_from(data: &NotificationData) -> Option<OrgId> {
    let raw = data
        .custom_data
        .as_ref()?
        .get(ORG_CUSTOM_DATA_KEY)?
        .as_str()?;
    Uuid::parse_hyphenated(raw).map(OrgId)
}

/// The state a handled subscription event describes, or `None` when the event
/// is one we handle but does not carry the fields that state is made of.
fn state_from(notification: &Notification) -> Option<SubscriptionState> {
    let data = &notification.data;
    let current_period_end = match data
        .current_billing_period
        .as_ref()
        .and_then(|period| period.ends_at.as_deref())
    {
        // A period that is present and unreadable is a refusal rather than an
        // absence: absent means Paddle carried no period, and quietly
        // recording that for a period we simply failed to parse would state
        // something Paddle did not.
        Some(raw) => Some(paddle::instant_from_rfc3339(raw).ok()?),
        None => None,
    };
    Some(SubscriptionState {
        paddle_subscription_id: data.id.clone()?,
        paddle_customer_id: data.customer_id.clone()?,
        status: data.status.clone()?,
        current_period_end,
        occurred_at: paddle::instant_from_rfc3339(&notification.occurred_at).ok()?,
    })
}

fn unconfigured() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new("no billing webhook secret is configured; nothing was recorded")
            .kind(APIErrorKind::Internal),
    )
}

/// One refusal for every way a signature fails, saying nothing about which
/// check it failed: the caller here is either Paddle, which never sees this,
/// or somebody probing, who learns nothing they can work with.
fn unsigned() -> APIError {
    APIError::new(
        StatusCode::UNAUTHORIZED,
        APIErrorEntry::new("the notification is not signed for this endpoint")
            .kind(APIErrorKind::Unauthenticated),
    )
}

/// A body that carried a valid signature and still could not be read as the
/// event it claims to be.
///
/// Refused rather than acknowledged, and refused only for the three event
/// types this route acts on. Paddle records a non-2xx notification as failed
/// and offers it for replay, so this is the answer that keeps a schema drift
/// visible and recoverable; acknowledging it would drop a real subscription
/// change silently, which is the one outcome nothing downstream could detect.
fn unreadable() -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new("the notification did not carry the state its event type describes")
            .kind(APIErrorKind::Validation),
    )
}

/// Paddle's notification endpoint. No session, no [`OrgContext`]: the
/// signature is the authentication and the organisation travels in the signed
/// payload.
///
/// `Bytes` is the last argument because it consumes the body, and it is the
/// body extractor rather than `Json` because the signature covers the bytes
/// as received. Reading them any other way would verify a re-rendering.
///
/// Acknowledged with 200 and no action: an event type this milestone does not
/// handle, and an event whose payload names no organisation we can resolve.
/// Both are ordinary — the notification setting may subscribe to more than
/// three event types, and a subscription created outside our checkout carries
/// no `custom_data` of ours. Neither is logged: the payload is Paddle's to
/// shape, and a log line per unhandled delivery is a disk-filling primitive
/// handed to whoever can cause one.
pub(crate) async fn webhook(
    _version: APIVersion,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, APIError> {
    let secret = state
        .config
        .paddle_webhook_secret
        .as_ref()
        .ok_or_else(unconfigured)?;
    let signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(unsigned)?;
    paddle::verify(signature, &body, secret.expose(), (state.wall)())
        .map_err(|_refusal| unsigned())?;

    let Ok(notification) = serde_json::from_slice::<Notification>(&body) else {
        return Err(unreadable());
    };
    if !matches!(
        notification.event_type.as_str(),
        SUBSCRIPTION_CREATED | SUBSCRIPTION_UPDATED | SUBSCRIPTION_CANCELED
    ) {
        return Ok(StatusCode::OK);
    }
    let Some(org) = org_from(&notification.data) else {
        return Ok(StatusCode::OK);
    };
    let subscription = state_from(&notification).ok_or_else(unreadable)?;

    // The answer is 200 whether the state landed or was declined as stale: a
    // reordered delivery is Paddle behaving as documented, not a fault to
    // report back to it.
    let _applied: bool = BillingRepo::new(state.pool.clone())
        .apply(org, &subscription, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::{org_from, state_from, Notification, WebhookSecret, ORG_CUSTOM_DATA_KEY};
    use tam_types::Timestamp;

    fn notification(payload: serde_json::Value) -> Notification {
        serde_json::from_value(payload).expect("the fixture payload is the envelope shape")
    }

    fn subscription_payload(custom_data: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "event_id": "evt_01",
            "event_type": "subscription.updated",
            "occurred_at": "2026-08-30T12:34:56.789123Z",
            "data": {
                "id": "sub_01",
                "customer_id": "ctm_01",
                "status": "active",
                "current_billing_period": {
                    "starts_at": "2026-08-30T12:34:56Z",
                    "ends_at": "2026-09-30T12:34:56Z"
                },
                "custom_data": custom_data.clone()
            }
        })
    }

    #[test]
    fn the_secret_does_not_render_itself() {
        let rendered = format!(
            "{:?}",
            WebhookSecret::new("pdl_ntfset_supersecret".to_owned())
        );
        assert!(
            !rendered.contains("supersecret"),
            "a configuration Debug must not carry the webhook secret: {rendered}"
        );
    }

    #[test]
    fn the_org_is_read_from_the_agreed_custom_data_key() {
        let event = notification(subscription_payload(
            &serde_json::json!({ ORG_CUSTOM_DATA_KEY: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa" }),
        ));
        let org = org_from(&event.data).expect("the payload names an organisation");
        assert_eq!(
            org.0.to_hyphenated(),
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "the identifier is read verbatim from custom_data"
        );
    }

    #[test]
    fn a_payload_that_names_no_readable_org_resolves_to_none() {
        for custom_data in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({ "organisation": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa" }),
            serde_json::json!({ ORG_CUSTOM_DATA_KEY: "not-a-uuid" }),
            serde_json::json!({ ORG_CUSTOM_DATA_KEY: 7 }),
        ] {
            let event = notification(subscription_payload(&custom_data));
            assert_eq!(
                org_from(&event.data),
                None,
                "{custom_data} names no organisation this route can resolve"
            );
        }
    }

    #[test]
    fn a_full_subscription_event_reads_as_the_state_it_describes() {
        let event = notification(subscription_payload(
            &serde_json::json!({ ORG_CUSTOM_DATA_KEY: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa" }),
        ));
        let state = state_from(&event).expect("the event carries its state");
        assert_eq!(state.paddle_subscription_id, "sub_01");
        assert_eq!(state.paddle_customer_id, "ctm_01");
        assert_eq!(
            state.status, "active",
            "Paddle's vocabulary is stored as received"
        );
        assert_eq!(
            state.occurred_at,
            Timestamp(1_788_093_296_789),
            "the event's own instant, truncated to milliseconds"
        );
        assert_eq!(
            state.current_period_end,
            Some(Timestamp(1_790_771_696_000)),
            "the period end is the billing period's ends_at"
        );
    }

    #[test]
    fn an_event_carrying_no_billing_period_records_no_period_end() {
        let mut payload = subscription_payload(&serde_json::json!({}));
        payload["data"]["current_billing_period"] = serde_json::Value::Null;
        let state = state_from(&notification(payload)).expect("the event still carries its state");
        assert_eq!(
            state.current_period_end, None,
            "an absent period is absent rather than fabricated"
        );
    }

    #[test]
    fn an_event_missing_a_field_its_state_is_made_of_reads_as_none() {
        for field in ["id", "customer_id", "status"] {
            let mut payload = subscription_payload(&serde_json::json!({}));
            payload["data"][field] = serde_json::Value::Null;
            assert!(
                state_from(&notification(payload)).is_none(),
                "an event with no {field} cannot be recorded and must refuse rather than \
                 land a half-row"
            );
        }
    }

    #[test]
    fn an_unreadable_instant_refuses_rather_than_recording_a_guess() {
        let mut payload = subscription_payload(&serde_json::json!({}));
        payload["occurred_at"] = serde_json::json!("30 August 2026");
        assert!(
            state_from(&notification(payload)).is_none(),
            "the ordering fence reads occurred_at, so a guess there corrupts the fence"
        );

        let mut payload = subscription_payload(&serde_json::json!({}));
        payload["data"]["current_billing_period"]["ends_at"] = serde_json::json!("soon");
        assert!(
            state_from(&notification(payload)).is_none(),
            "a period that is present and unreadable is not the same as no period"
        );
    }
}
