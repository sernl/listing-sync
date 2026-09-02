# Splitting the engine driver from storage

Phase 1 of the two-branch architecture: what the repository trait must be, what it must refuse, and what has to be fixed before it is cut.

- date: 2026-09-03
- method: five parallel read-only review lenses (seam, scheduling, correctness, simplicity, custody) over `crates/tam-engine`, `crates/tam-domain`, `crates/tam-storage`, the two hosts and the transport seam, with the blocking and should-fix findings put through adversarial refutation; no code written
- status: design input to Phase 1, nothing built; it inherits `docs/notes/design/vendoo-for-teachers-rethink.md` and `docs/notes/design/client-side-architecture.md` sections 5 and 6

## 1. Purpose, and the decisions it serves

Phase 1 splits `crates/tam-engine/src/driver.rs` from concrete `tam-storage` repositories so the effect interpreter runs against a remote ledger.
It exists because D1 puts every TPT and Tes request on the seller's own device, and the interpreter is the process that decides what request to make.
D27 follows it, because the upload is itself such a request and the bytes must be on the device at upload time.
D14 constrains it, because a seller may have several devices claiming from one queue.
D11 gates it, because an entitlement token is the only subscription enforcement left for work that runs on the seller's machine.

The review's structural result is that Phase 1 is not the mechanical refactor its one-line description implies.
The trait extracted in Phase 1 becomes the wire contract in Phase 2, so every trust decision the trait's shape encodes is taken now or re-cut later.
A trait derived mechanically from what the driver calls today would hand a seller's device four capabilities it must never hold: a fleet-wide halt that stops every tenant on a marketplace (`crates/tam-engine/src/driver.rs:834`), a rate window and ceiling supplied by the governed party (`crates/tam-engine/src/driver.rs:389`), an organisation id supplied as an argument under a BYPASSRLS role (`crates/tam-engine/src/driver.rs:498`), and an outbox topic string that names any relay the drainer knows (`crates/tam-engine/src/driver.rs:1365`).
None of those is a defect while the only caller is our own worker in our own process, and all four become one the moment the caller is a process the seller controls.

Two further results reshape the phase.
The stated verification, that the existing engine tests pass unchanged against an in-memory ledger, cannot be run: all forty-nine engine integration tests are `#![cfg(feature = "pg-tests")]`, take a `PgPool` from `#[sqlx::test]`, open a second connection as the BYPASSRLS `tam_engine` role, and assert with raw SQL against `mapping`, `job_item` and `job_event`.
And the split does not reach far enough as scoped: `DriverContext` holds `pub pool: &'a sqlx::PgPool` beside the four repositories (`crates/tam-engine/src/driver.rs:87`), used at `:1262` and `:1358` for the event journal and the seller-notification outbox, so a trait over the four repositories alone leaves the interpreter unable to compile without a live Postgres connection.

## 2. The driver as it is today

The seam is in better shape than the plan implies for granularity and worse for portability.
No driver flow needs a SQL transaction spanning two repository calls, so the seam can be cut at the method boundary without inventing a distributed transaction: every multi-statement transaction, savepoint and row lock is already sealed inside one repository method.
`WriteAttemptRepo::settle` (`crates/tam-storage/src/jobs.rs:1530`) does the attempt row, the mapping bind or sever with a savepoint, and the counterpart revive in one transaction; `LeaseRepo::settle` (`:890`) does the item settle, `settle_if_complete`, the `JobSettled` event and the seller's outbox row in one.

| Concern | Held as | Driver call sites | Shape |
|---|---|---|---|
| Connection identity | `LeaseRepo::connection_for` | `driver.rs:498` | read, once per run, cannot change during a run |
| Lease lifecycle | `LeaseRepo` preflight, park, settle | `:550`, `:769`, `:919`, `:1052`, `:1104`, `:1338` | epoch-fenced compare-and-set on `(org, item, lease_epoch)` |
| Connection gate | `LeaseRepo::gate_connection` | `:797`, `:1082` | control-plane write outliving the lease |
| Write attempts | `WriteAttemptRepo` open, settle | `:562`, `:882`, `:1207` | the duplicate-create fence and the mapping bind |
| Rate budget | `RateBudgetRepo::consume` | `:392`, `:978` | one call per write plus one per verification try, up to twelve per item |
| Halts | `HaltRepo` three scopes | `:829`, `:832`, `:834` | one tenant-scoped, two cross-tenant |
| Event journal | raw `ctx.pool` transaction | fourteen sites via `:1262` | per-organisation counter lock per append |
| Notification outbox | raw `ctx.pool` transaction | `:839`, `:1115` via `:1358` | at-least-once, deduped on `(org, topic, dedupe_key)` |
| Preparation | `seed.rs` eleven calls | `seed.rs:244` | mapping, product, taxonomy, elections; three writes |

