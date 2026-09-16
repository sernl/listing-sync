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
/// them being edited. The main window's origin is derived from this constant
/// rather than configured, so it cannot drift from it and is not a third place
/// to edit; the genuinely independent ones are the two capabilities below.
///
/// One host carries the console, `/v1` and `/api/auth`. Under
/// `teachouse.stowiq.io` that was forced rather than chosen: Cloudflare's free
/// certificate covered one label under `stowiq.io`, so `api.teachouse` would
/// have been a second. At the apex the constraint has lifted — Universal SSL
/// covers `teachouse.io` and `*.teachouse.io` — and the single host is kept
/// because splitting it would move the session cookie, which is a decision of
/// its own rather than a consequence of this cutover.
///
/// Four places carry this origin and all of them must move together:
/// this constant; the `connect-src` in `tauri.conf.json`; the `remote.urls`
/// entry in `capabilities/console.json`; and the `remote.urls` entry in
/// `capabilities/opener.json`.
/// The console capability is the one a cutover would miss and the build would
/// not catch: the window would navigate to the new origin, `Origin::matches`
/// would test it against the old pattern, and every application command would
/// be refused with the console rendering "this page is not one the app accepts
/// commands from" until a new build shipped.
/// Nothing here can catch that for them: the capability is read by Tauri from
/// the file at run time, so the only honest check is the console refusing a
/// command, which `a_marketplace_page_in_the_console_window_reaches_no_command`
/// in `commands.rs` exercises. The opener capability is pinned to the
/// console's by `remote('opener')` equalling `remote('console')` in
/// `web/src/lib/desktop.test.ts`.
pub const DEFAULT_BASE_URL: &str = "https://teachouse.io";

/// The development override. `just web-dev` serves the console on the vite
/// origin and proxies `/v1` from there to a local `tam-server`, so in
/// development both the session cookie and the control plane live on that one
/// origin.
pub const BASE_URL_ENV: &str = "TAM_CONTROL_PLANE";

/// The control plane's origin: the override if the environment sets one, the
/// one baked in at compile time otherwise, and the compiled-in default
/// failing both.
///
/// The compile-time arm exists for a phone. A process on Android has no
/// environment a developer can set, so `cargo tauri android dev` run with
/// `TAM_CONTROL_PLANE` exported bakes the vite origin in and the debug build
/// reaches a local server through `adb reverse`; every release build is made
/// without the variable and carries the default. Runtime still wins, so a
/// desktop developer's shell keeps behaving as before.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "the desktop client is a configuration-reading process boundary; this is the one \
              site that reads it, which is what the ban asks for"
)]
pub fn base_url() -> String {
    std::env::var(BASE_URL_ENV)
        .ok()
        .or_else(|| option_env!("TAM_CONTROL_PLANE").map(str::to_owned))
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

/// What one fetch returned. The body is bytes rather than a string for the
/// reason `tam_marketplace::transport::HttpResponse` gives: `text()` decodes
/// lossily rather than failing, so a payload carried as `String` would have
/// every non-UTF-8 byte silently replaced, and a payload is a seller's own
/// file rather than JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BytesReply {
    pub status: u16,
    pub body: Vec<u8>,
}

pub type TransportFuture<'a> = Pin<Box<dyn Future<Output = Result<Reply, String>> + Send + 'a>>;

pub type BytesFuture<'a> = Pin<Box<dyn Future<Output = Result<BytesReply, String>> + Send + 'a>>;

/// Our control plane, as the protocols above need it.
///
/// Deliberately narrow. Neither method takes a header argument or a host
/// argument, so the only thing a caller can vary is which of our own paths it
/// reaches and what metadata it sends. Two verbs rather than one because the
/// payload fetch is a read of bytes and cannot be a POST of metadata; it is
/// still one host, still one session per call, and still nothing a marketplace
/// would accept as authentication.
pub trait Transport: Send + Sync {
    /// `session` is the console session cookie value this request speaks
    /// under. A parameter rather than a field, so no implementation can retain
    /// one and none has to be invalidated when the seller signs out.
    fn post<'a>(&'a self, path: &'a str, session: &'a str, body: String) -> TransportFuture<'a>;

    /// Reads one of our own paths as bytes, under the same per-call session.
    ///
    /// Two callers now. The interim payload fetch it was written for — D27 puts
    /// file ingest on the seller's device, and until it does the bytes an
    /// upload needs are on our servers and have to come down, see
    /// `docs/notes/design/desktop-data-plane.md` — and the two JSON reads the
    /// import added, the sync-request source and the reachability probe.
    ///
    /// It sends `accept: application/octet-stream`, which is honest for the
    /// payload and wrong-looking for the other two. Harmless because axum's
    /// `Json` responder ignores `Accept`, and recorded here rather than
    /// silently relied on: a server that ever content-negotiated would break
    /// those two reads with nothing to explain why.
    fn fetch<'a>(&'a self, path: &'a str, session: &'a str) -> BytesFuture<'a>;

    /// Deletes one of our own paths, under the same per-call session, with
    /// a JSON body naming what to delete. One caller: cancelling a library
    /// want. Defaulted to a refusal so the test doubles, which never cancel
    /// one, need no arm for it.
    fn delete<'a>(
        &'a self,
        _path: &'a str,
        _session: &'a str,
        _body: String,
    ) -> TransportFuture<'a> {
        Box::pin(async { Err("this transport cannot delete".to_owned()) })
    }
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
        let builder = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT);
        #[cfg(target_os = "android")]
        let builder = builder.tls_backend_preconfigured(bundled_roots());
        let client = builder
            .build()
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))?;
        Ok(Self {
            client,
            base: base_url.trim_end_matches('/').to_owned(),
        })
    }
}

