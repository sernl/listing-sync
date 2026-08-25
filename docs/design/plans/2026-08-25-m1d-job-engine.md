# M1d job engine implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** The machine gets its transition table and property suite; the job ledger gets org-first repositories with per-item leases, epoch fencing, the durable per-tenant mutex and fail-closed halts; a driver turns machine effects into adapter calls and ledger writes; the outbox drains through a delivery seam; the circuit breaker and rate governor bound account risk; and the worker and canary exist as thin binaries. This is where the account-safety and correctness kill gates are instrumented.

**Architecture:** `SyncMachine::step` implements `docs/design/sync-machine.md`'s table verbatim in `tam-domain` (pure, property-tested). Repositories land in `tam-storage` over the M1b tables. A new `tam-engine` library holds the driver, breaker, governor, outbox drainer and canary logic — the design allows early merging and later splitting, and a binary must link a library, not own logic. `tam-worker` and `tam-canary` are thin binaries. The scheduler binary is deliberately NOT in M1d (the build order omits it; M1j wires scheduling).

**Tech Stack:** proptest for the machine, sqlx over the M1b schema, uuid (v5) and blake3 for the idempotency derivation, tokio + tokio-util (`CancellationToken`) in the engine crate only.

## Global constraints

- The transition table in `docs/design/sync-machine.md` is the specification; `step` implements it verbatim, and every business outcome including ambiguity travels the success channel. `Err` is only for `InputNotApplicable`, `AttemptMismatch`, `EffectBudgetExceeded`.
- The five properties from the spec are the suite's floor: no `Submit` twice per `WriteAttemptId`; every terminal `Ambiguous` preceded by `RecordIntent`; no path from `Ambiguous` back to `Submit`; every `Committed`/`Degraded` preceded by `ReadBack`; `BudgetExhausted` terminal in one transition.
- The governing axiom: a stalled queue is recoverable, a duplicate-upload storm is not; every ambiguous resolution biases toward stalling.
- `lease_epoch` is the fencing token: a steal increments it, every ledger write carries the epoch it leased at, and a stale epoch's write is rejected, not raced.
- The per-tenant mutex (`JOBS_PER_TENANT = 1`) is a design rule from the form-token race, not a tunable; it is enforced in the lease-acquisition SQL and `UNIQUE (org_id, idempotency_key)` is its database backstop.
- Halts fail closed: a worker that cannot read the three halt tables refuses to automate.
- The idempotency key is UUIDv5 over the fixed-width canonical encoding in `sync-machine.md`, keyed on the INVENTORY (GB and US must not collide), namespace constant `NAMESPACE_TAM_INTENT` in `tam-types`, never rotated.
- Sessions for the Tes adapter come from an operator-supplied jar path until M1e's broker exists; the seam is the constructor, so M1e changes wiring, not flows.
- Commit recipe (jj, signed): `jj describe -m "<msg>"`, `jj bookmark set main -r @`, `jj new`, `git push origin main`.
- Deferred out of M1d, recorded so omission is decision: the scheduler binary and real cron windows (M1j); real email/push/billing deliverers behind the outbox seam (M5-adjacent; M1d ships the seam and a logging deliverer); `cargo-mutants` adequacy run (tracked as a lane to add when the tool enters the devshell); deterministic simulation (deferred by the design with named triggers).

---

### Task 1: `SyncMachine::step` and the property suite

**Files:**
- Modify: `crates/tam-domain/src/lib.rs` (implement `step`, add an `initial` constructor emitting the entry `AssertFormSchema` effect, plus whatever private helpers the table needs)

**Interfaces:**
- Produces: `SyncMachine::initial(org, inventory, item, key, strategy, budget) -> Transition` (entry row), `step(self, Input, LogicalInstant) -> Result<Transition, MachineError>` implementing all 20 rows; effect emission decrements `StepBudget` and exceeding it is `EffectBudgetExceeded`.
- The five properties as proptest over arbitrary input sequences, plus unit rows pinning each table row exactly (state in, input in, state out, effects out, in order).

- [ ] Steps: implement; `cargo clippy -p tam-domain --all-targets -- --deny warnings`; `cargo test -p tam-domain`; commit `feat(m1d): SyncMachine transition table and the five-property suite`.

---

### Task 2: idempotency derivation and the job-event vocabulary