Two methods the driver never calls are nonetheless part of the split's problem.
`LeaseRepo::acquire` (`crates/tam-storage/src/jobs.rs:793`) is the cross-tenant BYPASSRLS scan whose candidate filter is the entire admission policy, and `expire_and_steal` (`:1040`) is its reaper.
Both are host concerns today, and an inventory taken from `driver.rs` alone misses them.

## 3. The split

### The trait surface

Three ports, shaped by the trust boundary rather than by the current call graph.

`ItemLedger` is the client-reachable one, and every method is keyed on `&LeaseRef` so the server derives org, connection, inventory and epoch from the lease it issued rather than trusting a client argument.
Its methods are `preflight_succeeded`, `preflight_failed`, `park`, `settle_item`, `settle_attempt`, `open_attempt`, `record_event`, `notify`, `report_reauth_required`, `halt_this_tenant`, `request_grant` and `renew`.
Four of those are reshapings rather than renames.
`open_attempt` takes a client-minted attempt id so a lost response is recoverable.
`request_grant` returns a windowed grant with the window and the ceiling derived server-side.
`halt_this_tenant` fixes the scope to org-inventory and takes no scope argument.
`notify` takes the closed `SellerEvent` enum and never a topic string, because `seller_event_topic` is already a total function of it (`crates/tam-engine/src/driver.rs:448`).

`Preparation` serves the work order: `work_order(&LeaseRef) -> ItemPreparation`, implemented on the server by today's `prepare_item` against Postgres, returning the projected listing.
That keeps the whole taxonomy server-side and makes D1's declarative intent literally the return value of one call.

`LedgerInspector` exists only for tests, carrying the observations the existing assertions need: binding state, attempt state, item state, connection state, halt rows, outbox rows and events for an item.
It is not exposed to the client and has no wire form.

### What moves to the device

The interpreter itself, `run_item` and its effect loop.
`seed_from_projection` and `seed_for_removal`, which touch no storage and call only `adapter.project_fields` (`crates/tam-engine/src/seed.rs:501`, `:544`).
Both adapters and the transport, which the plan already books as portable.
`FileSource`, under D27, once the `blake3` `pure` feature question is answered.

The intent hash is computed over the rendered field set (`crates/tam-engine/src/seed.rs:507`) and the idempotency key is derived from it, so the server cannot know either until the device has composed.
`RecordIntent` therefore records what the device rendered, and the port operation carries the composed field set back.
If instead the server computes the intent hash it must render the field set, which is composition, and puts the architecture at S3.

### What stays on the server

`prepare_item` whole, because it writes the seller's decision surface: the taxonomy raise, the election raise and `record_losses` all run under the engine's own grant (`crates/tam-engine/src/seed.rs:386`, `:401`, `:472`).
`outbox.rs`, which claims fleet-wide and delivers the seller's email (`crates/tam-engine/src/outbox.rs:73`).
`breaker.rs` and `canary.rs`, both of which raise fleet halts across tenants (`crates/tam-engine/src/breaker.rs:42`, `crates/tam-engine/src/canary.rs:30`).
`LeaseRepo::acquire`, `expire_and_steal` and `revive_expired`, because the stall bias depends on someone reaping leases whose holder went away, and on a device-hosted driver the holder going away is the common case.
The per-organisation event sequence, which is allocated under a row lock (`crates/tam-storage/src/jobs.rs:367`) and cannot be minted on a device.
`raise_org` and `raise_fleet_inventory`, which leave the effect vocabulary's reach entirely.

### What is deleted

`tam-session-broker` at 4,326 lines, per the plan's table, with one correction: `vault.rs`'s `claim`, `is_exclusivity_violation` and the digest-clearing half of revoke implement the global exclusivity claim that section 5 explicitly keeps, so they lift into `tam-storage` with a non-KEK pepper rather than going with the file.
`broker_client.rs` at 249 lines, with the same correction: roughly 100 lines go (`LeaseRequest`, `LeasePurpose`, `GatewayLease`, `request_lease`, the `Leased` arm and the purpose test) and `claim_account`, `ClaimError`, `BrokerErrorCode` and `exchange` re-home as a `tam-api` call.
The dead custody scaffolding in `crates/tam-marketplace/src/lib.rs:711-767` plus `GrantId` at `:32`, 57 lines with no implementation and no call site, which compiles into every client build today.
`const _: fn(sqlx::PgPool) -> OutboxRepo` at `crates/tam-engine/src/driver.rs:1376`, which exists only to make an import used.

## 4. The pull model and the device-side scheduler