/// The trust anchors the Android build verifies servers against.
///
/// Everywhere else reqwest verifies through `rustls-platform-verifier`, which
/// reads the operating system's own trust store. On Android that verifier
/// requires a Kotlin component in the Gradle build and an initialisation call
/// carrying the JNI environment, and without them its first use panics rather
/// than failing; founder decision O6 takes Mozilla's bundled roots on this one
/// platform instead. `Cargo.toml` records what that costs.
///
/// The downcast this is handed to is the footgun worth naming: reqwest accepts
/// the configuration as `impl Any` and recovers it by downcasting to its own
/// `rustls::ClientConfig` (reqwest 0.13.4, `src/async_impl/client.rs:2192`).
/// If this crate's `rustls` ever resolved to a different version from
/// reqwest's, the downcast would fail silently and the client would fall back
/// to an unconfigured backend rather than refusing to build. Cargo unifies the
/// two into one package today, and the workspace lock holds a single rustls
/// 0.23; a second one appearing is what would break this.
#[cfg(target_os = "android")]
fn bundled_roots() -> rustls::ClientConfig {
    // The provider is installed by the caller immediately above, which is what
    // makes this builder's own lookup of the process default total.
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth()
}

/// Installs the process-wide rustls crypto provider, once.
///
/// Not optional and not defensive: with reqwest's `rustls-no-provider`
/// feature, `Client::build()` panics outright when no default provider is
/// installed (reqwest 0.13.4, `src/async_impl/client.rs:719` falls back to
/// `:2482`, which is a bare `panic!`). Nothing else in this binary installs
/// one first: tauri-plugin-updater does it lazily inside its own update check
/// (2.11.0, `src/updater.rs:492`) and is not compiled for Android at all.
///
/// The footgun is that this crate does not build the first client in the
/// process. On a development build for a phone, Tauri's own dev-server proxy
/// builds one while preparing the webview (2.11.5, `src/protocol/tauri.rs:41`,
/// under `cfg(all(dev, mobile))`), Tauri creates that window before it calls
/// `setup` (`src/app.rs:2525` then `:2530`), and it `unwrap`s the build
/// (`src/protocol/tauri.rs:79`).
/// Tauri's own guard for this cannot fire here, being gated on both an https
/// development url and its `rustls-tls` feature (`src/protocol/tauri.rs:42`).
/// That is why [`run`](crate::run) calls this before `tauri::Builder` and not
/// only from the constructor below.
///
/// A losing race is success: `install_default` fails only because another
/// component installed one first, which is the outcome this wants.
pub(crate) fn install_crypto_provider() {
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

    fn delete<'a>(&'a self, path: &'a str, session: &'a str, body: String) -> TransportFuture<'a> {
        Box::pin(async move {
            let response = self
                .client
                .delete(format!("{}{path}", self.base))
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

    fn fetch<'a>(&'a self, path: &'a str, session: &'a str) -> BytesFuture<'a> {
        Box::pin(async move {
            let response = self
                .client
                .get(format!("{}{path}", self.base))
                .header("accept", "application/octet-stream")
                .header("cookie", format!("{SESSION_COOKIE}={session}"))
                .send()
                .await
                .map_err(|why| why.to_string())?;
            let status = response.status().as_u16();
            let body = response.bytes().await.map_err(|why| why.to_string())?;
            Ok(BytesReply {
                status,
                body: body.to_vec(),
            })
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

/// One file this device holds, as the heartbeat advertises it. A digest and
/// a length: nothing a byte of the file could travel in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HoldingLine {
    pub hash: String,
    pub byte_len: u64,
}

/// Where this device's transfer endpoint listens and what its library
/// holds, sent with every heartbeat once the endpoint is up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryAdvert {
    pub node_id: String,
    pub direct_addrs: Vec<String>,
    pub holdings: Vec<HoldingLine>,
}

#[derive(Debug, Serialize)]
struct HeartbeatBody<'a> {
    sessions: Vec<SessionLine<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    library: Option<LibraryAdvert>,
}

