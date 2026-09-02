//! The wire implementation of [`ControlPlane`]: this device's own HTTP to our
//! registry.
//!
//! This is the only place in the crate that makes a network request, and it
//! reaches exactly one host — ours. Decision D1 puts every marketplace request
//! on this machine under the seller's own session and forbids the server from
//! composing one; the converse obligation lives here, and it is that this
//! client never speaks to a marketplace. Nothing below can: the two paths are
//! literals, the bodies are metadata, and no type reachable from here can
//! carry a cookie jar.
//!
//! The module is split in two so the protocol is testable without a socket.
//! [`Transport`] is one POST reduced to a path, a body and a reply, and
//! [`HttpControlPlane`] is the protocol above it: which path, what JSON, and
//! what each status means. The unit tests substitute a fake at that seam, so
//! the paths, the body shape and the status mapping are pinned without binding
//! a port; [`HttpTransport`] is the one implementation that opens a socket.
//!
//! The server side of this contract is pinned by `crates/tam-api/tests/
//! devices_flow.rs`, which drives the same two paths and the same body keys
//! against the real router over a real database.
//!
//! The session this client speaks under is resolved per call from a
//! [`SessionSource`] rather than held, and is checked before the transport is
//! reached: a device nobody has signed in on makes no request at all, rather
//! than one the server would answer with a blank 401.

use core::future::Future;
use core::pin::Pin;
use core::time::Duration;
use std::sync::Arc;

use serde::Serialize;

use crate::console_session::{SessionSource, SESSION_COOKIE};
use crate::device::{DeviceId, DeviceIdentity};
use crate::heartbeat::{
    CheckIn, ControlPlane, ControlPlaneError, HostFacts, PlaneFuture, SessionReport,
};

/// Where the control plane is, when nothing overrides it.
///
/// The same origin `tauri.conf.json`'s content-security policy already names
/// as the one the console may reach, so the two cannot disagree without one of
/// them being edited.
pub const DEFAULT_BASE_URL: &str = "https://api.teachouse.io";

/// The development override. `just web-dev` serves the console on the vite
/// origin and proxies `/v1` from there to a local `tam-server`, so in
/// development both the session cookie and the control plane live on that one
/// origin.
pub const BASE_URL_ENV: &str = "TAM_CONTROL_PLANE";

/// The control plane's origin: the override if the environment sets one, and
/// the compiled-in default otherwise.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "the desktop client is a configuration-reading process boundary; this is the one \
              site that reads it, which is what the ban asks for"
)]
pub fn base_url() -> String {
    std::env::var(BASE_URL_ENV)
        .ok()
        .map(|raw| raw.trim().to_owned())
        .filter(|raw| !raw.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned())
}

/// How long a check-in may take end to end. Generous, because the seller's
/// connection is a home connection, and bounded, because a check-in that hung
/// would stall the scheduler tick behind it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the connection itself may take. Shorter than the whole request:
/// a machine that cannot reach us at all should fail fast and be retried at
/// the next tick rather than occupy one.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The most of a refusal body that reaches an error message. The server's own
/// structured refusals are short; a proxy's HTML error page is not, and the
/// whole of one in a message helps nobody.
const REFUSAL_EXCERPT: usize = 400;

/// What one POST returned. A status and a body, because the protocol above
/// branches on the status and reads the body only for the two-hundred case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

pub type TransportFuture<'a> = Pin<Box<dyn Future<Output = Result<Reply, String>> + Send + 'a>>;

/// One POST to our control plane, as the protocol above needs it.
///
/// Deliberately narrow. There is no GET, no header argument and no host
/// argument, so the only thing a caller can vary is which of our own paths it
/// posts to and what metadata it sends.
pub trait Transport: Send + Sync {
    /// `session` is the console session cookie value this request speaks
    /// under. A parameter rather than a field, so no implementation can retain
    /// one and none has to be invalidated when the seller signs out.
    fn post<'a>(&'a self, path: &'a str, session: &'a str, body: String) -> TransportFuture<'a>;
}

/// The socket-opening [`Transport`].
///
/// Holds no session: one arrives per request and is dropped with it, so there
/// is no field a credential could be read out of and none to clear on
/// sign-out.
pub struct HttpTransport {
    client: reqwest::Client,
    base: String,
}