There is no cron anywhere in the tree today, because the charter's deterministic, cron-scheduled shape is two long-lived poll loops and two timer-driven one-shots, and only the location of the timer changes under section 6 of the client-side note.

Two disjoint queues share one ledger: the claim statement gains a transport-class predicate so the server's cross-tenant `acquire` sees only `OfficialApi` items and the device's org-pinned claim sees only `SellerDevice` ones.
Make the predicate a column on `marketplace_inventory` seeded from `Marketplace::transport_class()` and asserted equal to it by a pg-gated test, so D1's build-failing test has a database fact to bind to rather than a Rust match a SQL statement can route around.

The device claim is a new org-pinned `claim_for_device` under `tam_app`, where forced row-level security pins the tenant for free.
It writes `lease_owner = <device_id>` into the existing free-form column, computes `lease_expires_at` in SQL as `now() + $ttl`, and emits `ItemLeased` in the same transaction.
The response is the declarative envelope: the server-computed operation and projected listing, the create strategy, the step budget, the verify policy, a bulk rate grant, the absolute server deadline, the server's own `now`, and a jittered next-poll delay.
It contains no URL, no header, no form field name and no encoding; the device's own adapter composes those.
The server never says now: it answers what is due when asked, and the seller's local timer decides when to ask.

For two devices under one seller, the live-lease mutex is a partial unique index on `(org_id)` (`crates/tam-storage/migrations/0008_tenant_mutex.sql:8`), so a seller's second device polls, loses at the index, and receives `Ok(None)`, which is byte-identical to an empty queue.
The invariant it protects is one live session per marketplace account, which is a per-connection property; the per-tenant scope was right only because there was exactly one claimant process.
Re-scope it to the connection, and have the pull endpoint distinguish no work from held by another device so the idle one backs off instead of hot-polling.

For a device that goes offline mid-flow, claims become renewable: add an epoch-fenced `renew`, heartbeat it during a submit or an upload, and reclaim a device that stopped heartbeating rather than one that is merely slow.
The lease TTL is a fixed 300 seconds (`crates/tam-worker/src/main.rs:81`) and the driver's own budget inequality already sits at roughly 180 seconds of submit against it, failing at TPT's theoretical worst case where two queue polls alone spend 360 seconds (`crates/tam-engine/src/driver.rs:44`).
Every reason that margin is thin gets worse on a laptop that closes its lid.

Replay and idempotency are decided per operation, and the ledger already knows it.
`Effect::Revise` and `Effect::Remove` carry no idempotency key, because neither platform offers an idempotent edit or delete (`crates/tam-domain/src/lib.rs:631`).
Create is at-most-once, fenced by the standing in-flight attempt, while publish, revise and remove are at-least-once, safe to repeat because they address an existing listing, and proved by `verification_settles` on the read-back rather than by the write's own status code (`crates/tam-domain/src/lib.rs:551`).
Carry the operation's delivery class in the envelope as a field the device branches on, so a future operation cannot be added without its class being decided.
One rule makes this hold: re-offer the same `job_item` with a bumped epoch, never a fresh job, because `intent_digest` keys publish, revise and remove on the JobId (`crates/tam-storage/src/job_reads.rs:641`, `:649`).

On clock skew, every expiry must be a server fact computed in SQL and never a device timestamp.
Today both `lease_expires_at` and `park_expires_at` are written from the claimant's clock and read against the reaper's (`crates/tam-storage/src/jobs.rs:799`, `:1053`, `:1125`), which is safe only because both are our processes.
The device drives its local budgets off `server_deadline - server_now`, not off its own absolute clock, exactly as `wall_deadline` does today but with the origin instant supplied rather than read.

Evidence returns as a settle envelope fenced on `(item, lease_epoch)`, carrying the submit evidence, the read-back observation, the accumulated action events, the actual rate consumption, and any control-plane effect the device requested.
The server decides, so `Halt`, `Notify`, `RequeueBehindGate`, `ParkItem` and `RecordIntent` are server-only in the sense that the server re-derives their scope from the lease rather than honouring an asserted one.

Entitlement gates the pull rather than each request, per D10's per-check-in nuance, and is enforced twice.
The endpoint verifies the signature, the one-hour validity and the 24-hour grace, failing closed.
Independently, an entitlement predicate joins the candidate CTE beside the three halt tables, so a forged token still selects zero rows.
D11's per-marketplace kill switch needs no new mechanism: `org_inventory_halt` and `inventory_halt` are already in the candidate filter and already fail closed (`crates/tam-storage/src/jobs.rs:807`), and they only need a new writer.

## 5. Custody

