//! Opening an imported file in a browser, from the seller's own device.
//!
//! Files a seller imports from a marketplace never rest on our servers: they
//! stay on the seller's devices
//! (`docs/notes/design/research/2026-09-29-file-custody.md`). When the seller
//! views, previews or downloads one in the console, this module asks a device
//! that holds it for the bytes and passes them to the browser as they arrive,
//! without writing them anywhere. It is a relay, not a store.
//!
//! The device channel is the device's own outbound HTTPS, because a device
//! sits behind NAT and the server cannot dial it:
//!
//! 1. The browser's request lands on some server process, which writes one
//!    ask per window of at most [`WINDOW_BYTES`] into `device_stream`, with a
//!    capability signed by the entitlement key (`tam_domain::serve::Claims`),
//!    and announces it with `NOTIFY`.
//! 2. The device's long poll (`GET /devices/{d}/streams`), on whichever
//!    process it reached, wakes and hands the ask over.
//! 3. The device checks the capability and posts exactly the bytes asked for
//!    to `POST /devices/{d}/streams/{s}`. The process holding the browser's
//!    request pipes them through a bounded channel of [`FRAMES_BUFFERED`]
//!    frames, so a slow browser slows the device's upload rather than
//!    filling memory. An answer that lands on another process is passed on
//!    to the one named in the ask ([`crate::Config::broker_advertise`]).
//!
//! Range requests are answered from the length the catalogue records, so
//! pdf.js can page through a large PDF by asking for the parts it draws. A
//! device that does not pick an ask up within [`Timeouts::pickup`], or stops
//! sending for [`Timeouts::frame`], is treated as offline; the browser is told
//! which device holds the file and that it should open the app there.
//!
//! What this process keeps in memory is per ask: the channel ends and the
//! capability. What Postgres keeps is the ask's metadata, deleted when the
//! answer ends. Neither ever holds a byte of a file.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::Json;
use futures_util::StreamExt as _;
use jsonwebtoken::{encode, Algorithm, Header};
use serde::Deserialize;
use tam_domain::serve::{
    Claims, Refusal, StreamRequest, StreamRequests, AUDIENCE, CAPABILITY_HEADER, ISSUER,
    REFUSED_HEADER, VALIDITY_SECS,
};
use tam_storage::device_stream::NOTIFY_CHANNEL;
use tam_storage::{DeviceStreamRepo, NewStream, StreamHolder};
use tam_types::{ContentHash, OrgId, Timestamp, Uuid};
use tokio::sync::{mpsc, oneshot, Mutex, Notify};

use crate::error::{APIError, APIErrorCode, APIErrorEntry, APIErrorKind};
use crate::{AppState, OrgContext};

/// The most one ask carries. A device's answer is one request body, and
/// every request reaches us through Cloudflare, which refuses a body over
/// 100 MB; a larger read is several asks one after another.
pub const WINDOW_BYTES: u64 = 32 * 1024 * 1024;

/// The longest a device's poll waits for an ask. Well inside the edge's
/// 100-second idle limit.
pub const POLL_WAIT_MAX_MS: u64 = 25_000;

/// How recently a device must have polled to count as serving. Two polls'
/// worth, so one slow reconnect does not read as a device that went away.
pub const SERVING_WINDOW_MS: i64 = 60_000;

/// Whether a device that last polled at `polled` counts as serving at `now`.
/// The one test behind both the ask routing and the console's online dot, so
/// the dot never says online while View says offline.
#[must_use]
pub fn is_serving(polled: Option<Timestamp>, now: Timestamp) -> bool {
    polled.is_some_and(|at| at.0 >= now.0.saturating_sub(SERVING_WINDOW_MS))
}

/// The most asks one server process holds open at once. A bound rather than
/// a hope: each holds a channel of [`FRAMES_BUFFERED`] frames.
pub const STREAMS_MAX: usize = 64;

/// Frames held between the device's upload and the browser's download. A
/// hyper frame is at most a few tens of kilobytes, so this is well under a
/// megabyte per stream.
pub const FRAMES_BUFFERED: usize = 8;

/// Marks an answer one server process passed to another, so it is never
/// passed on twice.
pub const FORWARDED_HEADER: &str = "x-teachouse-forwarded";

/// How often a waiting poll looks again without being woken, in case a
/// notification was lost between processes.
const RECHECK_MS: u64 = 5_000;