impl core::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Non-exhaustive rather than complete: the client is a pool handle
        // nobody reads. There is deliberately nothing else to show, because
        // the only credential this type ever touches is a parameter.
        f.debug_struct("HttpTransport")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl HttpTransport {
    /// Builds the client.
    ///
    /// Both timeouts are set rather than defaulted, which the workspace lint
    /// table requires of every client: `reqwest::Client::new` is banned
    /// precisely because it has neither.
    ///
    pub fn new(base_url: &str) -> Result<Self, ControlPlaneError> {
        install_crypto_provider();
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))?;
        Ok(Self {
            client,
            base: base_url.trim_end_matches('/').to_owned(),
        })
    }
}

/// Installs the process-wide rustls crypto provider, once.
///
/// Not optional and not defensive: with reqwest's `rustls-no-provider`
/// feature, `Client::build()` panics outright when no default provider is
/// installed. The only thing that installs one in this binary today is
/// tauri-plugin-updater, and it does so lazily inside its own update check
/// (2.11.0, `src/updater.rs:492`), so a check-in that ran before the first
/// update check would panic. This is the same guard and the same provider,
/// performed where this client needs it.
///
/// A losing race is success: `install_default` fails only because another
/// component installed one first, which is the outcome this wants.
fn install_crypto_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    }
}

impl Transport for HttpTransport {
    fn post<'a>(&'a self, path: &'a str, session: &'a str, body: String) -> TransportFuture<'a> {
        Box::pin(async move {
            let response = self
                .client
                .post(format!("{}{path}", self.base))
                .header("content-type", "application/json")
                .header("accept", "application/json")
                .header("cookie", format!("{SESSION_COOKIE}={session}"))
                .body(body)
                .send()
                .await
                .map_err(|why| why.to_string())?;
            let status = response.status().as_u16();
            let body = response.text().await.map_err(|why| why.to_string())?;
            Ok(Reply { status, body })
        })
    }
}

/// What `POST /v1/devices` takes. The keys are the server's `RegisterBody`.
#[derive(Debug, Serialize)]
struct RegisterBody<'a> {
    id: &'a str,
    name: &'a str,
    os: &'a str,
    arch: &'a str,
    app_version: &'a str,
}

/// One line of `POST /v1/devices/{device}/heartbeat`. The server's
/// `HeartbeatSession`, and structurally unable to carry a cookie.
#[derive(Debug, Serialize)]
struct SessionLine<'a> {
    marketplace: tam_types::Marketplace,
    account_label: Option<&'a str>,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct HeartbeatBody<'a> {
    sessions: Vec<SessionLine<'a>>,
}

/// The one field this client reads off a heartbeat answer. Deliberately not
/// the whole `HeartbeatView`: the revocation instant is the console's to
/// render, and a client that insisted on every field would break on the next
/// one the server adds.
#[derive(Debug, serde::Deserialize)]
struct HeartbeatReply {
    revoked: bool,
}

/// The registry, over HTTP.
pub struct HttpControlPlane {
    transport: Arc<dyn Transport>,
    sessions: Arc<dyn SessionSource>,
}

impl HttpControlPlane {
    #[must_use]
    pub fn new(transport: Arc<dyn Transport>, sessions: Arc<dyn SessionSource>) -> Self {
        Self {
            transport,
            sessions,
        }
    }

    /// The ordinary construction: a real socket against our own base URL,
    /// speaking under whatever session the source resolves at the time.
    pub fn against(
        base_url: &str,
        sessions: Arc<dyn SessionSource>,
    ) -> Result<Self, ControlPlaneError> {
        Ok(Self::new(Arc::new(HttpTransport::new(base_url)?), sessions))
    }

    /// Resolves the session, then posts.
    ///
    /// The order is the point. A device nobody has signed in on makes no
    /// request at all, rather than one the server answers with the same blank
    /// 401 it gives a forged cookie — which the client could not tell from a
    /// revocation, and which would cost a round trip on every tick of a
    /// machine sitting at a sign-in screen.
    async fn dispatch(&self, path: &str, body: String) -> Result<Reply, ControlPlaneError> {
        let session = self
            .sessions
            .session()
            .await
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))?
            .ok_or(ControlPlaneError::NoSession)?;
        self.transport
            .post(path, &session, body)
            .await
            .map_err(ControlPlaneError::Refused)
    }
}