The line D1 draws is not the transport seam, which is a genuine single network door whose `HttpRequest` carries no header field, so no session material is representable there (`crates/tam-marketplace/src/transport.rs:293`).
The line is which process composes and issues, and today five server binaries compose and issue requests to `SellerDevice` marketplaces under seller sessions: `tam-worker`, `tam-sync-worker`, `tam-analytics`, `tam-canary` and `tam-import`.
Section 4 of the client-side note lists four of those five as staying server-bound, which contradicts D1 and needs correcting rather than leaving two documents disagreeing.

The entitlement gate sits in two places in the loop, and one is not enough: the transition table emits at most one network-bearing effect per batch, so a per-effect check immediately before each is exact rather than conservative, beside `consume_write_grant` at `crates/tam-engine/src/driver.rs:595`, `:619` and `:653`, and at `AssertFormSchema` at `:547`.
It must be repeated inside `verify_with_backoff`'s per-try preamble at `:381`, because one `ReadBack` expands into up to eleven marketplace reads on TPT.
A failed check produces the shape `BudgetGrant::Exhausted` already produces, settling the open attempt abandoned and returning `Abandoned`, never a new terminal outcome, so a revocation mid-run never settles an item on evidence the run does not have.

The audit trail does not survive relocation unchanged, because every ledger write the driver makes is stamped `Stamp::system(SystemComponent::Engine, at)` from a closed four-member set (`crates/tam-types/src/actor.rs:19`), and after relocation the engine is a process on the seller's machine and the instant is the seller's clock.
Add a `Device` component carrying the device id from D14's registry, and record two instants on device-originated rows: the device-asserted one and the server's receipt.
`org_seq` already makes ordering server-authoritative, so the seller's timestamp is preserved as their assertion rather than mistaken for our record.
`JobEventPayload::ItemLeased` is declared and emitted nowhere (`crates/tam-types/src/lib.rs:716`), and D1 requires progress that names the device doing the work; emit it from inside the claim's own transaction with the device id as `worker`.

The authorship attestation's only writer is inside the crate being deleted (`crates/tam-session-broker/src/vault.rs:234`), and the attestation is per connection while TPT's copyright declaration is a per-listing radio group.
Carry `attested_by` and `attested_at` into `AttemptIntent.body` at `RecordIntent`, so the `write_attempt` row is the immutable record of the attestation the write went out under, and leave `intent_hash` alone because it feeds the idempotency key.
The exclusivity claim needs the same care from the other side: its identity read becomes client-asserted, which is the seller-typed value `claim_account` documents as a denial-of-service primitive, and its digest pepper is derived from the key-encryption key that goes with the vault.
Re-site the pepper in a server-held key that outlives the vault, bump `key_version` and re-claim rather than attempting a re-keying migration, because the account reference preimage is deliberately never stored.

## 6. The correctness defects to fix as part of or before the split

Eight defects survived adversarial refutation, and each of them is a stall or a duplicate that today needs a worker crash and after relocation needs a closed lid.

The loop-top cancellation and deadline check steps `Input::BudgetExhausted` into whatever machine the previous iteration produced, without checking whether that machine is already terminal or still holds an unexecuted effect list (`crates/tam-engine/src/driver.rs:535`).
A terminal machine answers `InputNotApplicable`, so the run returns `Err` with a committed listing unsettled; a parked one silently discards `[ParkItem, RequeueBehindGate, Notify]` and settles the attempt abandoned, releasing the only fence against a second create while the mapping is unbound.
No engine test reaches either path, because nothing cancels the token and `SteppingClock` advances one second per call against a thirty-minute deadline.
Guard the check on the next state being neither terminal nor parked, or hoist it to just before the step at `:948`.

A stranded in-flight attempt on a create is permanent, mapping-scoped and invisible.
`may_settle_unverified` deliberately leaves the row standing (`:1179`), no reaper settles it (`crates/tam-storage/src/jobs.rs:1040`, `:1209`), and `failed_writes` filters on a non-null failure code so the row never reaches an operator (`crates/tam-storage/src/backoffice.rs:463`).
The remedy the code names for itself is a read-back reconciliation that settles the attempt on what is actually there; until it exists, widen the operator view to surface `state = 'in_flight'` past the lease TTL.

`WriteAttemptRepo::open` mints the attempt id server-side with no re-read (`crates/tam-storage/src/jobs.rs:1481`), so a lost response strands the run and burns the item's whole attempt budget.
Have the driver mint the id and pass it in, turning `open` into an idempotent write keyed on `(org, attempt_id)`, and note that a resuming create still needs the read-back before it may proceed.

`RateBudgetRepo::consume` takes both the window key and the ceiling from the caller (`crates/tam-storage/src/jobs.rs:2125`), so a client-hosted driver sets its own rate limit.
Replace it with `request_grant(&LeaseRef, GrantKind)` where the server derives the connection from the lease and the ceiling from its own copy of `tam-limits`.