const MILLIS_PER_SEC: i64 = 1_000;

/// What the browser is told when no device answers.
fn offline_sentence(device: &str) -> String {
    format!("Your file is on {device}, which is offline. Open the Teachouse app there.")
}

/// What the browser is told when no device holds the file at all.
pub const NOWHERE: &str = "None of your devices has this file any more.";

/// How long the broker waits, as configuration so a test need not wait as
/// long as production does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// From writing an ask to the device starting its answer. Long enough
    /// for a poll to wake, the device to open its sealed library and its
    /// upload to begin.
    pub pickup: core::time::Duration,
    /// The longest gap between two frames of an answer.
    pub frame: core::time::Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            pickup: core::time::Duration::from_secs(15),
            frame: core::time::Duration::from_secs(30),
        }
    }
}

type Frame = Result<Bytes, std::io::Error>;

/// How a device began its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    Serving,
    Refused(Refusal),
}

/// One ask this process holds the browser's side of.
struct Pending {
    org: OrgId,
    device: String,
    capability: String,
    expected: u64,
    frames: mpsc::Sender<Frame>,
    start: oneshot::Sender<Start>,
}

/// The process's asks and waiting polls. Process-wide rather than per
/// router, because a notification from Postgres names a device and nothing
/// else, and both maps are keyed by values no two tenants share.
#[derive(Default)]
struct Registry {
    pending: Mutex<HashMap<[u8; 16], Pending>>,
    waiters: Mutex<HashMap<(OrgId, String), Arc<Notify>>>,
    /// The databases a listener runs for.
    listening: Mutex<HashSet<String>>,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::default);

/// The client one process passes an answer to another with. Plain HTTP
/// inside the cluster network; generous overall, because a window is up to
/// [`WINDOW_BYTES`] at the device's upload speed.
static FORWARD: LazyLock<Option<reqwest::Client>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(core::time::Duration::from_mins(10))
        .connect_timeout(core::time::Duration::from_secs(5))
        .build()
        .ok()
});

async fn waiter(org: OrgId, device: &str) -> Arc<Notify> {
    Arc::clone(
        REGISTRY
            .waiters
            .lock()
            .await
            .entry((org, device.to_owned()))
            .or_default(),
    )
}

async fn wake(org: OrgId, device: &str) {
    let found = REGISTRY
        .waiters
        .lock()
        .await
        .get(&(org, device.to_owned()))
        .cloned();
    if let Some(notify) = found {
        notify.notify_waiters();
    }
}

/// Drops a poll's waiter once nothing else holds it.
async fn release(org: OrgId, device: &str, notify: Arc<Notify>) {
    let mut waiters = REGISTRY.waiters.lock().await;
    drop(notify);
    let key = (org, device.to_owned());
    if waiters
        .get(&key)
        .is_some_and(|held| Arc::strong_count(held) == 1)
    {
        waiters.remove(&key);
    }
}

async fn pending_len() -> usize {
    REGISTRY.pending.lock().await.len()
}

async fn take_pending(id: Uuid) -> Option<Pending> {
    REGISTRY.pending.lock().await.remove(&id.0)
}

/// Listens for asks written by other processes, once per database, for the
/// life of the pool. A lost notification costs at most one [`RECHECK_MS`].
async fn ensure_listening(pool: &sqlx::PgPool) {
    let options = pool.connect_options();
    let key = format!(
        "{}:{}/{}",
        options.get_host(),
        options.get_port(),
        options.get_database().unwrap_or_default()
    );
    if !REGISTRY.listening.lock().await.insert(key.clone()) {
        return;
    }
    let pool = pool.clone();
    crate::blocking::spawn_supervised("the device stream listener", async move {
        while !pool.is_closed() {
            let listener = match sqlx::postgres::PgListener::connect_with(&pool).await {
                Ok(mut listener) => match listener.listen(NOTIFY_CHANNEL).await {
                    Ok(()) => Some(listener),
                    Err(error) => {
                        eprintln!("tam-api: the device stream listener did not listen: {error}");
                        None
                    }
                },
                Err(error) => {
                    eprintln!("tam-api: the device stream listener did not connect: {error}");
                    None
                }
            };
            if let Some(mut listener) = listener {
                while let Ok(notification) = listener.recv().await {
                    if let Some((org, device)) = notification.payload().split_once(':') {
                        if let Some(org) = Uuid::parse_hyphenated(org) {
                            wake(OrgId(org), device).await;
                        }
                    }
                }
            }
            tokio::time::sleep(core::time::Duration::from_secs(2)).await;
        }
        // The pool closed: a later pool on the same database starts again.
        REGISTRY.listening.lock().await.remove(&key);
    });
}

