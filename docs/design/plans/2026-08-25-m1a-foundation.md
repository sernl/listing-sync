# M1a foundation implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Stand up the production Rust workspace under the full enforcement gate, draw the five expensive-to-retrofit crate boundaries, and promote the compile-clean domain types and the pure `SyncMachine` from `sketches/domain.rs` into real crates with property tests.

**Architecture:** A cargo workspace of small crates per `docs/design/2026-08-25-listing-sync-design.md` section 14. The pure, sans-IO correctness core (`tam-domain`, `tam-marketplace`, `tam-types`) is promoted from `sketches/domain.rs`, which already compiles clean under `rustc 1.97.1 -D warnings`. Storage, API and binaries are drawn as real-but-thin boundaries. No marketplace I/O yet; that is M1c.

**Tech Stack:** Rust 1.97.1, the workspace lints table already in `Cargo.toml`, `clippy.toml`/`deny.toml`/`ban-probe` from `docs/design/enforcement-toolchain.md`, `proptest` for the state machine, `sqlx` (offline) for storage, `axum` for the API library.

## Global constraints

- Every crate inherits `[lints] workspace = true`; the gate is `cargo clippy --all-targets -- --deny warnings` plus `cargo fmt --check` plus `cargo nextest run`.
- `sketches/domain.rs` is the artefact of record for every domain type; promote its definitions verbatim, splitting them across crates per the table in Task 2-4, and never re-derive a type from memory.
- `tam-domain`, `tam-marketplace` and `tam-types` take no `tokio`, `tokio-util`, `reqwest` or `sqlx` dependency; this is asserted mechanically by `cargo-deny` bans, not left to convention.
- Public async trait methods are written `fn f(..) -> impl Future<Output = ..> + Send`, never `async fn`, because `async_fn_in_trait` is denied on the pinned toolchain (verified in M-1).
- The organisation identifier is `OrgId` and every repository method takes it as the first positional parameter.
- The enforcement config files (`Cargo.toml` lints, `clippy.toml`, `deny.toml`) are founder-gated shared state; changing a lint or adding a dependency is a founder decision, not a way to pass a build.
- The throwaway `spikes/tes-spike/` is retained until M1c promotes its endpoints, then deleted.
- Commit recipe (jj, signed): `jj describe -m "<msg>"`, `jj bookmark set main -r @`, `jj new`, `git push origin main`.

---

### Task 1: Production workspace and the full enforcement gate

**Files:**
- Modify: `Cargo.toml` (workspace members)
- Create: `clippy.toml`, `deny.toml`, `crates/ban-probe/{Cargo.toml,src/lib.rs}`
- Rename: `crates/tam-limits` stays; add it and `ban-probe` to members

**Interfaces:**
- Produces: a workspace where `cargo clippy --all-targets -- --deny warnings` is green with the full `disallowed-methods` list live, because `ban-probe` pulls the named crates into the graph so their paths resolve.

- [ ] **Step 1: Copy the verified gate config from the charter**

Extract the `clippy.toml`, `deny.toml`, and the `crates/ban-probe` crate verbatim from `docs/design/enforcement-toolchain.md` (they are the versions verified there). Add `crates/ban-probe` to `Cargo.toml` `[workspace] members`.

- [ ] **Step 2: Run the gate**

Run: `cargo clippy --all-targets -- --deny warnings`
Expected: exit 0, zero diagnostics. If reachability warnings fire, a `disallowed-methods` path is misspelled or its crate is missing from `ban-probe`'s manifest; fix the manifest, not the ban list.

- [ ] **Step 3: Confirm the ban list is armed**

Run: `cargo test -p ban-probe 2>&1 | rg -c 'unfulfilled'` after temporarily misspelling one `path =` entry; expected `1`, then revert. This proves a typo cannot silently disarm a ban.

- [ ] **Step 4: Commit**

Commit per the recipe: `chore(m1a): activate the full enforcement gate with ban-probe`

---

### Task 2: `tam-types` — the pure wire and vocabulary ADTs

**Files:**
- Create: `crates/tam-types/{Cargo.toml,src/lib.rs}`

**Interfaces:**
- Produces: `OrgId`, `Uuid`, `Timestamp`, `ContentHash`, `Marketplace`, `InventoryId`, `Currency`, `CurrencyRule`, `Money`, `PriceIntent`, `FileRole`, `FileKind`, `Title`, `ListingCopy`, `FieldKey`, `FailureCode`, `FailureDetail`, and the other pure serde ADTs, exactly as in `sketches/domain.rs`.

- [ ] **Step 1: Create the crate with serde only**