/// The two fields this client reads off a heartbeat answer. Deliberately not
/// the whole `HeartbeatView`: the revocation instant is the console's to
/// render, and a client that insisted on every field would break on the next
/// one the server adds.
///
/// `entitlement` is `default` rather than required, so a server that mints no
/// token — one started without a signing key, or an older one — is answered
/// as granting none rather than as unparseable. That is also why the field
/// could be added at all without stranding the published 0.1.3 client: neither
/// this type nor the server's `HeartbeatView` denies unknown fields, so each
/// end ignores what the other added.
#[derive(Debug, serde::Deserialize)]
struct HeartbeatReply {
    revoked: bool,
    #[serde(default)]
    entitlement: Option<String>,
}

/// The registry, over HTTP.
pub struct HttpControlPlane {
    transport: Arc<dyn Transport>,
    sessions: Arc<dyn SessionSource>,
    /// What the next heartbeat says about this device's library, set by
    /// the transfer loop before each cycle. `None` until the endpoint is
    /// up, and a heartbeat then says nothing about the library, which the
    /// server reads as "unchanged".
    advert: tokio::sync::Mutex<Option<LibraryAdvert>>,
}

impl HttpControlPlane {
    #[must_use]
    pub fn new(transport: Arc<dyn Transport>, sessions: Arc<dyn SessionSource>) -> Self {
        Self {
            transport,
            sessions,
            advert: tokio::sync::Mutex::new(None),
        }
    }

    /// Sets what the next heartbeat advertises about this device's library.
    pub async fn advertise(&self, advert: Option<LibraryAdvert>) {
        *self.advert.lock().await = advert;
    }

    /// The files this device has been asked to fetch, by digest hex.
    pub async fn library_wants(&self, device: &DeviceId) -> Result<Vec<String>, ControlPlaneError> {
        let view = self
            .view(
                &format!("/v1/devices/{device}/library/wants"),
                "this device is not registered",
            )
            .await?;
        serde_json::from_value(view.get("hashes").cloned().unwrap_or_default())
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))
    }

    /// Where the online holders of one file can be reached, and every node
    /// id of the organisation's live devices.
    pub async fn library_peers(
        &self,
        device: &DeviceId,
        hash: &str,
    ) -> Result<LibraryPeers, ControlPlaneError> {
        let view = self
            .view(
                &format!("/v1/devices/{device}/library/peers?hash={hash}"),
                "this device is not registered",
            )
            .await?;
        serde_json::from_value(view).map_err(|why| ControlPlaneError::Refused(why.to_string()))
    }

    /// Cancels a want this device has satisfied or cannot satisfy.
    pub async fn library_unwant(
        &self,
        device: &DeviceId,
        hash: &str,
    ) -> Result<(), ControlPlaneError> {
        let session = self
            .sessions
            .session()
            .await
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))?
            .ok_or(ControlPlaneError::NoSession)?;
        let body = serde_json::json!({ "hash": hash }).to_string();
        let reply = self
            .transport
            .delete(
                &format!("/v1/devices/{device}/library/want"),
                &session,
                body,
            )
            .await
            .map_err(ControlPlaneError::Refused)?;
        if reply.status == 204 || reply.status == 200 {
            Ok(())
        } else {
            Err(refusal(&reply))
        }
    }
}

/// The answer to the peers read.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct LibraryPeers {
    pub peers: Vec<LibraryPeer>,
    pub trusted: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct LibraryPeer {
    pub device: String,
    pub node_id: String,
    pub direct_addrs: Vec<String>,
}

impl HttpControlPlane {
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