fn device_offline(device: &str) -> APIError {
    APIError::new(
        StatusCode::CONFLICT,
        APIErrorEntry::new(&offline_sentence(device))
            .code(APIErrorCode::DeviceOffline)
            .kind(APIErrorKind::NotFound)
            .detail(serde_json::json!({ "device": device })),
    )
}

fn nowhere() -> APIError {
    APIError::new(
        StatusCode::NOT_FOUND,
        APIErrorEntry::new(NOWHERE)
            .code(APIErrorCode::ResourceMissing)
            .kind(APIErrorKind::NotFound),
    )
}

fn unavailable() -> APIError {
    APIError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        APIErrorEntry::new(
            "Opening files from your devices isn't available right now. Try again later.",
        )
        .code(APIErrorCode::StreamingUnavailable)
        .kind(APIErrorKind::Internal),
    )
}

fn gone() -> APIError {
    APIError::new(
        StatusCode::GONE,
        APIErrorEntry::new("Nobody is waiting for this file any more.")
            .kind(APIErrorKind::NotFound),
    )
}

fn validation(message: &str) -> APIError {
    APIError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        APIErrorEntry::new(message).kind(APIErrorKind::Validation),
    )
}

fn device_of(raw: &str) -> Result<&str, APIError> {
    if raw.is_empty() || raw.chars().count() > crate::devices::ID_MAX_CHARS {
        return Err(validation("a device id is between 1 and 64 characters"));
    }
    Ok(raw)
}

// ------------------------------------------------------------ choosing a device

/// Which device an ask goes to, or why none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// A holder that polled inside [`SERVING_WINDOW_MS`].
    Serving { device: String, name: String },
    /// Holders exist and none is serving; the one seen most recently.
    Offline { name: String },
    /// No device reports holding the file.
    Nowhere,
}

/// Chooses among a file's holders: the one serving most recently, or the
/// name the seller will recognise as where the file is.
#[must_use]
pub fn choose(holders: &[StreamHolder], now: Timestamp) -> Choice {
    if let Some(holder) = holders
        .iter()
        .filter(|holder| is_serving(holder.stream_polled_at, now))
        .max_by_key(|holder| holder.stream_polled_at.map_or(i64::MIN, |at| at.0))
    {
        return Choice::Serving {
            device: holder.device.clone(),
            name: holder.name.clone(),
        };
    }
    holders
        .iter()
        .max_by_key(|holder| holder.last_seen_at.0)
        .map_or(Choice::Nowhere, |holder| Choice::Offline {
            name: holder.name.clone(),
        })
}

/// The windows one read is cut into: inclusive bounds, each at most
/// [`WINDOW_BYTES`] long.
#[must_use]
pub fn window_end(first: u64, last: u64) -> u64 {
    first.saturating_add(WINDOW_BYTES - 1).min(last)
}

// ------------------------------------------------------------ one window

/// One file being read for one browser request.
#[derive(Debug, Clone)]
struct Ask {
    org: OrgId,
    device: String,
    name: String,
    hash: ContentHash,
}

#[derive(Debug)]
enum WindowFailure {
    /// The device did not start answering in time.
    NoAnswer,
    Refused(Refusal),
    Busy,
    Unavailable,
    Fault(String),
}

impl WindowFailure {
    fn into_error(self, state: &AppState, name: &str) -> APIError {
        match self {
            Self::NoAnswer => device_offline(name),
            Self::Refused(Refusal::Missing) => nowhere(),
            Self::Refused(refusal @ (Refusal::Capability | Refusal::Range)) => {
                state.internal(&format!("a device refused a stream: {}", refusal.word()))
            }
            Self::Busy => APIError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                APIErrorEntry::new("Teachouse is busy opening files right now. Try again shortly.")
                    .code(APIErrorCode::StreamingUnavailable)
                    .kind(APIErrorKind::Internal),
            ),
            Self::Unavailable => unavailable(),
            Self::Fault(why) => state.internal(&why),
        }
    }
}