`Cargo.toml`: `serde = { version = "1", features = ["derive"] }`, `[lints] workspace = true`, `publish = false`. No `tokio`, `reqwest`, `sqlx`.

- [ ] **Step 2: Promote the pure ADTs from the sketch**

Copy from `sketches/domain.rs` into `src/lib.rs` the value types that carry no behaviour depending on I/O: `Uuid`, `Timestamp`, `ContentHash`, `Marketplace`, `InventoryId` (with its `impl`), `Currency`, `CurrencyRule`, `Money`/`MoneyError` (with `impl`), `PriceIntent`, `PriceRule`, `Rounding`, `FileRole`, `FileKind`, `ScanOutcome`, `Title`, `LengthUnit`, `ListingCopy`, `FieldKey`, `FailureCode`, `FailureDetail`. Add `OrgId(pub Uuid)`. Derive `Serialize, Deserialize` throughout.

- [ ] **Step 3: Build and gate**

Run: `cargo clippy -p tam-types --all-targets -- --deny warnings && cargo test -p tam-types`
Expected: green. `FailureCode` is the closed enum shared with the client; assert its variant count in a test so an accidental change is caught.

- [ ] **Step 4: Commit**

Commit per the recipe: `feat(m1a): tam-types promoted from the domain sketch`

---

### Task 3: `tam-marketplace` — the adapter seam and the three-valued outcome

**Files:**
- Create: `crates/tam-marketplace/{Cargo.toml,src/lib.rs}`

**Interfaces:**
- Consumes: `tam-types`.
- Produces: the `adapter` module's `MarketplaceAdapter` trait, `DraftSupport`, `CreateStrategy`, `MarkerField`, plus `Outcome`, `AmbiguityCause`, `ChallengeKind`, `WriteReceipt` (with its private constructor and sole caller in this crate), `FetchReason`, `RemoteListingId`, `IdempotencyKey`, `WriteAttemptId` — exactly as in `sketches/domain.rs`.

- [ ] **Step 1: Create the crate**

`Cargo.toml`: depends on `tam-types`; `futures` only for `Future` if needed; no `tokio`, `reqwest`, `sqlx`. `[lints] workspace = true`.

- [ ] **Step 2: Promote the seam from the sketch**

Copy the `pub mod adapter { .. }` block and the outcome types (`Outcome`, `FieldDiffReport`, `AmbiguityCause`, `ChallengeKind`, `WriteReceipt`, `FetchReason`, `RemoteListingId`, `CorrelationMarker`) from `sketches/domain.rs`. Keep `WriteReceipt`'s constructor private so the read capability is airtight, as the design's crate-layout note requires. Write every trait method as `fn f(..) -> impl Future<Output = ..> + Send`.

- [ ] **Step 3: Prove the async-trait desugar compiles under the gate**

Run: `cargo clippy -p tam-marketplace --all-targets -- --deny warnings`
Expected: green, with no `async_fn_in_trait` diagnostic, confirming the `impl Future + Send` form.

- [ ] **Step 4: Commit**

Commit per the recipe: `feat(m1a): tam-marketplace seam with Ambiguous outcome and read capability`

---

### Task 4: `tam-domain` — the pure `SyncMachine`, projection and diff

**Files:**
- Create: `crates/tam-domain/{Cargo.toml,src/lib.rs}`

**Interfaces:**
- Consumes: `tam-types`, `tam-marketplace`.
- Produces: `SyncMachine` with its seven `SyncState`, eight `Input`, ten `Effect`, `Transition`, `MachineError`, `StepBudget`, `LogicalInstant`, plus `CanonicalProduct`, `ListingProjection`, `ProjectionBlocked`, `Mapping`, `FieldMismatch`, `MismatchClass`, `FieldPolicies`, per `docs/design/sync-machine.md` and `sketches/domain.rs`.

- [ ] **Step 1: Create the crate with the dependency ban asserted**

`Cargo.toml`: depends on `tam-types`, `tam-marketplace`; dev-dep `proptest`. Add a `deny.toml` `[bans]` entry (or a workspace-level assertion) that `tokio`, `tokio-util`, `reqwest`, `sqlx` never appear in `tam-domain`'s subgraph, because a convention is one `cargo add` from false.

- [ ] **Step 2: Promote the machine and the projection/diff from the sketch**

Copy `SyncMachine`, its `step` function, the `SyncState`/`Input`/`Effect`/`EffectList`/`Transition`/`MachineError` types, `projection` (`CanonicalProduct`, `ListingProjection`, `ProjectionBlocked`), and `diff` (`FieldDiffReport`, `FieldMismatch`, `MismatchClass`, `MismatchResponse`, `FieldPolicies`) from `sketches/domain.rs`. The machine performs no I/O; every wanted action is an `Effect` in the returned `EffectList`.

