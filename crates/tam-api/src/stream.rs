//! The progress stream: Server-Sent Events as a projection of the ledger.
//! One stream per tab, org-scoped, multiplexing every job the tenant
//! watches; the event id is the `org_seq` scalar, so `Last-Event-ID` is the
//! resume cursor across all of them. A cursor below the pruning watermark
//! gets a `resync` event carrying the current snapshot cursor rather than a
//! partial replay. A dead session answers 204 — the specified stop signal —
//! because a failed SSE response permanently halts `EventSource`
//! reconnection.
//!
//! Deploy caveat, recorded where it bites: if a `CompressionLayer` ever
//! wraps this router, its predicate must keep `NotForContentType::SSE`, or
//! every stream silently buffers.

use core::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{AppendHeaders, IntoResponse, Response};
use futures_util::stream;
use serde::Deserialize;
use tam_storage::{EventRow, JobReadRepo};
use tam_types::OrgId;

use crate::error::APIError;
use crate::session::StreamAuth;
use crate::AppState;

/// The specification pairs axum's fifteen-second keepalive with nginx's
/// sixty-second read timeout; both halves matter.
const KEEPALIVE: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_secs(1);
const BATCH: i64 = 256;

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    /// The initial cursor for a client without a `Last-Event-ID` yet; the
    /// header wins when both are present, because the header is what the
    /// browser replays on reconnect.
    pub cursor: Option<i64>,
}

fn resume_cursor(headers: &HeaderMap, params: &StreamParams) -> i64 {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse().ok())
        .or(params.cursor)
        .unwrap_or(0)
}

fn wire_event(row: &EventRow) -> Event {
    let data = serde_json::to_string(&row.payload).unwrap_or_else(|_| "{}".to_owned());
    Event::default()
        .id(row.org_seq.to_string())
        .event(row.kind.clone())
        .data(data)
}

struct StreamState {
    reads: JobReadRepo,
    org: OrgId,
    cursor: i64,
    pending: Vec<EventRow>,
    resync_to: Option<i64>,
}

async fn next_event(mut s: StreamState) -> Option<(Result<Event, APIError>, StreamState)> {
    if let Some(latest) = s.resync_to.take() {
        s.cursor = latest;
        let event = Event::default()
            .id(latest.to_string())
            .event("resync")
            .data(format!("{{\"cursor\":{latest}}}"));
        return Some((Ok(event), s));
    }
    loop {
        if let Some(row) = s.pending.pop() {
            s.cursor = row.org_seq;
            return Some((Ok(wire_event(&row)), s));
        }
        match s.reads.events_after(s.org, s.cursor, BATCH).await {
            Ok(mut rows) if !rows.is_empty() => {
                rows.reverse();
                s.pending = rows;
            }
            Ok(_) => tokio::time::sleep(POLL).await,
            // A transient read fault ends this stream; the client's
            // EventSource reconnects with its Last-Event-ID and loses
            // nothing, which is the resumption SSE exists to provide.
            Err(_) => return None,
        }
    }
}

pub(crate) async fn events_stream(
    State(state): State<AppState>,
    auth: StreamAuth,
    headers: HeaderMap,
    Query(params): Query<StreamParams>,
) -> Result<Response, APIError> {
    let context = match auth {
        StreamAuth::Live(context) => context,
        StreamAuth::Stop => return Ok(StatusCode::NO_CONTENT.into_response()),
    };
    let reads = JobReadRepo::new(state.pool.clone());
    let cursor = resume_cursor(&headers, &params);
    let watermark = reads
        .watermark(context.org)
        .await
        .map_err(|error| state.internal(&error.to_string()))?;
    let resync_to = if cursor < watermark {
        let latest = reads
            .latest_seq(context.org)
            .await
            .map_err(|error| state.internal(&error.to_string()))?;
        Some(latest)
    } else {
        None
    };

    let initial = StreamState {
        reads,
        org: context.org,
        cursor,
        pending: Vec::new(),
        resync_to,
    };
    let events = stream::unfold(initial, next_event);
    let sse = Sse::new(events).keep_alive(KeepAlive::new().interval(KEEPALIVE));
    Ok((AppendHeaders([("x-accel-buffering", "no")]), sse).into_response())
}

#[cfg(test)]
mod tests {
    use super::resume_cursor;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn the_header_wins_over_the_query_and_absence_is_zero() {
        let mut headers = HeaderMap::new();
        let params = super::StreamParams { cursor: Some(7) };
        assert_eq!(
            resume_cursor(&headers, &params),
            7,
            "a stored cursor seeds the first connection"
        );
        headers.insert("last-event-id", HeaderValue::from_static("42"));
        assert_eq!(
            resume_cursor(&headers, &params),
            42,
            "the browser's replayed id wins on reconnect"
        );
        assert_eq!(
            resume_cursor(&HeaderMap::new(), &super::StreamParams { cursor: None }),
            0,
            "no cursor means from the beginning"
        );
    }
}
