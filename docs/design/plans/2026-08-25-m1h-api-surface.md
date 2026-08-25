# M1h: the API surface

Goal: the internal API built properly — nested versioned router, idempotency keys on every sync-starting request, long-running work modelled as operation resources, cursor pagination, progress SSE as a projection of the ledger, and the OpenAPI document — serving exactly what M1i's client renders and M1j's duplication drives.
The design of record is the specification's progress-reporting and API sections (the SSE operational details, the org-scalar cursor, the roll-up rule) and the M1a `tam-api` seam this milestone grows: the versioned router, the closed error vocabulary, and the fail-closed disclosure split.

## Decisions this plan makes

- Authentication is the specification's HttpOnly SameSite cookie session, and M1h builds the session floor without any login flow: self-serve signup is M5's, so sessions are minted by an operator one-shot (`tam-mint-session <db-url> <org> <email>`) that prints the token once, following the `tam-taxonomy-seed` pattern.
  Migration 0014 adds `app_user` and `user_session`; both are classified global in the closed-world matrix because a session row is the authentication root that must be readable before any tenant pin exists — the token digest is the capability, and only its SHA-256 digest is stored.
- `tam-api` depends on `tam-storage` and its integration tests run in a `pg-tests` lane against per-test databases, exactly as the broker's tests already do across the crate boundary; the hermetic lane keeps the extractors, error mapping, pagination codecs and the OpenAPI parity test.
  A port-trait over the whole storage surface would abstract nothing real and is not built.
- The tenant boundary is one extractor: `OrgContext` resolves the session cookie to `(org, user)` or refuses with the structured 401 — except on the stream route, which answers an unauthenticated or expired session with 204, because a failed SSE response permanently stops `EventSource` reconnection and 204 is the specified stop signal.
- A job is the operation resource.
  `POST /v1/jobs` requires an `Idempotency-Key` header (a UUID); the key lands in a new `job.request_idempotency_key` column under `UNIQUE (org_id, request_idempotency_key)` (migration 0014), and a replay returns the original job with 200 where creation returns 201 — the retry and the double-click are the same request, not two jobs.
  Job status in `GET /v1/jobs/{id}` is a computed roll-up over item states that never gates on the first bad item; every item reports independently.
- Cursor pagination is keyset, never offset: products and job listings page on `(created_at, id)` descending, items on `position`, and the cursor is an opaque base64 token the server mints and parses — a client cannot construct page state the server did not issue.
- The SSE stream is one per tab, org-scoped, multiplexing every job: `GET /v1/events/stream` replays from `Last-Event-ID` (the `org_seq` scalar), then polls the ledger; a cursor below the per-organisation pruning watermark (a new `org_event_counter.prune_watermark` column, migration 0014) yields a `resync` event carrying the current snapshot cursor rather than a partial replay.
  Keepalive is axum's fifteen-second comment frame; `X-Accel-Buffering: no` travels on the response; the compression caveat (`NotForContentType::SSE`) is recorded where a future `CompressionLayer` would land.
- Connection revocation on the API path is a thin line-JSON client over the broker's Unix socket, whose path arrives in `Config`; a deployment without the socket answers 503 with the structured body rather than pretending to revoke.
  Only the broker role can tombstone the vault, so the API never touches `connection_secret`.
- The reconciliation queue is served (`GET` open items, resolve with an edge, resolve no-counterpart, drain stats) directly over `TaxonomyRepo`, so M1i's queue page and the founder's drain workflow have an API the day the client lands.
- The OpenAPI document is hand-built as data and served at `/v1/openapi.json`: the route table is one constant array that both mounts the router and generates the document, so the parity test is structural rather than aspirational.
  No new dependency (utoipa or otherwise) without the founder; publishing, metering, per-tier rate limiting and SDK generation stay deferred exactly as the specification defers them.
- The outbox drain loop stays off in M1h: the drain logic is M1d's, the specification hosts the loop in `tam-api.service`, and M1j turns the pumps on together.