fn mint(
    state: &AppState,
    ask: &Ask,
    id: Uuid,
    bounds: (u64, u64),
) -> Result<String, WindowFailure> {
    let key = state
        .config
        .entitlement_key
        .as_ref()
        .ok_or(WindowFailure::Unavailable)?;
    let now = (state.wall)();
    let claims = Claims {
        sub: ask.org.0.to_hyphenated(),
        aud: AUDIENCE.to_owned(),
        iss: ISSUER.to_owned(),
        device: ask.device.clone(),
        stream: id.to_hyphenated(),
        hash: tam_secrets::hex_encode(&ask.hash.0),
        first: bounds.0,
        last: bounds.1,
        exp: now
            .0
            .div_euclid(MILLIS_PER_SEC)
            .saturating_add(VALIDITY_SECS),
    };
    encode(&Header::new(Algorithm::EdDSA), &claims, &key.signing_key()).map_err(|error| {
        WindowFailure::Fault(format!("the serve capability did not sign: {error}"))
    })
}

/// Asks the device for bytes `first..=last` and waits for it to start.
async fn open_window(
    state: &AppState,
    ask: &Ask,
    first: u64,
    last: u64,
) -> Result<mpsc::Receiver<Frame>, WindowFailure> {
    if pending_len().await >= STREAMS_MAX {
        return Err(WindowFailure::Busy);
    }
    let id = Uuid(*uuid::Uuid::new_v4().as_bytes());
    let capability = mint(state, ask, id, (first, last))?;
    let (frames, received) = mpsc::channel(FRAMES_BUFFERED);
    let (start, mut started) = oneshot::channel();
    REGISTRY.pending.lock().await.insert(
        id.0,
        Pending {
            org: ask.org,
            device: ask.device.clone(),
            capability: capability.clone(),
            expected: last - first + 1,
            frames,
            start,
        },
    );
    let timeouts = state.config.broker_timeouts;
    let now = (state.wall)();
    let pickup_ms = i64::try_from(timeouts.pickup.as_millis()).unwrap_or(i64::MAX);
    let repo = DeviceStreamRepo::new(state.pool.clone());
    let opened = repo
        .open(
            ask.org,
            &NewStream {
                id,
                device: &ask.device,
                hash: ask.hash,
                first,
                last,
                capability: &capability,
                pod: state.config.broker_advertise.as_deref(),
                created_at: now,
                expires_at: Timestamp(now.0.saturating_add(pickup_ms)),
            },
        )
        .await;
    if let Err(error) = opened {
        take_pending(id).await;
        return Err(WindowFailure::Fault(error.to_string()));
    }
    wake(ask.org, &ask.device).await;
    let outcome = if let Ok(outcome) = tokio::time::timeout(timeouts.pickup, &mut started).await {
        outcome
    } else {
        // Timed out. Still pending means no answer ever began; gone means
        // one began at this very moment and its start is on its way.
        if take_pending(id).await.is_some() {
            close(state, ask.org, id).await;
            return Err(WindowFailure::NoAnswer);
        }
        started.await
    };
    match outcome {
        Ok(Start::Serving) => Ok(received),
        Ok(Start::Refused(refusal)) => Err(WindowFailure::Refused(refusal)),
        Err(_) => Err(WindowFailure::NoAnswer),
    }
}

async fn close(state: &AppState, org: OrgId, id: Uuid) {
    if let Err(error) = DeviceStreamRepo::new(state.pool.clone())
        .close(org, id)
        .await
    {
        eprintln!("tam-api: a device stream row was not closed: {error}");
    }
}

/// The browser's body: the frames of each window in turn, asking for the
/// next when one ends. A window that ends short, a device that falls silent
/// and a next window nobody answers all end the body with an error, which
/// the browser sees as a broken download rather than a wrong file.
struct Flow {
    state: AppState,
    ask: Ask,
    frames: mpsc::Receiver<Frame>,
    /// Bytes still owed by the current window.
    owed: u64,
    /// The first byte of the next window.
    next: u64,
    last: u64,
}

fn broken(why: &str) -> std::io::Error {
    std::io::Error::other(why.to_owned())
}

