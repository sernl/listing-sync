//! The product-analytics capture path: what leaves this process for PostHog,
//! and the two properties that make it safe to have at all.
//!
//! **Who an event is about is an organisation, never a person.** The distinct
//! id is the organisation's hyphenated UUID and nothing else. No address, no
//! slug, no marketplace handle and no resource title crosses this boundary;
//! the caller's `props` are a closed set of counts, enum spellings and
//! booleans chosen at each call site, and this module never writes them to a
//! log, so a mistake at a call site cannot be compounded into a log file that
//! outlives the send.
//!
//! **A capture cannot change an answer.** [`Telemetry::capture`] returns
//! nothing, blocks on nothing and fails at nothing: it offers the event to a
//! bounded queue and returns. A full queue drops, an unreachable collector
//! drops, a non-2xx drops. This is the one place in the codebase where the
//! usual fail-closed instinct is wrong — a migration confirm must not 500
//! because an analytics host is down, and a drop costs one row on a funnel
//! chart.
//!
//! **Unconfigured is the default and is inert.** [`Telemetry::disabled`] holds
//! no queue and spawns no task, which is what every test and every
//! development run gets. The console behaves identically either way.

use tam_types::OrgId;

/// PostHog's batch capture endpoint, relative to the configured host. Batched
/// rather than single-event because the sender already has to own a queue to
/// be non-blocking, and once there is a queue a batch is free.
const CAPTURE_PATH: &str = "/batch/";

/// The EU region's ingestion host, which is the only one this deployment is
/// allowed to name: the compliance floor's Article 3(2) reading applies to
/// every branch of the jurisdiction fork, and residency cannot be changed
/// later without abandoning the history.
pub const DEFAULT_HOST: &str = "https://eu.i.posthog.com";

/// Both hops are one well-known collector, so these are short. Named at all
/// because the lint table refuses a client built without them.
const HTTP_TIMEOUT_SECS: u64 = 10;
const HTTP_CONNECT_TIMEOUT_SECS: u64 = 3;

/// How many events wait for the sender before new ones are dropped.
///
/// Bounded, and deliberately small. The queue exists to decouple a handler
/// from a network round trip, not to buffer an outage: an unreachable
/// collector fills any depth and the only question is how much memory that
/// costs first.
const QUEUE_DEPTH: usize = 256;

/// How many queued events one request carries. PostHog's own limit is far
/// higher; this is sized so a burst drains in a few requests rather than one
/// large one that a timeout would lose whole.
const BATCH_MAX: usize = 32;

/// The PostHog project API key.
///
/// Public by PostHog's own description — it ships in the browser bundle — and
/// still held in a newtype with a redacting `Debug`, because the alternative
/// is a second convention for what counts as a secret. One shape for all of
/// them, as [`crate::billing::WebhookSecret`] does.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectKey(String);

impl ProjectKey {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The one legitimate use: the `api_key` field of a capture body.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for ProjectKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ProjectKey(redacted)")
    }
}

/// One captured event, as it waits in the queue.
struct Captured {
    org: OrgId,
    event: &'static str,
    props: serde_json::Value,
}

/// The capture handle every handler holds, through router state.
///
/// `Clone` is the shared-sender kind: axum clones the state per request and
/// all of those clones feed one queue and one sender task.
#[derive(Clone, Default)]
pub struct Telemetry {
    queue: Option<tokio::sync::mpsc::Sender<Captured>>,
}

impl core::fmt::Debug for Telemetry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(if self.queue.is_some() {
            "Telemetry(configured)"
        } else {
            "Telemetry(disabled)"
        })
    }
}

impl Telemetry {
    /// The inert form: no key was configured, so nothing is queued and nothing
    /// is sent. Every test and every development run takes this branch, and it
    /// is what `Default` yields.
    #[must_use]
    pub fn disabled() -> Self {
        Self { queue: None }
    }