**Files:**
- Modify: `crates/tam-types/src/lib.rs` (`NAMESPACE_TAM_INTENT: Uuid`, `JobEventPayload` serde enum whose tags are exactly `JobEventKind`'s twelve)
- Create: `crates/tam-marketplace/src/idempotency.rs` (uuid dep for v5)

**Interfaces:**
- Produces: `derive_idempotency_key(org, inventory, product, intent_version, intent_hash) -> IdempotencyKey` — fixed-width canonical bytes per the spec, UUIDv5; property tests: deterministic, inventory-sensitive (GB and US differ), and stable across requeue by construction.
- `JobEventPayload` with a test asserting its serde tags equal the `JobEventKind` set (the M1b deferral).

- [ ] Steps: implement with tests; purity lane still green (uuid is runtime-free); commit `feat(m1d): inventory-keyed idempotency derivation and the job-event vocabulary`.

---

### Task 3: ledger repositories — leases, fencing, mutex, halts, outbox, budget

**Files:**
- Create: `crates/tam-storage/src/jobs.rs`, `crates/tam-storage/tests/leases.rs`
- Modify: `crates/tam-storage/src/lib.rs`, `.sqlx` regenerated

**Interfaces (org first on every method):**
- `JobRepo`: `enqueue(org, job, items)` in one transaction (unique key refusal surfaces per item), `append_event(org, tx-scoped, job, item, JobEventPayload)` allocating `org_seq` by locking the counter row, `item_outcomes(org, job)` for the roll-up.
- `LeaseRepo`: `acquire(org-agnostic scan, worker, now, ttl) -> Option<LeasedItem>` — `SELECT ... FOR UPDATE SKIP LOCKED` honouring: no org with another live lease (the per-tenant mutex), no org/inventory/fleet halt (fail closed: absent read = no lease), rate budget available; `extend`, `settle(org, item, epoch, outcome…)` and `park`/`requeue` all epoch-fenced (a mismatched epoch updates zero rows and reports `StaleLease`); `expire_and_steal(now)` increments `lease_epoch`.
- `WriteAttemptRepo`: `open(org, item, mapping, epoch, intent, hash) -> attempt` (the partial index refuses a second in-flight), `settle(org, attempt, epoch, …)`.
- `OutboxRepo`: `append(tx, org, topic, dedupe, payload)`, `claim_due(now, batch)`, `delivered`, `retry_later(backoff)`, `dead`.
- `HaltRepo`: `raise_org_inventory`, `raise_fleet`, `active_scopes(org, inventory)`.
- `RateBudgetRepo`: `consume(org, connection, window, ceiling) -> Granted|Exhausted`.
- Tests (live database): epoch fencing — a stolen lease's old holder cannot settle; the per-tenant mutex — org A's second item is not acquirable while the first is leased, org B's is; halts fail closed; outbox dedupe and dead-letter at attempt cap; org_seq gapless-per-org under two concurrent appenders.

- [ ] Steps: implement; `just db-test`; commit `feat(m1d): ledger repositories with epoch fencing, tenant mutex and fail-closed halts`.

---

### Task 4: `tam-engine` — driver, breaker, governor, outbox drainer, canary logic

**Files:**
- Create: `crates/tam-engine/{Cargo.toml,src/lib.rs,src/driver.rs,src/breaker.rs,src/outbox.rs,src/canary.rs}`; workspace member.

**Interfaces:**
- `driver::run_item`: pump one leased item through the machine — execute each `Effect` in order (`AssertFormSchema`/`Submit`/`ReadBack`/`Reconcile` via `MarketplaceAdapter`; `RecordIntent`/`ParkItem`/`RequeueBehindGate`/`CaptureDiagnostics`/`Halt`/`Notify` via the repositories and outbox), feed the result back as the next `Input`, stop at `Terminal` or park; every ledger write carries the lease epoch; wall-clock and `StepBudget` enforced from `tam-limits`.
- `breaker`: ambiguous terminal → org-inventory halt (the machine already demands the effect; the breaker adds the failure-RATE trip to a fleet-inventory halt over a sliding window, durable in the halt tables).
- `outbox::drain(pool, deliverer, now)`: claim due, deliver through `trait Deliverer` (M1d ships a logging deliverer), exponential backoff `OUTBOX_BACKOFF_BASE..MAX`, dead-letter at `OUTBOX_MAX_ATTEMPTS`.
- `canary::probe(adapter, org)`: `assert_form_schema` for each Tes inventory, recording fingerprint or drift; drift raises the halt and notifies.
- Tests: a fake in-memory adapter walks the happy path to `Committed` and the ambiguous path to `Terminal(Ambiguous)` with the halt raised — cassette-style, no network; the drainer's backoff and dead-letter against the live database.

- [ ] Steps: implement; full lane; commit `feat(m1d): the engine — effect driver, breaker, outbox drainer, canary probe`.

---

### Task 5: the worker and canary binaries

**Files:**
- Create: `crates/tam-worker/{Cargo.toml,src/main.rs}`, `crates/tam-canary/{Cargo.toml,src/main.rs}`; workspace members.

**Interfaces:**
- `tam-worker`: argv only (database url, jar path, worker name, lanes ≤ `CONCURRENT_JOBS_GLOBAL_MAX`); lease pump loop with jittered poll, graceful ctrl_c (finish or park the in-flight item), `pg_notify` cancellation listener wired to a per-item `CancellationToken`.
- `tam-canary`: one probe run per invocation (systemd timer owns the schedule); exit code carries the verdict so the timer's unit state is the alert.

- [ ] Steps: implement thin; full workspace gate, `just db-test`, `cargo deny check`, `nix flake check` on the committed tree; commit `feat(m1d): tam-worker lease pump and tam-canary probe binaries`.