    /// Resolves the session, then reads. The same order and the same reason as
    /// [`Self::dispatch`].
    async fn read(&self, path: &str) -> Result<BytesReply, ControlPlaneError> {
        let session = self
            .sessions
            .session()
            .await
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))?
            .ok_or(ControlPlaneError::NoSession)?;
        self.transport
            .fetch(path, &session)
            .await
            .map_err(ControlPlaneError::Refused)
    }

    /// One org-scoped read, as the JSON the server answered with.
    ///
    /// `absent` is the sentence a 404 becomes, and it is a parameter because
    /// the reason differs by route while the shape does not: the server
    /// scopes every one of these to the organisation the session names, so
    /// another tenant's row is missing here rather than forbidden — which is
    /// also what an id the seller mistyped looks like.
    async fn view(&self, path: &str, absent: &str) -> Result<serde_json::Value, ControlPlaneError> {
        let reply = self.read(path).await?;
        match reply.status {
            200 => {}
            404 => return Err(ControlPlaneError::Refused(absent.to_owned())),
            403 => return Err(forbidden(&String::from_utf8_lossy(&reply.body))),
            status => {
                return Err(ControlPlaneError::Refused(format!(
                    "{status}: {}",
                    excerpt(&String::from_utf8_lossy(&reply.body))
                )))
            }
        }
        serde_json::from_slice(&reply.body)
            .map_err(|why| ControlPlaneError::Refused(why.to_string()))
    }
}

impl crate::ledger::LedgerTransport for HttpControlPlane {
    /// One POST to one of our own paths, with the statuses the callers act on
    /// kept apart.
    ///
    /// A conflict is the import protocol's fence answer: another attempt owns
    /// the run, the lease lapsed, or the run is over. The device must stop on
    /// that and must not stop on an outage, so the two cannot share a
    /// variant. An unauthorised or forbidden answer is the console session
    /// this request spoke under being refused, which no number of retries
    /// changes, so it is kept apart from an outage for the opposite reason:
    /// what is owed is dropped rather than offered again forever.
    ///
    /// A bad request, a body too large and an unprocessable body are the
    /// third class: the server read this payload and will never accept it.
    /// Left with the outages it would be offered for as long as the device
    /// runs, and each offer of an import page re-reads the seller's shop and
    /// supersedes the run's fence to build it. Everything else that is not a
    /// success stays an outage — a timeout, a rate limit, a gateway — because
    /// those resolve themselves and dropping what is owed would lose the only
    /// copy of work the server never recorded.
    fn post<'a>(&'a self, path: &'a str, body: String) -> PlaneFuture<'a, String> {
        Box::pin(async move {
            let reply = self.dispatch(path, body).await?;
            match reply.status {
                // Two-hundred-and-four included: the abandonment route
                // answers no content, and a success read as a refusal would
                // have the device replay a stop the server had accepted
                // forever.
                200 | 202 | 204 => Ok(reply.body),
                401 | 403 => Err(forbidden(&reply.body)),
                404 => Err(ControlPlaneError::Unregistered),
                400 | 413 | 422 => Err(ControlPlaneError::Rejected(format!(
                    "{}: {}",
                    reply.status,
                    excerpt(&reply.body)
                ))),
                // The whole body rather than an excerpt: the caller reads the
                // structured `code` off it to tell a settled run from a
                // fenced one, and a truncated body is one it cannot parse.
                409 | 410 => Err(ControlPlaneError::Fenced(reply.body.clone())),
                status => Err(ControlPlaneError::Refused(format!(
                    "{status}: {}",
                    excerpt(&reply.body)
                ))),
            }
        })
    }
}

impl crate::payload::PayloadTransport for HttpControlPlane {
    fn fetch<'a>(&'a self, path: &'a str) -> PlaneFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let reply = self.read(path).await?;
            match reply.status {
                200 => Ok(reply.body),
                403 => Err(forbidden(&String::from_utf8_lossy(&reply.body))),
                404 => Err(ControlPlaneError::Unregistered),
                status => Err(ControlPlaneError::Refused(format!(
                    "{status}: {}",
                    excerpt(&String::from_utf8_lossy(&reply.body))
                ))),
            }
        })
    }
}

/// The path a heartbeat posts to. Free so the tests name the same expression
/// the client uses rather than a copy of it.
#[must_use]
pub fn heartbeat_path(device: &DeviceId) -> String {
    format!("/v1/devices/{device}/heartbeat")
}