fn body_of(flow: Flow) -> Body {
    let frames = futures_util::stream::unfold(Some(flow), |flow| async move {
        let mut flow = flow?;
        loop {
            let frame =
                tokio::time::timeout(flow.state.config.broker_timeouts.frame, flow.frames.recv())
                    .await;
            match frame {
                Err(_) => return Some((Err(broken("the device stopped sending")), None)),
                Ok(Some(Ok(bytes))) => {
                    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                    flow.owed = flow.owed.saturating_sub(len);
                    return Some((Ok(bytes), Some(flow)));
                }
                Ok(Some(Err(error))) => return Some((Err(error), None)),
                Ok(None) if flow.owed > 0 => {
                    return Some((Err(broken("the device's answer ended short")), None));
                }
                Ok(None) if flow.next > flow.last => return None,
                Ok(None) => {
                    let end = window_end(flow.next, flow.last);
                    match open_window(&flow.state, &flow.ask, flow.next, end).await {
                        Ok(frames) => {
                            flow.frames = frames;
                            flow.owed = end - flow.next + 1;
                            flow.next = end.saturating_add(1);
                        }
                        Err(failure) => {
                            return Some((
                                Err(broken(&format!(
                                    "the next part did not arrive: {failure:?}"
                                ))),
                                None,
                            ));
                        }
                    }
                }
            }
        }
    });
    Body::from_stream(frames)
}

// ------------------------------------------------------------ the browser's read

/// One imported file a browser asked for.
#[derive(Debug, Clone)]
pub(crate) struct Brokered<'a> {
    pub hash: ContentHash,
    pub byte_len: u64,
    pub served: crate::resources::Served<'a>,
    /// Only whether it could be served now, not the bytes.
    pub probe: bool,
}

/// Answers a browser's read of an imported file from a device that holds it.
pub(crate) async fn serve(
    state: &AppState,
    org: OrgId,
    file: &Brokered<'_>,
    range: Option<&HeaderValue>,
) -> Result<axum::response::Response, APIError> {
    let now = (state.wall)();
    let holders = DeviceStreamRepo::new(state.pool.clone())
        .holders(org, file.hash)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let (device, name) = match choose(&holders, now) {
        Choice::Serving { device, name } => (device, name),
        Choice::Offline { name } => return Err(device_offline(&name)),
        Choice::Nowhere => return Err(nowhere()),
    };
    if state.config.entitlement_key.is_none() {
        return Err(unavailable());
    }
    if file.probe {
        return crate::resources::probe_answer(state);
    }
    let len = file.byte_len;
    let builder = axum::response::Response::builder()
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_TYPE, file.served.content_type)
        .header(
            header::CONTENT_DISPOSITION,
            crate::resources::disposition(file.served.name, file.served.download),
        );
    let ranged = crate::resources::ranged(range.and_then(|value| value.to_str().ok()), len);
    let (status, first, last) = match ranged {
        crate::resources::Ranged::Unsatisfiable => {
            return builder
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{len}"))
                .body(Body::empty())
                .map_err(|error| {
                    state.internal(&format!("the file answer did not build: {error}"))
                });
        }
        crate::resources::Ranged::Whole if len == 0 => {
            return builder
                .status(StatusCode::OK)
                .header(header::CONTENT_LENGTH, 0)
                .body(Body::empty())
                .map_err(|error| {
                    state.internal(&format!("the file answer did not build: {error}"))
                });
        }
        crate::resources::Ranged::Whole => (StatusCode::OK, 0, len - 1),
        crate::resources::Ranged::Part { first, last } => {
            (StatusCode::PARTIAL_CONTENT, first, last)
        }
    };
    ensure_listening(&state.pool).await;
    let ask = Ask {
        org,
        device,
        name,
        hash: file.hash,
    };
    let end = window_end(first, last);
    let frames = open_window(state, &ask, first, end)
        .await
        .map_err(|failure| failure.into_error(state, &ask.name))?;
    let mut builder = builder
        .status(status)
        .header(header::CONTENT_LENGTH, last - first + 1);
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {first}-{last}/{len}"));
    }
    builder
        .body(body_of(Flow {
            state: state.clone(),
            ask,
            frames,
            owed: end - first + 1,
            next: end.saturating_add(1),
            last,
        }))
        .map_err(|error| state.internal(&format!("the file answer did not build: {error}")))
}

// ------------------------------------------------------------ the device's side

#[derive(Debug, Deserialize)]
pub struct PollParams {
    pub wait_ms: Option<u64>,
}