/// The path a heartbeat posts to. Free so the tests name the same expression
/// the client uses rather than a copy of it.
#[must_use]
pub fn heartbeat_path(device: &DeviceId) -> String {
    format!("/v1/devices/{device}/heartbeat")
}

/// The registration path.
pub const REGISTER_PATH: &str = "/v1/devices";

fn excerpt(body: &str) -> String {
    let taken: String = body.chars().take(REFUSAL_EXCERPT).collect();
    taken.trim().to_owned()
}

/// What a status other than success means.
///
/// Four-hundred-and-four is only ever "this device is not registered" on these
/// two paths, and the caller re-registers rather than retrying; everything else
/// is a refusal carrying what the server said.
fn refusal(reply: &Reply) -> ControlPlaneError {
    match reply.status {
        404 => ControlPlaneError::Unregistered,
        status => ControlPlaneError::Refused(format!("{status}: {}", excerpt(&reply.body))),
    }
}

impl ControlPlane for HttpControlPlane {
    fn register<'a>(&'a self, device: &'a DeviceIdentity, facts: HostFacts) -> PlaneFuture<'a, ()> {
        Box::pin(async move {
            let body = serde_json::to_string(&RegisterBody {
                id: device.id.as_str(),
                name: &device.label,
                os: facts.os,
                arch: facts.arch,
                app_version: facts.app_version,
            })
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))?;

            let reply = self.dispatch(REGISTER_PATH, body).await?;
            if reply.status == 200 {
                Ok(())
            } else {
                Err(refusal(&reply))
            }
        })
    }

    fn heartbeat<'a>(
        &'a self,
        device: &'a DeviceId,
        sessions: &'a [SessionReport],
    ) -> PlaneFuture<'a, CheckIn> {
        Box::pin(async move {
            let body = serde_json::to_string(&HeartbeatBody {
                sessions: sessions
                    .iter()
                    .map(|session| SessionLine {
                        marketplace: session.marketplace,
                        account_label: session.account_label.as_deref(),
                        status: session.status.as_str(),
                    })
                    .collect(),
            })
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))?;

            let reply = self.dispatch(&heartbeat_path(device), body).await?;
            if reply.status != 200 {
                return Err(refusal(&reply));
            }
            // A body we cannot read is a refusal rather than a default. The
            // default would have to be `revoked: false`, which is the answer
            // that keeps working, so guessing it here would turn a broken
            // proxy into a device that never learns it was signed out.
            let parsed: HeartbeatReply = serde_json::from_str(&reply.body).map_err(|why| {
                ControlPlaneError::Refused(format!("unreadable check-in answer: {why}"))
            })?;
            Ok(CheckIn {
                revoked: parsed.revoked,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        base_url, heartbeat_path, HttpControlPlane, HttpTransport, Reply, Transport,
        TransportFuture, DEFAULT_BASE_URL, REGISTER_PATH,
    };
    use crate::console_session::{NoSession, SessionFuture, SessionSource, SessionUnreadable};
    use crate::device::{DeviceId, DeviceIdentity};
    use crate::heartbeat::{
        check_in, first_run, ControlPlane, ControlPlaneError, HostFacts, Offline, SessionReport,
        SessionState,
    };
    use crate::session::memory::MemorySessionStore;
    use crate::session::{Cookie, CookieJar, SessionRecord, SessionStore};
    use crate::state::DesktopState;
    use std::sync::Arc;
    use tam_types::{Marketplace, Timestamp};

    const DEVICE: &str = "11112222333344445555666677778888";

    fn identity() -> DeviceIdentity {
        DeviceIdentity {
            id: DeviceId::from_raw(DEVICE),
            label: "founder-pc".to_owned(),
        }
    }

    /// A transport that answers a canned reply and records what it was asked
    /// to send, so a test can assert on the exact path and body that would go
    /// out over a socket.
    struct Fake {
        reply: Result<Reply, String>,
        seen: tokio::sync::Mutex<Vec<Sent>>,
    }

    #[derive(Clone)]
    struct Sent {
        path: String,
        session: String,
        body: String,
    }

    impl Fake {
        fn answering(status: u16, body: &str) -> Self {
            Self {
                reply: Ok(Reply {
                    status,
                    body: body.to_owned(),
                }),
                seen: tokio::sync::Mutex::new(Vec::new()),
            }
        }

        fn failing(why: &str) -> Self {
            Self {
                reply: Err(why.to_owned()),
                seen: tokio::sync::Mutex::new(Vec::new()),
            }
        }

        async fn last(&self) -> Sent {
            self.seen
                .lock()
                .await
                .last()
                .cloned()
                .expect("the transport was reached")
        }

        async fn calls(&self) -> usize {
            self.seen.lock().await.len()
        }
    }

    impl Transport for Fake {
        fn post<'a>(
            &'a self,
            path: &'a str,
            session: &'a str,
            body: String,
        ) -> TransportFuture<'a> {
            let reply = self.reply.clone();
            Box::pin(async move {
                self.seen.lock().await.push(Sent {
                    path: path.to_owned(),
                    session: session.to_owned(),
                    body,
                });
                reply
            })
        }
    }

    /// A session that is simply there. The ordinary case.
    struct Signed(&'static str);

    impl SessionSource for Signed {
        fn session(&self) -> SessionFuture<'_> {
            Box::pin(async move { Ok(Some(self.0.to_owned())) })
        }
    }

    /// A session that is absent to begin with and appears later, which is what
    /// actually happens: the application starts, and the seller signs in
    /// afterwards in the window.
    #[derive(Default)]
    struct AppearsLater {
        value: tokio::sync::Mutex<Option<String>>,
    }

    impl AppearsLater {
        async fn sign_in(&self, session: &str) {
            *self.value.lock().await = Some(session.to_owned());
        }
    }

    impl SessionSource for AppearsLater {
        fn session(&self) -> SessionFuture<'_> {
            Box::pin(async move { Ok(self.value.lock().await.clone()) })
        }
    }

    /// A cookie store that cannot be opened, which is a fault rather than an
    /// absence.
    struct Unreadable;

    impl SessionSource for Unreadable {
        fn session(&self) -> SessionFuture<'_> {
            Box::pin(async { Err(SessionUnreadable("the window is gone".to_owned())) })
        }
    }

    fn plane(fake: Arc<Fake>) -> HttpControlPlane {
        HttpControlPlane::new(fake, Arc::new(Signed("s3ss10n")))
    }

    fn held(marketplace: Marketplace, label: Option<&str>) -> SessionReport {
        SessionReport {
            marketplace,
            account_label: label.map(str::to_owned),
            status: SessionState::Connected,
        }
    }

    #[tokio::test]
    async fn a_registration_posts_the_host_facts_to_the_registration_path() {
        let fake = Arc::new(Fake::answering(200, "{}"));
        plane(Arc::clone(&fake))
            .register(&identity(), HostFacts::here())
            .await
            .expect("a two-hundred registers");

        let sent_call = fake.last().await;
        let body = sent_call.body.clone();
        assert_eq!(sent_call.path, REGISTER_PATH);
        assert_eq!(
            sent_call.session, "s3ss10n",
            "the request speaks under the console's own session"
        );
        let sent: serde_json::Value = serde_json::from_str(&body).expect("the body is JSON");
        assert_eq!(sent["id"], DEVICE);
        assert_eq!(sent["name"], "founder-pc");
        assert_eq!(
            sent.as_object().map(serde_json::Map::len),
            Some(5),
            "the registration carries exactly the server's five keys and nothing else: {body}"
        );
        for key in ["id", "name", "os", "arch", "app_version"] {
            assert!(sent.get(key).is_some(), "the server reads {key}");
        }
    }

    #[tokio::test]
    async fn a_heartbeat_posts_metadata_only_to_this_devices_own_path() {
        let fake = Arc::new(Fake::answering(
            200,
            r#"{"revoked":false,"revoked_at":null}"#,
        ));
        let answer = plane(Arc::clone(&fake))
            .heartbeat(
                &identity().id,
                &[
                    held(Marketplace::Tpt, Some("Founder's Classroom")),
                    held(Marketplace::Tes, None),
                ],
            )
            .await
            .expect("a two-hundred is a check-in");
        assert!(!answer.revoked);

        let sent_call = fake.last().await;
        let body = sent_call.body.clone();
        assert_eq!(sent_call.path, heartbeat_path(&identity().id));
        assert_eq!(sent_call.path, format!("/v1/devices/{DEVICE}/heartbeat"));
        let sent: serde_json::Value = serde_json::from_str(&body).expect("the body is JSON");
        let lines = sent["sessions"].as_array().expect("sessions is an array");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["marketplace"], "Tpt");
        assert_eq!(lines[0]["account_label"], "Founder's Classroom");
        assert_eq!(lines[0]["status"], "connected");
        assert_eq!(
            lines[1]["account_label"],
            serde_json::Value::Null,
            "no label is null rather than an invented string"
        );
        assert_eq!(
            lines[0].as_object().map(serde_json::Map::len),
            Some(3),
            "a line is three keys, none of which a credential could travel in: {body}"
        );
    }

    /// The property the whole architecture rests on, asserted at the one place
    /// bytes leave this machine for us.
    #[tokio::test]
    async fn nothing_a_captured_session_holds_reaches_the_wire() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&SessionRecord {
                marketplace: Marketplace::Tpt,
                account_label: Some("Founder's Classroom".to_owned()),
                captured_at: Timestamp(1_756_000_000_000),
                device_id: DeviceId::from_raw(DEVICE),
                jar: CookieJar::new(vec![Cookie {
                    name: "sessionKey".to_owned(),
                    value: "s3cr3t".to_owned(),
                }]),
            })
            .await
            .expect("the session stores");

        let fake = Arc::new(Fake::answering(200, r#"{"revoked":false}"#));
        let held: Arc<dyn SessionStore> = store.clone();
        let state =
            DesktopState::with_control_plane(identity(), held, Arc::new(plane(Arc::clone(&fake))));
        check_in(&state, state.control_plane())
            .await
            .expect("the check-in lands");

        let body = fake.last().await.body;
        assert!(
            !body.contains("s3cr3t") && !body.contains("sessionKey"),
            "a cookie reached the control plane, which is the one thing this client \
             must never send: {body}"
        );
    }

    #[tokio::test]
    async fn a_revoked_answer_reaches_the_store_through_the_wire_client() {
        let store = Arc::new(MemorySessionStore::new());
        for marketplace in Marketplace::ALL {
            store
                .put(&SessionRecord {
                    marketplace,
                    account_label: None,
                    captured_at: Timestamp(1_756_000_000_000),
                    device_id: DeviceId::from_raw(DEVICE),
                    jar: CookieJar::default(),
                })
                .await
                .expect("the session stores");
        }
        let fake = Arc::new(Fake::answering(
            200,
            r#"{"revoked":true,"revoked_at":1756000060000}"#,
        ));
        let held: Arc<dyn SessionStore> = store.clone();
        let state = DesktopState::with_control_plane(identity(), held, Arc::new(plane(fake)));

        let answer = check_in(&state, state.control_plane())
            .await
            .expect("the check-in lands");
        assert!(answer.revoked);
        assert!(state.revoked(), "the interface can say why syncing stopped");
        for marketplace in Marketplace::ALL {
            assert_eq!(
                store.get(marketplace).await.expect("the store reads"),
                None,
                "{marketplace:?} must be forgotten on a revoked answer"
            );
        }
    }

    #[tokio::test]
    async fn an_unregistered_device_is_told_to_register_rather_than_to_retry() {
        let refused = plane(Arc::new(Fake::answering(404, "")))
            .heartbeat(&identity().id, &[])
            .await
            .expect_err("a heartbeat is not a registration");
        assert_eq!(refused, ControlPlaneError::Unregistered);
    }

    #[tokio::test]
    async fn every_other_refusal_carries_what_the_server_said() {
        for (status, body) in [
            (401u16, r#"{"status":401,"errors":[]}"#),
            (422, r#"{"errors":[{"message":"unknown session status"}]}"#),
            (503, "upstream unavailable"),
        ] {
            let refused = plane(Arc::new(Fake::answering(status, body)))
                .heartbeat(&identity().id, &[])
                .await
                .expect_err("a non-two-hundred is a refusal");
            let ControlPlaneError::Refused(why) = refused else {
                panic!("{status} should be a refusal, not {refused:?}");
            };
            assert!(
                why.starts_with(&status.to_string()),
                "the refusal names the status: {why}"
            );
        }
    }

    #[tokio::test]
    async fn an_unreadable_answer_is_a_refusal_rather_than_a_guess() {
        let refused = plane(Arc::new(Fake::answering(200, "<html>proxy</html>")))
            .heartbeat(&identity().id, &[])
            .await
            .expect_err("a body that is not our answer cannot be believed");
        let ControlPlaneError::Refused(why) = refused else {
            panic!("expected a refusal, got {refused:?}");
        };
        assert!(
            why.contains("unreadable check-in answer"),
            "guessing `revoked: false` here would be a device that never learns it was \
             signed out: {why}"
        );
    }

    #[tokio::test]
    async fn a_transport_that_could_not_reach_us_is_a_refusal_and_wipes_nothing() {
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&SessionRecord {
                marketplace: Marketplace::Tpt,
                account_label: None,
                captured_at: Timestamp(1_756_000_000_000),
                device_id: DeviceId::from_raw(DEVICE),
                jar: CookieJar::default(),
            })
            .await
            .expect("the session stores");
        let held: Arc<dyn SessionStore> = store.clone();
        let state = DesktopState::with_control_plane(
            identity(),
            held,
            Arc::new(plane(Arc::new(Fake::failing("dns failure")))),
        );

        check_in(&state, state.control_plane())
            .await
            .expect_err("an unreachable server is not a check-in");
        assert!(!state.revoked());
        assert!(
            store
                .get(Marketplace::Tpt)
                .await
                .expect("the store reads")
                .is_some(),
            "an offline period is not a disconnect"
        );
    }

    /// A machine at a sign-in screen. The assertion that matters is the call
    /// count: an unauthenticated request would be answered with the same blank
    /// 401 a forged cookie gets, which this client could not tell from a
    /// revocation, so the request must not be made at all.
    #[tokio::test]
    async fn with_nobody_signed_in_no_request_is_made_and_the_state_says_so() {
        let fake = Arc::new(Fake::answering(200, r#"{"revoked":false}"#));
        let store = Arc::new(MemorySessionStore::new());
        store
            .put(&SessionRecord {
                marketplace: Marketplace::Tpt,
                account_label: None,
                captured_at: Timestamp(1_756_000_000_000),
                device_id: DeviceId::from_raw(DEVICE),
                jar: CookieJar::default(),
            })
            .await
            .expect("the session stores");
        let held: Arc<dyn SessionStore> = store.clone();
        let state = DesktopState::with_control_plane(
            identity(),
            held,
            Arc::new(HttpControlPlane::new(fake.clone(), Arc::new(NoSession))),
        );

        let refused = check_in(&state, state.control_plane())
            .await
            .expect_err("there is no session to speak under");
        assert_eq!(
            refused.to_string(),
            ControlPlaneError::NoSession.to_string()
        );
        assert_eq!(
            fake.calls().await,
            0,
            "the transport must not be reached at all without a session"
        );
        assert!(
            !state.signed_in(),
            "the interface has to be able to say that nobody is signed in here"
        );
        assert!(
            !state.revoked(),
            "not being signed in is not a revocation, and must not read as one"
        );
        assert!(
            store
                .get(Marketplace::Tpt)
                .await
                .expect("the store reads")
                .is_some(),
            "and it wipes nothing: signing out of the console is not signing the device out"
        );
    }

    /// The ordinary start-up order: the application runs before the seller
    /// signs in, so the first check-in finds nothing and a later one succeeds
    /// without anything having to be reconstructed.
    #[tokio::test]
    async fn a_session_that_appears_after_start_is_picked_up_on_the_next_check_in() {
        let fake = Arc::new(Fake::answering(200, r#"{"revoked":false}"#));
        let sessions = Arc::new(AppearsLater::default());
        let store = Arc::new(MemorySessionStore::new());
        let held: Arc<dyn SessionStore> = store.clone();
        let state = DesktopState::with_control_plane(
            identity(),
            held,
            Arc::new(HttpControlPlane::new(fake.clone(), sessions.clone())),
        );

        first_run(&state, state.control_plane())
            .await
            .expect_err("nobody is signed in yet, so even registration cannot speak");
        assert_eq!(fake.calls().await, 0);
        assert!(!state.signed_in());

        sessions.sign_in("later-s3ss10n").await;

        let answer = first_run(&state, state.control_plane())
            .await
            .expect("the session is there now");
        assert!(!answer.revoked);
        assert!(
            state.signed_in(),
            "the state follows the session rather than the start-up order"
        );
        assert_eq!(
            fake.calls().await,
            2,
            "the registration and the check-in both went out once a session existed"
        );
        assert_eq!(
            fake.last().await.session,
            "later-s3ss10n",
            "and they spoke under the session that appeared, not one captured at start"
        );
    }

    #[tokio::test]
    async fn a_cookie_store_that_cannot_be_opened_is_a_refusal_rather_than_a_sign_out() {
        let fake = Arc::new(Fake::answering(200, r#"{"revoked":false}"#));
        let plane = HttpControlPlane::new(fake.clone(), Arc::new(Unreadable));
        let refused = plane
            .heartbeat(&identity().id, &[])
            .await
            .expect_err("a store we cannot read is not a device that is signed out");
        let ControlPlaneError::Refused(why) = refused else {
            panic!("a fault is a refusal, not an absence: {refused:?}");
        };
        assert!(why.contains("the window is gone"), "{why}");
        assert_eq!(fake.calls().await, 0);
    }

    #[test]
    fn the_base_url_is_the_compiled_in_default_unless_the_environment_overrides_it() {
        // The process running the tests sets no override, which is also the
        // shipped condition.
        assert_eq!(
            base_url(),
            DEFAULT_BASE_URL,
            "a build with no override speaks to the origin the content-security policy \
             in tauri.conf.json already names"
        );
        assert!(
            !DEFAULT_BASE_URL.ends_with('/'),
            "the base is joined to paths that begin with a slash"
        );
    }

    /// The seam is still substitutable, which is what keeps a build with no
    /// configured base URL honest rather than silently successful.
    #[tokio::test]
    async fn the_offline_plane_remains_the_not_configured_answer() {
        let refused = Offline
            .heartbeat(&identity().id, &[])
            .await
            .expect_err("Offline reaches nothing");
        assert_eq!(refused, ControlPlaneError::NotConfigured);
        assert_eq!(
            Offline
                .register(&identity(), HostFacts::here())
                .await
                .expect_err("nor does it register"),
            ControlPlaneError::NotConfigured
        );
    }

    /// The regression this exists to prevent is a panic, not a failure.
    ///
    /// reqwest's `rustls-no-provider` feature makes `Client::build()` panic
    /// outright when no default crypto provider is installed, and the only
    /// other thing that installs one in this binary does it lazily inside an
    /// update check. Changing the feature line without the provider call, or
    /// removing the call, brings that panic back on the seller's machine at
    /// the first check-in; it fails here instead.
    #[test]
    fn a_client_builds_without_an_update_check_having_run_first() {
        HttpTransport::new("https://app.example.test")
            .expect("the crypto provider is installed before the client is built");
        // Twice, because `install_default` refuses a second install and the
        // guard has to treat that as success rather than as a fault.
        HttpTransport::new("https://app.example.test")
            .expect("a second client builds against the provider already installed");
    }

    #[tokio::test]
    async fn no_refusal_quotes_the_session_it_spoke_under() {
        let refused = plane(Arc::new(Fake::answering(500, "internal error")))
            .heartbeat(&identity().id, &[])
            .await
            .expect_err("a five-hundred is a refusal");
        let printed = refused.to_string();
        assert!(
            !printed.contains("s3ss10n"),
            "the session is the credential that authenticates this device to us, and a \
             refusal is a string that gets shown: {printed}"
        );
    }

    #[test]
    fn the_base_url_keeps_exactly_one_slash_between_it_and_the_path() {
        // A trailing slash on the configured base plus a leading slash on the
        // path is the ordinary way to get `//v1/devices`, which our router
        // does not mount.
        let transport = HttpTransport::new("https://app.example.test/").expect("the client builds");
        assert!(
            format!("{transport:?}").contains("https://app.example.test\""),
            "the trailing slash is trimmed once, at construction"
        );
    }
}