/// The path the consent record is read from.
#[must_use]
pub fn consents_path() -> String {
    "/v1/consents".to_owned()
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
/// is a refusal carrying what the server said, except the one forbidden answer
/// that is about this device rather than about the request.
fn refusal(reply: &Reply) -> ControlPlaneError {
    match reply.status {
        404 => ControlPlaneError::Unregistered,
        403 => forbidden(&reply.body),
        status => ControlPlaneError::Refused(format!("{status}: {}", excerpt(&reply.body))),
    }
}

/// Which forbidden answer this is: the seller having signed this device out,
/// or the console session being refused for any other reason.
///
/// Read off the body because the server states it there and nowhere else: the
/// registry's own refusal is `admissible_device` in `tam-api`'s import routes,
/// and the phrase is matched rather than a code because the error body carries
/// no machine-readable one. Narrow on purpose — an ordinary forbidden answer
/// must stay [`ControlPlaneError::Denied`], because telling a seller their
/// machine was signed out whenever a request was refused would be a false
/// accusation with a remedy attached.
fn forbidden(body: &str) -> ControlPlaneError {
    let said = body.to_lowercase();
    if said.contains("device is revoked") || said.contains("device revoked") {
        ControlPlaneError::Revoked
    } else {
        ControlPlaneError::Denied(excerpt(body))
    }
}

/// The path the device reads a sync request from. A free function so the wire
/// test names the same expression the client uses rather than a copy of it.
#[must_use]
pub fn sync_request_path(request: tam_types::Uuid) -> String {
    format!(
        "/v1/sync/{}",
        uuid::Uuid::from_bytes(request.0).as_hyphenated()
    )
}

/// One count off a run's execution block.
///
/// Required, and a value that is not a `u32` is a refusal rather than a zero.
/// The protocol states these counts on every run; a client that filled a
/// missing one in would resume a takeover from a number nobody sent.
fn counted(execution: &serde_json::Value, field: &str) -> Result<u32, ControlPlaneError> {
    execution
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or_else(|| {
            ControlPlaneError::Refused(format!("the import states no {field} this client can read"))
        })
}

/// The path the device reads an import run from.
#[must_use]
pub fn import_run_path(run: tam_types::Uuid) -> String {
    format!(
        "/v1/imports/runs/{}",
        uuid::Uuid::from_bytes(run.0).as_hyphenated()
    )
}

impl ControlPlane for HttpControlPlane {
    fn reachable(&self) -> PlaneFuture<'_, ()> {
        Box::pin(async move {
            // No session: this asks whether the server is there, and a seller
            // who is not signed in must still get a true answer rather than a
            // browser error page. `/healthz` carries no version segment, so a
            // client one version behind still reaches it.
            let reply = self
                .transport
                .fetch("/healthz", "")
                .await
                .map_err(ControlPlaneError::Refused)?;
            if reply.status == 200 {
                Ok(())
            } else {
                Err(ControlPlaneError::Refused(format!(
                    "the server answered {} rather than 200",
                    reply.status
                )))
            }
        })
    }

    fn consent_stands(&self, marketplace: tam_types::Marketplace) -> PlaneFuture<'_, bool> {
        Box::pin(async move {
            let view = self
                .view(&consents_path(), "this sign-in has no consent record")
                .await?;
            let rows = view
                .get("consents")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| {
                    ControlPlaneError::Refused("the consent record lists no rows".to_owned())
                })?;
            // Deserialised from the value the server sent, as the other reads
            // do, so the marketplace spelling is the wire's and not a copy.
            Ok(rows.iter().any(|row| {
                row.get("standing").and_then(serde_json::Value::as_bool) == Some(true)
                    && row.get("marketplace").cloned().and_then(|value| {
                        serde_json::from_value::<tam_types::Marketplace>(value).ok()
                    }) == Some(marketplace)
            }))
        })
    }

    fn sync_request_source(
        &self,
        request: tam_types::Uuid,
    ) -> PlaneFuture<'_, tam_types::InventoryId> {
        Box::pin(async move {
            let reply = self.read(&sync_request_path(request)).await?;
            match reply.status {
                200 => {}
                // The server scopes this read to the organisation the session
                // names, so a request belonging to another tenant is absent
                // rather than forbidden — and absent is also what a request id
                // the seller mistyped looks like.
                404 => {
                    return Err(ControlPlaneError::Refused(
                        "this sign-in has no such migration".to_owned(),
                    ))
                }
                status => {
                    return Err(ControlPlaneError::Refused(format!(
                        "{status}: {}",
                        excerpt(&String::from_utf8_lossy(&reply.body))
                    )))
                }
            }
            let view: serde_json::Value = serde_json::from_slice(&reply.body)
                .map_err(|why| ControlPlaneError::Refused(why.to_string()))?;
            let source = view.get("source").cloned().ok_or_else(|| {
                ControlPlaneError::Refused("the migration names no source inventory".to_owned())
            })?;
            // Deserialised from the value the server sent rather than from a
            // string rebuilt out of it. Rebuilding assumes the field is always
            // a bare string, so a changed wire shape would be reported as "no
            // source inventory" rather than as what it actually was.
            serde_json::from_value(source)
                .map_err(|why| ControlPlaneError::Refused(why.to_string()))
        })
    }

    fn import_run_facts(&self, run: tam_types::Uuid) -> PlaneFuture<'_, crate::import::RunFacts> {
        Box::pin(async move {
            let view = self
                .view(&import_run_path(run), "this sign-in has no such import")
                .await?;
            let source = view.get("source").cloned().ok_or_else(|| {
                ControlPlaneError::Refused("the import names no source inventory".to_owned())
            })?;
            let source = serde_json::from_value(source)
                .map_err(|why| ControlPlaneError::Refused(why.to_string()))?;
            // Required, every one of them. These are what a device taking a
            // run over resumes from, so a view this client cannot read is a
            // view it must refuse: inferring zero would report work the run
            // has already done as undone, and would do it silently, which is
            // the class of defect this whole repair is about.
            let execution = view.get("execution").ok_or_else(|| {
                ControlPlaneError::Refused("the import states no execution".to_owned())
            })?;
            Ok(crate::import::RunFacts {
                source,
                discovered: counted(execution, "discovered")?,
                processed: counted(execution, "processed")?,
                described: counted(execution, "described")?,
                enumeration_complete: execution
                    .get("enumeration_complete")
                    .and_then(serde_json::Value::as_bool)
                    .ok_or_else(|| {
                        ControlPlaneError::Refused(
                            "the import does not say whether its discovery is complete".to_owned(),
                        )
                    })?,
            })
        })
    }

    fn import_selection<'a>(
        &'a self,
        device: &'a DeviceId,
        run: tam_types::Uuid,
    ) -> PlaneFuture<'a, Vec<String>> {
        Box::pin(async move {
            let view = self
                .view(
                    &crate::import::selection_path(device, run),
                    "this sign-in has no such import",
                )
                .await?;
            let locators = view.get("locators").cloned().ok_or_else(|| {
                ControlPlaneError::Refused("the import names no selection".to_owned())
            })?;
            // An empty selection is a value rather than an error: the seller
            // may have unticked everything, and the run settles with every
            // item skipped rather than the device refusing to continue.
            serde_json::from_value(locators)
                .map_err(|why| ControlPlaneError::Refused(why.to_string()))
        })
    }

    fn open_import_runs<'a>(
        &'a self,
        device: &'a DeviceId,
    ) -> PlaneFuture<'a, Vec<crate::import::OpenImportRun>> {
        Box::pin(async move {
            let reply = self.read(&crate::import::open_import_path(device)).await?;
            match reply.status {
                // Three spellings of "nothing to do", and none is a fault: an
                // empty array, a `null` body from a server that has no run
                // open, and a not-found from one a version behind that does
                // not serve this route at all. A device asks this at every
                // check-in, so an ordinary answer that travelled as an error
                // would be an hourly failure in the seller's activity saying
                // nothing.
                200 => {
                    serde_json::from_slice::<Option<Vec<crate::import::OpenImportRun>>>(&reply.body)
                        .map(Option::unwrap_or_default)
                        .map_err(|why| ControlPlaneError::Refused(why.to_string()))
                }
                404 => Ok(Vec::new()),
                status => Err(ControlPlaneError::Refused(format!(
                    "{status}: {}",
                    excerpt(&String::from_utf8_lossy(&reply.body))
                ))),
            }
        })
    }

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
                library: self.advert.lock().await.clone(),
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
                entitlement: parsed.entitlement,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        base_url, heartbeat_path, install_crypto_provider, BytesFuture, BytesReply,
        HttpControlPlane, HttpTransport, Reply, Transport, TransportFuture, DEFAULT_BASE_URL,
        REGISTER_PATH,
    };
    use crate::console_session::{NoSession, SessionFuture, SessionSource, SessionUnreadable};
    use crate::device::{DeviceId, DeviceIdentity};
    use crate::heartbeat::{
        check_in, first_run, ControlPlane, ControlPlaneError, HostFacts, Offline, SessionReport,
        SessionState,
    };
    use crate::import::{
        ClaimedRun, ImportJournal, ImportPage, MemoryJournal, PendingPost, RunLedger, RunPhase,
        RunProgress, StopSignal,
    };
    use crate::ledger::LedgerTransport;
    use crate::session::memory::MemorySessionStore;
    use crate::session::{Cookie, CookieJar, SessionRecord, SessionStore};
    use crate::state::DesktopState;
    use std::sync::Arc;
    use tam_types::{Marketplace, Timestamp};

    const DEVICE: &str = "11112222333344445555666677778888";

    /// One run, for the ledger cases below.
    const RUN: tam_types::Uuid = tam_types::Uuid([0x2a; 16]);

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

        fn fetch<'a>(&'a self, path: &'a str, session: &'a str) -> BytesFuture<'a> {
            let reply = self.reply.clone();
            Box::pin(async move {
                self.seen.lock().await.push(Sent {
                    path: path.to_owned(),
                    session: session.to_owned(),
                    body: String::new(),
                });
                reply.map(|answer| BytesReply {
                    status: answer.status,
                    body: answer.body.into_bytes(),
                })
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
    async fn a_minted_token_is_carried_off_the_wire_unread() {
        let fake = Arc::new(Fake::answering(
            200,
            r#"{"revoked":false,"revoked_at":null,"entitlement":"a.b.c"}"#,
        ));
        let answer = plane(Arc::clone(&fake))
            .heartbeat(&identity().id, &[])
            .await
            .expect("a two-hundred is a check-in");
        assert_eq!(
            answer.entitlement.as_deref(),
            Some("a.b.c"),
            "the transport carries the token as a string; nothing here verifies it, because \
             the key belongs to the check-in and not to the socket"
        );
    }

    #[tokio::test]
    async fn an_answer_from_a_server_that_mints_nothing_still_parses() {
        let fake = Arc::new(Fake::answering(
            200,
            r#"{"revoked":false,"revoked_at":null}"#,
        ));
        let answer = plane(Arc::clone(&fake))
            .heartbeat(&identity().id, &[])
            .await
            .expect("a reply without the field is not a malformed reply");
        assert_eq!(
            answer.entitlement, None,
            "a deployment with no signing key, and every server older than the mint, answers \
             exactly what this client already accepted"
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

    /// The registry's own forbidden answer becomes the typed revocation, and
    /// every other forbidden answer does not.
    ///
    /// Both halves in one test, because the failure mode is a classifier that
    /// is right about one and wrong about the other, and either mistake is
    /// bad: a revocation read as an ordinary refusal puts the server's JSON in
    /// front of a teacher, which is the reported Android defect, and an
    /// ordinary refusal read as a revocation tells a seller their machine was
    /// signed out when it was not — and offers them a restore for a machine
    /// that never lost its standing.
    #[tokio::test]
    async fn a_revoked_answer_is_typed_and_other_forbidden_answers_are_not() {
        let revoked = r#"{"errors":[{"message":"this device is revoked and may not report a catalogue","kind":"validation"}]}"#;
        let refused = plane(Arc::new(Fake::answering(403, revoked)))
            .heartbeat(&identity().id, &[])
            .await
            .expect_err("a forbidden answer is not a check-in");
        assert_eq!(
            refused,
            ControlPlaneError::Revoked,
            "the registry's phrase is the one fact a seller has a remedy for"
        );
        // And the sentence a seller reads carries no JSON, no status and no id.
        let said = refused.to_string();
        assert!(
            !said.contains('{') && !said.contains("403"),
            "a revoked refusal reads as prose, not as a response body: {said}"
        );

        for body in [
            r#"{"errors":[{"message":"this device holds no live lease on an item whose projection names that file"}]}"#,
            r#"{"errors":[{"message":"your plan does not include that marketplace"}]}"#,
            "Forbidden",
        ] {
            let refused = plane(Arc::new(Fake::answering(403, body)))
                .heartbeat(&identity().id, &[])
                .await
                .expect_err("a forbidden answer is not a check-in");
            assert_ne!(
                refused,
                ControlPlaneError::Revoked,
                "an ordinary forbidden answer must not be read as a sign-out: {body}"
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

    /// The source inventory comes off the wire, from the path the request
    /// names.
    ///
    /// The device asks by request id and reads the shop out of the answer,
    /// which is what stops a console asking it to enumerate a shop the request
    /// does not name.
    #[tokio::test]
    async fn the_source_inventory_is_read_from_the_request() {
        let fake = Arc::new(Fake::answering(
            200,
            r#"{"request":"71717171-7171-7171-7171-717171717171","source":"Tes","target":"Tpt","disposition":"migrate","intent":"draft","state":"pending","resources":[]}"#,
        ));
        let source = plane(Arc::clone(&fake))
            .sync_request_source(tam_types::Uuid([0x71; 16]))
            .await
            .expect("the request reads");
        assert_eq!(
            source,
            tam_types::InventoryId::Tes,
            "the shop is the one the request names, not a default this device picked"
        );
        let path = fake.seen.lock().await[0].path.clone();
        assert_eq!(
            path,
            super::sync_request_path(tam_types::Uuid([0x71; 16])),
            "and it is read from the request's own path"
        );
    }

    /// Another organisation's request is absent, and the device says so in
    /// words a seller can act on.
    #[tokio::test]
    async fn a_request_this_sign_in_cannot_see_is_a_refusal_rather_than_a_fault() {
        let refused = plane(Arc::new(Fake::answering(404, "{}")))
            .sync_request_source(tam_types::Uuid([0x71; 16]))
            .await
            .expect_err("a 404 is a refusal");
        assert!(
            refused.to_string().contains("no such migration"),
            "the server scopes this read to the session's organisation, so absent is what \
             another tenant's request looks like: {refused}"
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

    /// Beside the test above, not instead of it: that one covers the transport
    /// constructor, and this one covers the call `run` makes before any
    /// transport exists, which is what keeps Tauri's own client from panicking.
    #[test]
    fn the_installed_provider_is_the_one_reqwest_looks_for() {
        install_crypto_provider();
        assert!(
            rustls::crypto::CryptoProvider::get_default().is_some(),
            "reqwest reads the process default; with none, building a client is a panic \
             rather than an error"
        );
        // Timeout-less and otherwise unconfigured on purpose: this is the shape
        // Tauri's own dev-server proxy builds (2.11.5,
        // `src/protocol/tauri.rs:40`), and it is that client, not one of ours,
        // that aborted the Android start-up. Built and dropped; it issues no
        // request. Passing also pins the version coupling `bundled_roots`
        // names: our `rustls` and reqwest's have to be one package for this
        // install to be the one reqwest finds.
        drop(
            reqwest::Client::builder()
                .build()
                .expect("the client builds"),
        );
    }

    /// One page offered to a server answering `status`, and what this device
    /// still owes for the run once the answer has been read.
    ///
    /// The real transport and the real outbox, because the classes only mean
    /// what they do: a class the outbox reads as an outage leaves the page
    /// queued and offers it again under every later fence, and each offer of
    /// an import page re-reads the seller's shop and supersedes the run's
    /// fence to build it.
    async fn still_owed(status: u16, body: &str) -> Vec<PendingPost> {
        let transport: Arc<dyn LedgerTransport> =
            Arc::new(plane(Arc::new(Fake::answering(status, body))));
        let journal = Arc::new(MemoryJournal::default());
        let kept: Arc<dyn ImportJournal> = Arc::<MemoryJournal>::clone(&journal);
        let ledger = RunLedger::new(
            identity().id,
            &ClaimedRun {
                run: RUN,
                source: tam_types::InventoryId::Tes,
                attempt: 1,
                lease_expires_at: 0,
                phase: RunPhase::Describe,
                stop: StopSignal::never(),
                progress: RunProgress::default(),
            },
            transport,
            kept,
        );
        ledger
            .post_page(a_page(), &RunProgress::default())
            .await
            .expect_err("a page the server did not accept is not a delivered page");
        journal
            .read()
            .await
            .expect("this device's journal reads")
            .owed(RUN)
    }

    /// One page, with nothing in it but the shape: what it carries is not
    /// what these statuses are about.
    fn a_page() -> ImportPage {
        ImportPage {
            run: RUN,
            attempt: None,
            receipt: None,
            request: None,
            listed: None,
            resources: Vec::new(),
            skipped: Vec::new(),
            enumeration_complete: false,
            complete: false,
            failed: None,
        }
    }

    /// An outage is offered again and a refusal of the payload is not.
    ///
    /// Before this the classifier sent everything that was not a success, a
    /// forbidden answer, a four-hundred-and-four or a conflict down one
    /// branch, so a page the server will never accept — too large, or
    /// carrying a field this version's route refuses — was queued and offered
    /// for as long as the device ran. A permanent refusal is a fact about the
    /// page rather than about the connection, so the device has to be able to
    /// settle it and report it rather than retry it forever.
    #[tokio::test]
    async fn a_permanent_refusal_is_not_offered_again_as_though_it_were_an_outage() {
        for status in [400_u16, 413, 422] {
            let owed = still_owed(status, r#"{"errors":[]}"#).await;
            assert!(
                owed.is_empty(),
                "{status} is a refusal of the page itself: offering it again cannot change the \
                 answer, and the cycle that offers it re-reads the seller's shop and supersedes \
                 the run's fence to do so — {owed:?} is still queued"
            );
        }

        for status in [408_u16, 429, 500, 502, 503, 504] {
            let owed = still_owed(status, "upstream unavailable").await;
            assert_eq!(
                owed.len(),
                1,
                "and {status} stays an outage, which resolves itself: dropping the page then \
                 would lose the only copy of work the server never recorded"
            );
        }
    }
}