/// A device's long poll: the asks waiting for it, handed over once each,
/// after waiting up to [`POLL_WAIT_MAX_MS`] for one to arrive.
pub(crate) async fn poll_streams(
    State(state): State<AppState>,
    context: OrgContext,
    Path((_version, device)): Path<(String, String)>,
    Query(params): Query<PollParams>,
) -> Result<Json<StreamRequests>, APIError> {
    let device = device_of(&device)?;
    let repo = DeviceStreamRepo::new(state.pool.clone());
    if !repo
        .polled(context.org, device, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?
    {
        return Err(APIError::new(
            StatusCode::NOT_FOUND,
            APIErrorEntry::new("This device isn't signed in to your account.")
                .code(APIErrorCode::ResourceMissing)
                .kind(APIErrorKind::NotFound),
        ));
    }
    ensure_listening(&state.pool).await;
    let wait = core::time::Duration::from_millis(
        params
            .wait_ms
            .unwrap_or(POLL_WAIT_MAX_MS)
            .min(POLL_WAIT_MAX_MS),
    );
    let deadline = tokio::time::Instant::now() + wait;
    let notify = waiter(context.org, device).await;
    let claimed = loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let rows = match repo.claim(context.org, device, (state.wall)()).await {
            Ok(rows) => rows,
            Err(error) => break Err(error),
        };
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if !rows.is_empty() || remaining.is_zero() {
            break Ok(rows);
        }
        let pause = remaining.min(core::time::Duration::from_millis(RECHECK_MS));
        // Woken or not, look again: a timeout is the recheck.
        let _woken = tokio::time::timeout(pause, notified).await;
    };
    release(context.org, device, notify).await;
    let claimed = claimed.map_err(|error| state.internal(&error.to_string()))?;
    Ok(Json(StreamRequests {
        requests: claimed
            .into_iter()
            .map(|row| StreamRequest {
                stream: row.id.to_hyphenated(),
                hash: tam_secrets::hex_encode(&row.hash.0),
                first: row.first,
                last: row.last,
                capability: row.capability,
            })
            .collect(),
    }))
}

/// What a device's answer arrived as.
enum Answer {
    Refused(Refusal),
    Bytes(Body),
}

fn answer_of(headers: &HeaderMap, body: Body) -> Result<Answer, APIError> {
    match headers.get(REFUSED_HEADER) {
        None => Ok(Answer::Bytes(body)),
        Some(word) => word
            .to_str()
            .ok()
            .and_then(Refusal::from_word)
            .map(Answer::Refused)
            .ok_or_else(|| validation("a refusal is missing, capability or range")),
    }
}