    /// The live form. Spawns the one sender task, so it has to be called from
    /// inside a Tokio runtime — which is where a serving binary's start-up
    /// already is.
    ///
    /// A client that cannot be built is not a start-up failure: the facility
    /// goes inert and says so once, on the same reasoning as every other drop
    /// here.
    #[must_use]
    pub fn new(host: &str, key: ProjectKey) -> Self {
        let Ok(client) = http_client() else {
            eprintln!("tam-api: telemetry disabled: the HTTP client could not be built");
            return Self::disabled();
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(QUEUE_DEPTH);
        let endpoint = format!("{}{CAPTURE_PATH}", host.trim_end_matches('/'));
        crate::blocking::spawn_supervised("telemetry", drain(client, endpoint, key, receiver));
        Self {
            queue: Some(sender),
        }
    }

    /// Record that `event` happened for `org`. Returns immediately, always
    /// succeeds, and is a no-op when no key was configured.
    ///
    /// `event` is `&'static str` so an event name is a literal at its call
    /// site and cannot be assembled from a value a seller supplied.
    pub fn capture(&self, org: OrgId, event: &'static str, props: serde_json::Value) {
        let Some(queue) = self.queue.as_ref() else {
            return;
        };
        // `try_send` rather than an await: the whole contract of this function
        // is that a handler never waits on it. A full queue means the
        // collector is behind, and the event is worth less than the latency.
        if queue.try_send(Captured { org, event, props }).is_err() {
            eprintln!("tam-api: telemetry dropped {event}: the queue is full");
        }
    }
}

/// The sender task: one batch per pass, for as long as the queue is open.
async fn drain(
    client: reqwest::Client,
    endpoint: String,
    key: ProjectKey,
    mut receiver: tokio::sync::mpsc::Receiver<Captured>,
) {
    let mut batch: Vec<Captured> = Vec::with_capacity(BATCH_MAX);
    while receiver.recv_many(&mut batch, BATCH_MAX).await > 0 {
        let body = serde_json::json!({
            "api_key": key.expose(),
            "batch": batch.iter().map(rendered).collect::<Vec<_>>(),
        });
        batch.clear();
        match client.post(&endpoint).json(&body).send().await {
            // No retry budget to spend and nothing downstream waits on the
            // answer, so both arms drop. Neither logs the payload.
            Ok(answer) if answer.status().is_success() => {}
            Ok(answer) => eprintln!("tam-api: telemetry refused: {}", answer.status()),
            Err(error) => eprintln!("tam-api: telemetry unreachable: {error}"),
        }
    }
}

/// One event in PostHog's capture shape. `distinct_id` is a property rather
/// than a sibling field, which is what the batch endpoint reads.
fn rendered(captured: &Captured) -> serde_json::Value {
    let mut properties = if let serde_json::Value::Object(map) = captured.props.clone() {
        map
    } else {
        // A caller that passed something other than an object gets it carried
        // under one key rather than dropped, because losing the event
        // silently is worse than an odd property name on one chart.
        let mut map = serde_json::Map::new();
        drop(map.insert("value".to_owned(), captured.props.clone()));
        map
    };
    drop(properties.insert(
        "distinct_id".to_owned(),
        serde_json::Value::String(captured.org.0.to_hyphenated()),
    ));
    serde_json::json!({
        "event": captured.event,
        "properties": serde_json::Value::Object(properties),
    })
}

fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .https_only(true)
        .timeout(core::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .connect_timeout(core::time::Duration::from_secs(HTTP_CONNECT_TIMEOUT_SECS))
        .build()
}

#[cfg(test)]
mod tests {
    use super::{ProjectKey, Telemetry};
    use tam_types::{OrgId, Uuid};

    #[test]
    fn an_unconfigured_capture_is_a_no_op() {
        // No runtime, no queue, no panic: this is what every deployment
        // without a key does on every handler.
        Telemetry::disabled().capture(
            OrgId(Uuid([0x11; 16])),
            "signup_completed",
            serde_json::json!({ "auth_method": "password" }),
        );
    }

    #[test]
    fn a_key_never_renders_itself() {
        let key = ProjectKey::new("phc_realkeymaterial".to_owned());
        assert_eq!(format!("{key:?}"), "ProjectKey(redacted)");
        assert!(!format!("{key:?}").contains("phc_"));
    }

    #[test]
    fn an_event_carries_the_org_as_its_distinct_id() {
        let rendered = super::rendered(&super::Captured {
            org: OrgId(Uuid([0xAB; 16])),
            event: "import_run_started",
            props: serde_json::json!({ "kind": "marketplace" }),
        });
        assert_eq!(rendered["event"], "import_run_started");
        assert_eq!(
            rendered["properties"]["distinct_id"],
            "abababab-abab-abab-abab-abababababab"
        );
        assert_eq!(rendered["properties"]["kind"], "marketplace");
    }
}