`Effect::Halt` reaches `raise_org` and `raise_fleet_inventory` (`crates/tam-engine/src/driver.rs:832`, `:834`), which stop one tenant entirely and every tenant on a marketplace respectively.
The machine only ever produces `OrgInventory` today, so both arms are unreachable, and they are exactly the arms a relocated interpreter makes reachable.

Claim and park expiry are computed from the claimant's clock and compared against the reaper's (`crates/tam-storage/src/jobs.rs:799`, `:1053`).
A device running fast wedges its tenant's queue past a TTL the server thinks it granted; one running slow has its claim stolen mid-submit.

Admission is checked once in `prepare_item` and never re-evaluated when the attempt opens (`crates/tam-engine/src/seed.rs:197`), so a resumed run and a stolen lease can both create.
Add the binding predicate to `open`'s statement for a create and return a distinct refusal the driver settles as skipped.

A create parked on `ReauthRequired` is reachable by no reaper: the clock and give-up arms filter on `REVIVABLE_GATES`, which deliberately excludes that gate, and the re-link arm excludes creates (`crates/tam-storage/src/jobs.rs:1209`).
Its `attempt_count` never advances either, so an attempt-budget give-up cannot fire; any give-up for this gate keys on park age.

## 7. A sequence of small landable changes

Each step keeps `just check` green on its own and names what it proves, how it is verified and what would falsify it, with step 2 a founder act rather than a code change and a gate on step 6.

Step 1, delete the dead scaffolding.
Remove `CustodyModel`, the second `LeasePurpose`, `SessionLease`, `CustodyError`, `ConnectionProvider` and `GrantId` from `crates/tam-marketplace/src/lib.rs`, drop the ignored `org` parameter from the five `MarketplaceAdapter` methods, and delete the `const _` and its import at `crates/tam-engine/src/driver.rs:1376`.
Proves nothing about the split and removes the most misleading vocabulary a reader of the client plan meets, 167 lines with no call sites.
Verification: `just check` and `just check-portable`, both unchanged in outcome.
Kill gate: a call site the search missed.

Step 2, restate the Phase 1 verification.
The current wording cannot be run, so replace it with a port-conformance statement: the same test bodies pass against both a Postgres ledger and an in-memory one, with the raw-SQL assertions lifted onto a `LedgerInspector` and the role-separation fixtures named and kept Postgres-only.
Proves the phase has a falsifier at all.
Verification: the founder's ruling, recorded in the decision table.
Kill gate: none; this is the gate.

Step 3, fix the loop-top budget step.
Guard `crates/tam-engine/src/driver.rs:535` on the next state being neither terminal nor parked, or move the check to just before `:948`.
Proves the interpreter cannot discard a committed transition, which is the defect that turns a suspended device into an unsettled listing.
Verification: two new driver tests, one cancelling the token immediately after the read-back predicate is satisfied and asserting the item settles, one cancelling after a `SessionExpired` submit and asserting the park, the gate and the notification all land.
Kill gate: the guard changes an existing verdict in the gauntlet.

Step 4, move the journal and the notification off the raw pool.
Add `JournalPort` and `NotifyPort`, implement both on the server with today's transaction-taking functions, and delete `pool` from `DriverContext`.
Proves the interpreter's own signature is free of `sqlx`, which no trait over the four repositories achieves.
Verification: `cargo clippy -p tam-engine --all-targets` under deny-warnings, plus the existing engine suite unchanged.
Kill gate: a transaction is widened or narrowed by the relocation; both wrappers open a transaction containing a single write today, so neither should be.

Step 5, lift the ledger port into its own crate and move the interpreter with it.
Create `tam-engine-driver` holding `driver.rs`, the effect vocabulary and the three port traits, leave `seed.rs`, `outbox.rs`, `breaker.rs` and `canary.rs` in `tam-engine`, and shape the trait for the trust boundary from the start: keyed on `&LeaseRef`, no scope argument on the halt, no window or ceiling on the grant, no topic string on the notification, a caller-minted attempt id on the open.
Proves the boundary by compilation rather than by review.
Verification: extend `just purity` to ban `sqlx`, `tokio`, `tokio-util` and `tam-storage` from the new crate, and add it to `portable_crates` so `just check-portable` compiles it for all five client triples.
Kill gate: a port method that cannot be shaped without an argument the client must not supply.

Step 6, add the in-memory ledger and run the suite twice.
Extract each existing test body into a function generic over the ports plus `LedgerInspector`, keep the current `#[sqlx::test]` wrapper as the Postgres instantiation, and add an ungated in-memory instantiation.
Proves what step 2 restated: a split that leaked a storage assumption shows up as a divergence between the two runs rather than as a compile error somebody fixes by hand.
Verification: every shared body green in both instantiations, with the in-memory run inside `just check` rather than `just db-test`.
Kill gate: an assertion that cannot be expressed through the inspector without exposing an operation the client must not have.