/// A device's answer to one ask: the bytes, or why not.
pub(crate) async fn answer_stream(
    State(state): State<AppState>,
    context: OrgContext,
    Path((version, device, stream)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, APIError> {
    let device = device_of(&device)?;
    let id = Uuid::parse_hyphenated(&stream).ok_or_else(|| validation("a stream id is a uuid"))?;
    let capability = headers
        .get(CAPABILITY_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let taken = {
        let mut pending = REGISTRY.pending.lock().await;
        let ours = pending.get(&id.0).is_some_and(|held| {
            held.org == context.org && held.device == device && held.capability == capability
        });
        if ours {
            pending.remove(&id.0)
        } else {
            None
        }
    };
    let Some(pending) = taken else {
        return pass_on(&state, &context, (&version, device, id), headers, body).await;
    };
    let outcome = pipe(pending, answer_of(&headers, body)?).await;
    close(&state, context.org, id).await;
    outcome
}

/// An answer for an ask this process does not hold: passed to the process
/// that does, or refused as gone.
async fn pass_on(
    state: &AppState,
    context: &OrgContext,
    (version, device, id): (&str, &str, Uuid),
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, APIError> {
    if headers.contains_key(FORWARDED_HEADER) {
        return Err(gone());
    }
    let row = DeviceStreamRepo::new(state.pool.clone())
        .find(context.org, id, (state.wall)())
        .await
        .map_err(|error| state.internal(&error.to_string()))?
        .ok_or_else(gone)?;
    let Some(pod) = row
        .pod
        .filter(|pod| row.device == device && state.config.broker_advertise.as_ref() != Some(pod))
    else {
        return Err(gone());
    };
    let client = FORWARD.as_ref().ok_or_else(unavailable)?;
    let mut request = client
        .post(format!(
            "http://{pod}/{version}/devices/{device}/streams/{}",
            id.to_hyphenated()
        ))
        .header(FORWARDED_HEADER, "1");
    for name in [
        header::COOKIE.as_str(),
        header::AUTHORIZATION.as_str(),
        header::CONTENT_TYPE.as_str(),
        CAPABILITY_HEADER,
        REFUSED_HEADER,
    ] {
        if let Some(value) = headers.get(name) {
            request = request.header(name, value.as_bytes());
        }
    }
    let answered = request
        .body(reqwest::Body::wrap_stream(body.into_data_stream()))
        .send()
        .await
        .map_err(|error| state.internal(&format!("an answer was not passed on: {error}")))?;
    let status = StatusCode::from_u16(answered.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    if status.is_success() {
        Ok(status)
    } else if status == StatusCode::GONE {
        Err(gone())
    } else {
        Err(state.internal(&format!("the process holding the ask answered {status}")))
    }
}

/// Passes one answer's bytes to the waiting browser, exactly as many as
/// were asked for.
async fn pipe(pending: Pending, answer: Answer) -> Result<StatusCode, APIError> {
    let Pending {
        expected,
        frames,
        start,
        ..
    } = pending;
    let body = match answer {
        Answer::Refused(refusal) => {
            // The browser may have gone; a refusal nobody reads is still a
            // refusal received.
            start.send(Start::Refused(refusal)).ok();
            return Ok(StatusCode::NO_CONTENT);
        }
        Answer::Bytes(body) => body,
    };
    if start.send(Start::Serving).is_err() {
        return Err(gone());
    }
    let mut data = body.into_data_stream();
    let mut sent: u64 = 0;
    while let Some(chunk) = data.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(
                    frames
                        .send(Err(broken(&format!("the device's upload broke: {error}"))))
                        .await,
                );
                return Err(validation("the upload broke off"));
            }
        };
        sent = sent.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        if sent > expected {
            drop(
                frames
                    .send(Err(broken("the device sent more than it was asked for")))
                    .await,
            );
            return Err(validation("an answer carries exactly the bytes asked for"));
        }
        if frames.send(Ok(chunk)).await.is_err() {
            return Err(gone());
        }
    }
    if sent < expected {
        drop(
            frames
                .send(Err(broken("the device sent less than it was asked for")))
                .await,
        );
        return Err(validation("an answer carries exactly the bytes asked for"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holder(device: &str, seen: i64, polled: Option<i64>) -> StreamHolder {
        StreamHolder {
            device: device.to_owned(),
            name: format!("{device} name"),
            last_seen_at: Timestamp(seen),
            stream_polled_at: polled.map(Timestamp),
        }
    }

    const NOW: Timestamp = Timestamp(10_000_000);

    #[test]
    fn the_most_recently_polling_holder_serves() {
        let holders = [
            holder("laptop", NOW.0 - 1_000, Some(NOW.0 - 50_000)),
            holder("phone", NOW.0 - 90_000, Some(NOW.0 - 5_000)),
        ];
        assert_eq!(
            choose(&holders, NOW),
            Choice::Serving {
                device: "phone".to_owned(),
                name: "phone name".to_owned()
            },
            "the phone polled last"
        );
    }

    #[test]
    fn a_holder_that_stopped_polling_is_offline_under_the_name_last_seen() {
        let holders = [
            holder("laptop", NOW.0 - 1_000, Some(NOW.0 - SERVING_WINDOW_MS - 1)),
            holder("phone", NOW.0 - 90_000, None),
        ];
        assert_eq!(
            choose(&holders, NOW),
            Choice::Offline {
                name: "laptop name".to_owned()
            },
            "a poll outside the window is not serving"
        );
    }

    #[test]
    fn no_holder_is_nowhere() {
        assert_eq!(choose(&[], NOW), Choice::Nowhere, "nobody holds it");
    }

    #[test]
    fn a_read_is_cut_into_windows_no_longer_than_the_bound() {
        assert_eq!(window_end(0, 99), 99, "a small read is one window");
        assert_eq!(
            window_end(0, 3 * WINDOW_BYTES),
            WINDOW_BYTES - 1,
            "a large read's first window is the bound"
        );
        assert_eq!(
            window_end(WINDOW_BYTES, WINDOW_BYTES + 5),
            WINDOW_BYTES + 5,
            "the last window ends at the read's end"
        );
    }

    #[test]
    fn the_offline_sentence_names_the_device() {
        assert_eq!(
            offline_sentence("Kitchen laptop"),
            "Your file is on Kitchen laptop, which is offline. Open the Teachouse app there."
        );
    }
}
