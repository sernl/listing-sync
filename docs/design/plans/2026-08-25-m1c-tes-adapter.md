# M1c Tes adapter implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Promote the M0 spike into a production `tam-marketplace-tes` crate: the confirmed endpoints as a typed client behind the adapter seam, three-valued outcome classification, the fetch-reason read capability, delete with positive verification, a schema-fingerprint pre-flight, and a cassette harness so every flow is tested offline — then delete the spike, per the decision record.

**Architecture:** A sans-io transport seam and the cassette harness land in `tam-marketplace` (the design's crate table places the harness there; the purity ban still holds because both are serde-only). `tam-marketplace-tes` is a `reqwest` client against Tes's internal JSON API — no browser anywhere in its tree — generic over the transport so cassettes stand where the network stands. Sessions are constructor state on the live transport, never request data, so cassettes cannot contain secrets by construction.

**Tech Stack:** reqwest 0.12 (rustls, builder-only per the ban list), serde/serde_json, sha2 for the schema fingerprint, the cassette harness for tests.

## Global constraints

- The M0-confirmed endpoints, from `docs/design/decisions.md`: create `POST /api/v2/resources`; metadata `POST /api/v2/resources/{id}/draft`; presign and confirm `POST /api/resources/v3/draft/{id}/attachment`; the S3 form POST to the policy's bucket; publish `POST /api/v2/resources/{id}/publish`, which requires a valid `licence` on the draft first (free resources use `CC-BY`, `CC-BY-SA` or `CC-BY-ND`; `TES-PAID` requires a price); read `GET /api/v2/resources/{id}/draft` then `/{id}`; delete `DELETE /api/v2/resources/{id}`.
- The misleading-204 rule: `DELETE .../{id}/draft` removes only the draft overlay, the public URL soft-404s, and delete reports success only after the API `GET` returns 404. The mutating call's status is never the verdict.
- Positive-assertion classification everywhere: `Committed` requires the response to be JSON carrying the expected id; interstitials, truncations, 401/403/429 and transients are `Ambiguous`, never retried.
- Reads are constructed through `FetchReason` only; the adapter offers no other read path.
- `tam-marketplace` stays inside the purity lane (no tokio/reqwest/sqlx); `WriteReceipt` remains serde-free.
- Cassettes are sanitized hand-authored fixtures of the M0-observed response shapes; tests run offline, in `just check` and the nix sandbox.
- Tes GB and US are one crate: two adapters sharing the flow, differing in data (vocabulary, currency) — per the design's crate table.
- The spike `spikes/tes-spike/` is deleted in the final task (decision record line 125); a guarded live-smoke example replaces its operator role.
- Commit recipe (jj, signed): `jj describe -m "<msg>"`, `jj bookmark set main -r @`, `jj new`, `git push origin main`.

---

### Task 1: transport seam and cassette harness in tam-marketplace

**Files:**
- Create: `crates/tam-marketplace/src/transport.rs`, `crates/tam-marketplace/src/cassette.rs`
- Modify: `crates/tam-marketplace/src/lib.rs`, `crates/tam-marketplace/Cargo.toml` (serde, serde_json)

**Interfaces:**
- Produces: `Method`, `RequestBody` (Empty | Json | Multipart with an optional file part), `HttpRequest` (method, url, body — deliberately NO headers: auth is live-transport constructor state, so a cassette cannot record a secret), `HttpResponse` (status, body), `TransportError` (`NotSent(ConnectFailure)` — the only retry-safe class; `AfterSend` — sent, response lost, ambiguous; `Harness` — produced only by the cassette transport on divergence), and `trait Transport { fn send(..) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send }`.
- `Cassette`/`Interaction` serde types; `CassetteTransport` replaying interactions strictly in order via an atomic cursor (no mutex — `std::sync::Mutex` is lint-banned and tokio is purity-banned), erroring loudly on any divergence, with `remaining()` so a test can assert every recorded interaction was exercised.

- [ ] Steps: write both modules with unit tests (in-order replay, mismatch reporting, exhaustion, remaining-count severity); `cargo clippy -p tam-marketplace --all-targets -- --deny warnings`; commit `feat(m1c): sans-io transport seam and cassette harness in the marketplace crate`.

---

### Task 2: tam-marketplace-tes — session, endpoints, classifier

**Files:**
- Create: `crates/tam-marketplace-tes/{Cargo.toml,src/lib.rs,src/session.rs,src/endpoints.rs,src/classify.rs}`; add to workspace members.

**Interfaces:**
- `TesSession` holding the cookie header with a redacting `Debug`; parsed from a Netscape jar (promoted from the spike) or raw header text.
- `endpoints`: typed request builders returning `HttpRequest` values and typed response parsers (create draft → `DraftId(i64)`; metadata body from a `TesListing` struct carrying title, markdown description, categories, ages, licence; presign response → the S3 form (policy base64-decoded for bucket and starts-with fields, promoted verbatim from the spike); confirm echo with `type: file, isUploaded: true`; publish; read; delete).
- `classify`: the spike's positive-assertion classifier promoted onto the seam vocabulary — 2xx with expected id → success value; 400/404/422 → `AdapterError::Rejected`; 401/403 → `AdapterError::SessionExpired`/`Challenge`; 429 → `RateLimited`; transients and sign-in interstitials → `Ambiguous`; `TransportError::NotSent` passes through as the one retry-safe class. The spike's seven classifier tests carry over and extend.

- [ ] Steps: crate with deps (tam-marketplace, tam-types, serde, serde_json, base64, sha2; reqwest NOT yet — Task 4); unit tests for jar parsing, policy decode, classifier; gate; commit `feat(m1c): tam-marketplace-tes session, typed endpoints and promoted classifier`.

---

### Task 3: schema-fingerprint pre-flight

**Files:**
- Modify: `crates/tam-marketplace-tes/src/endpoints.rs` (or new `src/schema.rs`)

**Interfaces:**
- The JSON-API analog of the selector-pack pre-flight: the draft object's sorted field-name set, canonically encoded and hashed (sha2) into `FormSchemaFingerprint`; compared against the crate's expected field set (the M0-observed shape). A difference produces `SchemaDrift { added, removed }` — the maintenance leading indicator the kill-gate section wants instrumented here.

- [ ] Steps: implement with tests (stable fingerprint under key reordering; drift names exactly the added and removed fields); gate; commit `feat(m1c): draft-schema fingerprint pre-flight with named drift`.

---

### Task 4: the flows and the adapter impl

**Files:**
- Create: `crates/tam-marketplace-tes/src/flows.rs`, `src/live.rs` (the reqwest transport)
- Modify: `src/lib.rs`

**Interfaces:**
- `TesAdapter<T: Transport>` keyed on `InventoryId` (TesGb | TesUs), implementing `MarketplaceAdapter`: `assert_form_schema` = the Task 3 pre-flight; `submit` = create draft → metadata → per-file presign/S3/confirm (files resolved through a sans-io `FileSource` seam trait — M1f wires object storage; tests provide bytes) → `SubmitEvidence`; `read_back` = the fetch-reason-gated GET.
- Inherent ops beyond the trait (the machine wires them in M1d): `publish` (licence asserted present first), `delete` (DELETE then GET, success only on 404 — the misleading-204 rule as code).
- `ReqwestTransport`: builder-constructed client (timeouts mandatory), session header injected, reqwest error mapping split `is_connect` → `NotSent` versus after-send → `AfterSend`.

- [ ] Steps: implement; gate (`clippy -p tam-marketplace-tes --all-targets -- --deny warnings`); commit `feat(m1c): Tes adapter flows — submit, publish, verified delete, gated read-back`.

---

### Task 5: cassette suite and the guarded live smoke

**Files:**
- Create: `crates/tam-marketplace-tes/tests/flows.rs`, `tests/cassettes/*.json`, `examples/live_smoke.rs`

**Interfaces:**
- Cassette tests: the full happy submit (create→metadata→presign→S3→confirm), publish, read-back, verified delete; the misleading-204 case (draft-DELETE 204 yet GET still 200 → the op refuses to report deletion); schema drift; the sign-in interstitial and rate-limit ambiguity cases; every test asserts the cassette is fully consumed.
- `examples/live_smoke.rs`: the spike's operator role, argument-driven (jar path), draft-only unless `--live-publish`; never runs in tests.

- [ ] Steps: author sanitized fixtures from the M0-observed shapes; write tests; full gate including `cargo nextest run -p tam-marketplace-tes`; commit `feat(m1c): cassette suite for every Tes flow and the guarded live smoke`.

---

### Task 6: delete the spike, close the milestone

**Files:**
- Delete: `spikes/tes-spike/` (decision record: deleted at promotion)
- Modify: `.gitignore` if spike-specific entries remain

- [ ] Steps: remove the tree; full workspace gate (`just check`, `just db-test`, `cargo deny check`, `nix flake check` on the committed tree); commit `chore(m1c): delete the promoted tes spike`.