Step 7, harden the server side behind the trait.
Compute `lease_expires_at` and `park_expires_at` in SQL, make `open` idempotent on a caller-minted id, add the transport-class column and its predicate, re-scope the live-lease mutex to the connection, add the entitlement predicate to the candidate CTE, and add a `Device` actor component.
Proves the ledger is safe against a caller it does not trust, independent of what the client does.
Verification: `just db-test` extended with a two-tenant negative case, a lease issued for org A refused every ledger write naming org B, and a two-device concurrency case asserting exactly one successful `open` per mapping.
Kill gate: the mutex re-scope is refused as a founder-gated limit, in which case the pull endpoint must instead distinguish held-by-another-device and the throughput ceiling is accepted.

Step 8, add the device pull endpoint and the settle envelope.
Serve `claim_for_device` and the declarative envelope from `tam-api`, bind the lease to the requesting device, and refuse any epoch-fenced write from a device other than the holder.
Proves the whole loop end to end without a client existing yet.
Verification: an api-flow test claiming as one device, settling as another and asserting the refusal; and the same claim replayed against a lapsed entitlement returning zero rows with a valid-looking token.
Kill gate: an envelope field that cannot be computed without composing a marketplace request.

## 8. Open questions for the founder

1. Restate the Phase 1 verification as port conformance over two ledger implementations. Recommended: yes, because the current wording is unrunnable and the phase would otherwise ship without evidence.
2. Re-scope the live-lease mutex from the organisation to the connection. Recommended: yes, because the form-token race it protects against is per marketplace session and D14 grants several devices; this changes `JOBS_PER_TENANT`, which is founder-gated.
3. Record a disposition for `tam-analytics`, `tam-sync-worker`'s Tes read leg, `tam-canary` and `tam-import`, and correct the client-side note's keep-list. Recommended: the analytics capture becomes a device-pulled read item, the Tes read leg moves to the device and the crate keeps its enqueue half, `tam-canary` becomes a founder-run desktop command, and `tam-import` is restricted to manifest bytes or routed the same way.
4. Enable `blake3`'s `pure` feature so `tam-pipeline` cross-compiles, and state whether the interim arrangement is server-held bytes streamed at upload time or full client-side ingest. Recommended: enable it in Phase 1 rather than discovering it in Phase 2, and state client-side ingest as the target with server-held bytes as the interim.
5. Decide whether the rate grant is issued in bulk at claim time and whether consumption moves into the transport seam. Recommended: bulk grant at claim time with reported consumption in the settle envelope, and move consumption into the seam, because the marketplace now sees the seller's own address.
6. Decide whether `HaltScope::Org` and `HaltScope::FleetInventory` leave the effect vocabulary. Recommended: yes, narrow the enum in `tam-domain` so the driver's match cannot name a scope wider than its lease, leaving the breaker and the canary as the only fleet-halt writers.

## 9. Appendix: the findings

Severity is B blocking, S should-fix, D design-input, N nit; every blocking and should-fix row survived adversarial refutation.