## Tasks

### Task 1: sessions and the tenant boundary

Migration `0014_sessions_and_operations.sql`: `app_user` (id, org FK, email, created_at), `user_session` (token digest PK, user FK carrying its org, expires_at, created_at), `job.request_idempotency_key uuid` with its partial-free unique constraint, `org_event_counter.prune_watermark bigint NOT NULL DEFAULT 0`; matrix classification of the two new global tables.
`tam-storage/src/sessions.rs`: `SessionRepo` — `mint` (returns the one-time token, stores the digest), `resolve` (digest lookup, expiry check, returns org and user), `expire`.
`tam-mint-session` one-shot binary.
`tam-api`: `AppState` (pool, config), `OrgContext` extractor (cookie parse, digest, resolve, structured 401), `tam-server` grows a `<db-url>` argument and builds the state.
Tests: pg lane for mint/resolve/expiry/tenant binding; hermetic tests for cookie parsing and the 401 body; the matrix test extended.

### Task 2: jobs as operation resources

Storage read side in `tam-storage/src/jobs.rs` (or a sibling `job_reads.rs`): `create_with_request_key` (enqueue wrapping the new column; replay detection), `list_jobs` keyset page, `job_snapshot` (the roll-up: counts by item state and outcome), `items_page` keyset by position, `item_detail` (the item, its outcome, its step events).
`tam-api` routes: `POST /v1/jobs` (Idempotency-Key mandatory, 422 when absent, 201/200 create/replay), `GET /v1/jobs`, `GET /v1/jobs/{id}`, `GET /v1/jobs/{id}/items`, `GET /v1/jobs/{id}/items/{item}`.
Cursor codec (opaque base64 of the keyset tuple) with unit tests; pg-lane tests: create, replay returns the same job, the roll-up never gates on a bad item, item pages are stable under concurrent settles.

### Task 3: the progress stream

Storage: `events_after(org, seq, limit)`, `latest_seq(org)`, `watermark(org)`.
`tam-api`: `GET /v1/events/stream` — auth-before-stream with the 204 stop signal, `Last-Event-ID` resume, the resync event below the watermark, ledger polling with the keepalive, `X-Accel-Buffering: no`.
Tests: pg-lane streaming test driving the router in-process — replay from a cursor, resync below the watermark, and the 204 on a dead session.

### Task 4: catalogue, connections, reconciliation

Storage: products keyset page (extending the existing summary listing), connections listing.
`tam-api` routes: `GET /v1/products`, `GET /v1/products/{id}`, `GET /v1/connections`, `POST /v1/connections/{id}/revoke` (the broker socket client behind `Config`), `GET /v1/reconciliation/items`, `POST /v1/reconciliation/items/{id}/resolve`, `POST /v1/reconciliation/items/{id}/no-counterpart`, `GET /v1/reconciliation/stats`.
Tests: pg lane for each route's happy path and tenant isolation; the revoke path against an in-process fake broker socket; resolution writing a durable edge visible to a re-projection.

### Task 5: the OpenAPI document

The route-table constant, the generator producing the document as `serde_json` data, `GET /v1/openapi.json`, and the structural parity test (every mounted route documented, every documented route mounted).

### Task 6: close

Full gates (`just check`, `just db-test`, `cargo deny`), the sandboxed flake check on the committed tree, the phase table, and the current-state note.

## Deferred, with owners

- Login, signup, passwords, verified email, signup rate limiting: M5 (the operational charter's signup-abuse controls land with self-serve signup).
- The outbox drain loop, the worker and pipeline pumps: M1j.
- Job-event pruning (the scheduled delete that advances the watermark M1h reads): M1j.
- OpenAPI publishing, metering, per-tier rate limiting, SDK generation: explicitly deferred by the specification until a third-party consumer exists.
- `CompressionLayer` and its SSE predicate: deploy; the caveat is recorded at the stream route.
- HTTP/2 termination and the nginx buffering headers beyond `X-Accel-Buffering`: deploy.