- [ ] **Step 3: Property tests over the machine**

Add `proptest` tests asserting the invariants the sketch documents: `step` is total (never panics on any `(state, input)`), the effect count never exceeds `StepBudget`, an `Ambiguous` create never yields a retry effect, and a park-then-requeue preserves the `IdempotencyKey`. Use `proptest-state-machine` for the transition-level frontier if available; otherwise hand-rolled generators.

- [ ] **Step 4: Run the tests and the ban**

Run: `cargo nextest run -p tam-domain && cargo deny check bans`
Expected: property tests pass; `cargo deny` confirms no banned crate in the subgraph.

- [ ] **Step 5: Commit**

Commit per the recipe: `feat(m1a): tam-domain SyncMachine, projection, diff, property-tested`

---

### Task 5: `tam-storage` skeleton — org-first repositories and the first migration

**Files:**
- Create: `crates/tam-storage/{Cargo.toml,src/lib.rs}`, `crates/tam-storage/migrations/0001_product.sql`, `crates/tam-storage/tests/tenancy.rs`

**Interfaces:**
- Consumes: `tam-types`.
- Produces: a `ProductRepo` whose every method takes `org: OrgId` as the first positional parameter, and the `product` table with row-level security, per `docs/design/schema.md`.

- [ ] **Step 1: The first migration**

Copy the `product` table DDL from `docs/design/schema.md` into `0001_product.sql`, including its `org_id` column and a row-level-security policy keyed on the tenant. Enable `FORCE ROW LEVEL SECURITY`.

- [ ] **Step 2: The org-first repository**

`ProductRepo::insert(org: OrgId, ..)`, `::get(org: OrgId, id: ..)`, `::list(org: OrgId, ..)` — `OrgId` is always first and never read from task-local state. Use `sqlx` with `SQLX_OFFLINE=true` and a checked-in `.sqlx/` metadata directory.

- [ ] **Step 3: The two-tenant negative test**

`tests/tenancy.rs`: insert a product under `OrgId` A, then assert a read under `OrgId` B returns nothing, proving RLS isolation. Run against a disposable Postgres (a `sqlx::test` fixture or a testcontainer).

- [ ] **Step 4: Run**

Run: `cargo sqlx migrate run` against the test database, then `cargo nextest run -p tam-storage`
Expected: migration applies; the two-tenant test passes (B sees nothing).

- [ ] **Step 5: Commit**

Commit per the recipe: `feat(m1a): tam-storage org-first repo, first migration, two-tenant RLS test`

---

### Task 6: `tam-api` library and the thin binaries

**Files:**
- Create: `crates/tam-api/{Cargo.toml,src/lib.rs}`, `crates/tam-server/{Cargo.toml,src/main.rs}`, `crates/tam-session-broker/{Cargo.toml,src/main.rs}`
- Create: `crates/tam-api/tests/router.rs`

**Interfaces:**
- Produces: `tam_api::router() -> axum::Router` built in-process, with the structured `APIError`/`APIErrorEntry` type and the `APIVersion` extractor adopted from the reference repo per the design's crate-layout section; `tam-server` and `tam-session-broker` as thin binaries holding only argument parsing and wiring.

- [ ] **Step 1: The API as a library**

`tam-api` is a library, not a binary, so integration tests drive the `Router` via `oneshot` with no bound port. Add the structured `APIError` builder with its debug-versus-release disclosure split, and the `APIVersion` `FromRequestParts` extractor.

- [ ] **Step 2: An in-process router test**

`tests/router.rs`: build `router()`, send a `oneshot` request to a `/healthz` route, assert 200. No network, no bound port.

- [ ] **Step 3: The thin binaries**

`tam-server` and `tam-session-broker` each contain only configuration parsing and a call into their library; the broker is a separate binary from the first commit because a privilege boundary cannot be introduced later without redesigning every call site holding plaintext session material.

- [ ] **Step 4: Run the whole gate**

Run: `cargo fmt --check && cargo clippy --all-targets -- --deny warnings && cargo nextest run`
Expected: the entire workspace is green under the gate.

- [ ] **Step 5: Commit**

Commit per the recipe: `feat(m1a): tam-api library, in-process router test, thin server and broker binaries`

---

## What M1a does not do

No marketplace I/O, no job engine, no credential encryption, no file pipeline, no taxonomy projection, no web client; those are M1b onward.
M1a's proof is that the workspace builds green under the full gate, the five expensive boundaries exist, and the pure correctness core is promoted from the sketch and property-tested.