| # | Finding | Location | Sev |
|---|---|---|---|
| 1 | Phase 1 verification cannot be run: every engine test is Postgres-gated and asserts in raw SQL | `crates/tam-engine/tests/driver.rs:7`, `gauntlet.rs:8`, `seed.rs:10`, `breaker.rs:8`, `drain.rs:5` | B |
| 2 | Loop-top budget step applied to an already-terminal machine returns `Err` with the item unsettled | `crates/tam-engine/src/driver.rs:535` | B |
| 3 | The same step discards an unexecuted effect list, releasing the duplicate-create fence | `crates/tam-engine/src/driver.rs:536` | B |
| 4 | A stranded in-flight create attempt is permanent, mapping-scoped and invisible to the operator | `crates/tam-engine/src/driver.rs:718`, `crates/tam-storage/src/jobs.rs:1209`, `backoffice.rs:463` | B |
| 5 | `WriteAttemptRepo::open` mints the id with no idempotent re-read, so a lost response burns the budget | `crates/tam-storage/src/jobs.rs:1481` | B |
| 6 | The rate window and ceiling are caller-supplied, and consumed one action at a time | `crates/tam-storage/src/jobs.rs:2125`, `crates/tam-engine/src/driver.rs:389` | B |
| 7 | Org-wide and fleet-wide halts are reachable from the effect loop, which classifies no effect | `crates/tam-engine/src/driver.rs:832`, `:834` | B |
| 8 | Claim and park expiry are computed from the claimant's clock and read against the reaper's | `crates/tam-storage/src/jobs.rs:799`, `:1053`, `:1125` | B |
| 9 | The lease candidate scan has no transport-class predicate, so both branches draw one queue | `crates/tam-storage/src/jobs.rs:802` | B |
| 10 | The per-organisation live-lease mutex caps a seller at one working device and serialises the branches | `crates/tam-storage/migrations/0008_tenant_mutex.sql:8` | B |
| 11 | The interpreter holds a raw `PgPool` for the event journal and the notification outbox | `crates/tam-engine/src/driver.rs:87`, `:1262`, `:1358` | B |
| 12 | `AssertFormSchema` is write-bearing on Tes, first in every create run, and consumes no rate grant | `crates/tam-engine/src/driver.rs:547`, `crates/tam-marketplace-tes/src/flows.rs:590` | B |
| 13 | `tam-canary` reads a raw Tes cookie jar off disk and runs a write-bearing probe on a server timer | `crates/tam-canary/src/main.rs:65` | B |
| 14 | Server processes still originate scheduled no-API requests, and the keep-list contradicts D1 | `crates/tam-analytics/src/main.rs:102`, `crates/tam-sync-worker/src/main.rs:182` | B |
| 15 | The exclusivity identity read becomes client-asserted, and its pepper dies with the vault | `crates/tam-engine/src/broker_client.rs:177`, `crates/tam-secrets/src/lib.rs:422` | B |
| 16 | The only fence against a duplicate listing is a Postgres partial unique index the driver must reach | `crates/tam-storage/migrations/0005_job_ledger.sql:109` | B |
| 17 | No ledger vocabulary type derives serde, and `EngineError` transitively contains `sqlx::Error` | `crates/tam-storage/src/jobs.rs:99`, `crates/tam-storage/src/lib.rs:80` | S |
| 18 | Tenancy across the whole engine path is a caller-supplied argument under BYPASSRLS | `crates/tam-engine/src/driver.rs:498` | S |
| 19 | The state write and its event are separate transactions on three paths | `crates/tam-engine/src/driver.rs:919`, `:1262` | S |
| 20 | Every job event takes a `FOR UPDATE` lock on a per-organisation counter row | `crates/tam-storage/src/jobs.rs:367` | S |
| 21 | Every ledger timestamp the driver writes comes from the local `NowSource` | `crates/tam-engine/src/driver.rs:382` | S |
| 22 | Admission is checked once in `prepare_item` and never re-checked when the attempt opens | `crates/tam-storage/src/jobs.rs:1481`, `crates/tam-engine/src/seed.rs:197` | S |
| 23 | A create parked on `ReauthRequired` is reachable by no reaper and stays parked | `crates/tam-storage/src/jobs.rs:1209` | S |
| 24 | The claim TTL is a fixed 300 seconds with no renewal primitive, against a marginal budget | `crates/tam-worker/src/main.rs:81` | S |
| 25 | Nothing emits `ItemLeased`, so progress cannot name the device doing the work | `crates/tam-types/src/lib.rs:716` | S |
| 26 | The authorship attestation reaches no ledger row, and its only writer is in the deleted crate | `crates/tam-engine/src/driver.rs:168`, `crates/tam-session-broker/src/vault.rs:234` | S |
| 27 | `TransportClass` is decorative: nothing fails the build when a no-API marketplace gains a server transport | `crates/tam-types/src/lib.rs:125` | S |
| 28 | No cross-compilation gate covers the split crate, so the portability claim stays unfalsifiable | `justfile:59` | S |
| 29 | `LeaseRepo` and `HaltRepo` call inventory, and only one halt scope is tenant-scoped | `crates/tam-engine/src/driver.rs:82`, `:84` | D |
| 30 | `WriteAttemptRepo::settle` carries the mapping bind and must never be decomposed across the wire | `crates/tam-engine/src/driver.rs:85` | D |
| 31 | Raw-pool event inventory, fourteen reachable sites; `outbox.rs` is entirely server-side | `crates/tam-engine/src/driver.rs:1262`, `crates/tam-engine/src/outbox.rs:66` | D |
| 32 | `seed.rs` inventory: eleven calls, including a whole-table taxonomy read per item | `crates/tam-engine/src/seed.rs:244` | D |
| 33 | `acquire` and `expire_and_steal` are outside the driver's seam and will be missed by its inventory | `crates/tam-storage/src/jobs.rs:793`, `:1040` | D |
| 34 | The lease TTL against the driver's wall-clock budget is already marginal on TPT | `crates/tam-engine/src/driver.rs:503` | D |
| 35 | Delivery class is already decided per operation by the effect vocabulary | `crates/tam-domain/src/lib.rs:631` | D |
| 36 | The counterpart park give-up now spans the branch boundary and charges an offline seller | `crates/tam-storage/src/jobs.rs:1119` | D |
| 37 | The entitlement predicate belongs in the candidate CTE beside the halts | `crates/tam-storage/src/jobs.rs:807` | D |
| 38 | The fleet breaker has no distinct-tenant guard, so one bad uplink can halt an inventory | `crates/tam-engine/src/breaker.rs:35` | D |
| 39 | `broker_client.rs` cannot be deleted whole: the exclusivity claim survives inside it | `crates/tam-engine/src/broker_client.rs:177` | D |
| 40 | Dead custody scaffolding compiles into every client build | `crates/tam-marketplace/src/lib.rs:711` | D |
| 41 | One item costs ten to twenty independent ledger round trips, each its own transaction | `crates/tam-engine/src/driver.rs:390` | D |
| 42 | `tam-engine` is five unrelated concerns sharing a pool; split the crate before the trait | `crates/tam-engine/src/lib.rs:9` | D |
| 43 | Every `MarketplaceAdapter` method takes an org id no implementation reads | `crates/tam-marketplace/src/lib.rs:524` | D |
| 44 | `Effect::Reconcile` only ever synthesises a refusal, and its absence halts an ambiguous create | `crates/tam-engine/src/driver.rs:753` | D |
| 45 | `prepare_item` and `seed_from_projection` are already the server and device halves | `crates/tam-engine/src/seed.rs:9` | D |
| 46 | Where the entitlement gate sits in the loop, and why one check is not enough | `crates/tam-engine/src/driver.rs:381` | D |
| 47 | The gateway route allow-lists become signed client data with refusals reported as ledger events | `crates/tam-session-broker/src/gateway.rs:46` | D |
| 48 | Device-side ledger writes would be attributed to `engine` and dated by the seller's clock | `crates/tam-types/src/actor.rs:19` | D |
| 49 | `tam-pipeline` cannot cross-compile today, which gates D27's client-side ingest | `justfile:55` | D |
| 50 | The worker's docstring claims a jittered poll the code does not implement | `crates/tam-worker/src/main.rs:3` | N |
| 51 | The reauth notification dedupe key is per item and event, so a seller is told once and never again | `crates/tam-engine/src/driver.rs:1365` | N |
| 52 | A compile-time no-op exists only to silence an unused-import warning | `crates/tam-engine/src/driver.rs:1376` | N |

Thirteen findings were refuted under adversarial verification and are recorded so they are not re-raised.

- The interpreter can issue cross-tenant effects: refuted, because `halt_scope()` only ever produces `OrgInventory`, so the wide arms are unreachable today and the fix is deleting them rather than partitioning the enum.
- `prepare_item` reads the tenant's whole taxonomy: refuted, because `canonical_term`, `projection_edge` and `projection_no_counterpart` are global reference data with no tenant dimension.
- Re-offering presumed-dead work as a new job defeats idempotency: refuted, because no code path mints a job for stalled work, and `job_request_idempotent` already fences the one that could.
- Every expired claim is charged against the attempt budget: refuted, because the counter is a lease budget by construction and suppressing the charge removes the only terminator of the `AttemptInFlight` loop.
- `OutboxRepo::claim_due` claims nothing: refuted, because at-least-once delivery to an idempotent party is the specified contract and the drainer is a single non-templated unit.
- The raw pool defeats any repository trait: refuted as a separate finding, because the four repositories are themselves `PgPool` wrappers and the pool is one member of a homogeneous set; kept as row 11.
- The lease epoch fence is observed rather than enforced: refuted, because server-side compare-and-set with the client observing the verdict is the only sound shape for an untrusted caller.
- Two settle operations hide a second write: refuted, because both are already single methods whose return values surface the second write, and the compound is atomic by necessity.
- `RunVerdict::Abandoned` carries a String: refuted, because the proposed enum cannot derive `Eq` through `EngineError` and would break the engine tests the gate requires unchanged.
- Cancellation is not modelled as a capability: refuted, because `is_cancelled` is an atomic read needing no runtime, and `tokio-util` is not the crate's blocking dependency.
- The attestation's only write path is inside the deleted crate: refuted as stated, because the described `UploadRejected` failure is unreachable and the refusing host is deleted in the same stroke; the durable-record half is kept as row 26.
- TPT's bucket hops bypass the broker: refuted, because Tes has the same leg, no credential of ours rides it, and the omission is a documented protection.
- The gateway composes the CSRF mirror: refuted, because `TptSession` derives and mirrors the token inside the adapter already, and the gateway's copy exists only for the jar-less transport.
