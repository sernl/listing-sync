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

Implemented 2026-09-03 as the interim, and it is interim on purpose.
D27's target is client-side ingest, where the seller's own machine holds the bytes and the server never has them.
Until that exists the bytes are ours: the work order carries a `PayloadManifest` per file — id, name, content type, blake3 hash, byte length — and the device fetches the bytes from `GET /v1/devices/{device}/payload/{file}`, then checks what arrived against the manifest before it uploads anything.
The route is guarded three ways: the device must hold a live lease, the item that lease names must reference the file in its projection, and the file must be the organisation's.
The response carries the bytes and nothing else, because the manifest is the commitment and a second copy travelling beside the bytes would be a second thing to disagree with.

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

Implemented 2026-09-03, and the line is worth stating plainly for the founder: entitlement here is not "has paid".
The predicate admits a device that is registered and unrevoked, and it blocks on the plan only when a subscription lapsed and stayed lapsed past the grace.
An organisation that never subscribed has no billing row and is never blocked by it, because the Free tier is entitled within its own quotas exactly as `tam-limits` already grants them; a card that failed this morning still syncs this morning.
The three tests that pin those edges are `a_revoked_device_claims_nothing`, `a_free_tier_organisation_still_claims` and `a_plan_lapsed_past_the_grace_claims_nothing` in `crates/tam-storage/tests/leases.rs`.

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

### Phase 1b

Phase 1 closed at step 9, and the work continued past the plan above, so steps 9 to 11 are recorded after the fact from the landed code and its tests rather than proposed.
Steps 12 to 14 are what remains.

Step 9, narrow the halt scope.
`HaltScope` becomes a single-shape struct, so the effect vocabulary cannot name a scope wider than the lease that produced it, and `breaker.rs` and `canary.rs` stay the only writers of a fleet halt.
Proves finding 7 is closed by deletion rather than by a guard a relocated interpreter could route around, which is what open question 6 recommended.
Verification: the engine suite and the gauntlet unchanged in outcome, with the two wide arms absent from the match rather than merely unreachable within it.
Kill gate: a caller outside the breaker and the canary needing a scope wider than its own inventory.

Step 10, one wire vocabulary.
Put serde on the domain and marketplace ids, on the driver vocabulary and on the work and settle envelopes, and have the desktop's `work.rs` serialise those shared types rather than a parallel set of its own.
Proves the ports' types are the wire types, so the Phase 2 contract cannot drift from the trait Phase 1 extracted.
Verification: `crates/tam-engine-driver/tests/wire.rs`, round-tripping a full work envelope, a full settle envelope and every ledger answer.
Kill gate: a vocabulary type that cannot derive serde without dragging storage across the boundary, which is finding 17's `EngineError` reaching `sqlx::Error`.

Step 10a, the envelope carries the preparation.
The work order carries the preparation and the payload manifests and never the seed, `seed_from_projection` and `seed_for_removal` move into the driver crate, one shared preparation constructor serves both the claim endpoint and the worker, and `describe_files` states the manifests.
Proves the device composes and the server does not: the intent hash is computed where the field set is rendered, which is what keeps the architecture off S3.
Verification: both hosts constructing the preparation through the one constructor, and `just check-portable` compiling the driver crate with the seed inside it for all five client triples.
Kill gate: an envelope field that cannot be computed without composing a marketplace request.

Step 10b, the ledger call as data, and the endpoint that dispatches it.
`LedgerCall` and `LedgerAnswer` become wire enums covering all twelve calls, a device-asserted instant is recorded beside the server's receipt under migration `0045_device_asserted_instants.sql`, the driver crate takes a direct `blake3` edge for its digests, and the device ledger endpoint dispatches every call under the settle fences with the payload route guarded by live lease, projection reference and organisation.
Proves finding 48: a device-originated row records the seller's assertion and our receipt as two facts rather than conflating them, while `org_seq` still orders.
Verification: `every_ledger_call_and_answer_round_trips` and `the_asserted_instant_travels_with_every_call_that_records_one` in `wire.rs`, with the Phase 2 desktop host as the first consumer, running the interpreter against a fake control plane end to end.
Kill gate: a call whose answer cannot be represented without a storage type, or a dispatch arm that trusts an organisation the caller named.

The filter step, the pull honours a marketplace filter.
`WorkFilter` on the request and `ClaimPolicy` on the claim statement, so held-by-another-device is scoped to the marketplace that was asked for rather than to the organisation.
Proves step 7's per-connection mutex re-scope reaches the pull: a seller running TPT on one device and Tes on another has neither device told the queue is empty because the other holds a lease.
Verification: `the_work_route_accepts_a_marketplace_filter_and_serves_without_one` in the api fixture, and the desktop scheduler pulling once per marketplace it holds a session for.
Kill gate: a filter honoured as given rather than intersected with what the lease and the entitlement already allow.

Step 11, renewable device leases.
`LedgerCall::Renew` carries the device's asserted instant, `HttpLedger::renew` calls it from the desktop, the `RunGate` deadline moves only by the expiry the server states, a refused renew stops the run before the next request, and the reaper reclaims only leases that stopped heartbeating rather than ones that are merely slow.
The wire vocabulary was made uniformly snake_case in the same step.
Proves finding 24 against a closed lid: a device that is working keeps its lease and a device that went away loses it, decided by the heartbeat rather than by a fixed 300 seconds.
Verification: `the_reaper_reclaims_a_lease_that_stopped_heartbeating_and_not_one_that_did_not` and `a_renew_against_a_bumped_epoch_is_refused` in `crates/tam-storage/tests/leases.rs`; `a_renewed_run_outlives_the_deadline_the_claim_gave_it` and `a_late_or_smaller_renewal_cannot_cut_a_run_short` in `apps/desktop/src-tauri/src/run.rs`; `a_renewal_moves_the_gate_by_the_servers_own_numbers` and `a_refused_renewal_stops_the_run_before_the_next_request` in `apps/desktop/src-tauri/src/ledger.rs`; and the api fixture `crates/tam-api/tests/devices_flow.rs` over the whole device surface.
Kill gate: a renewal a device can extend by its own arithmetic.

The fixture is the step's own finding, because writing it exposed three defects every test before it had missed.
`POST /v1/devices/{device}/work` leaked the lease when preparation returned `Blocked`, leaving an item claimed by a device that had just been told there was nothing to do; the path now calls `LeaseRepo::release`.
Unpinned reads under forced row-level security made the epoch fences refuse everyone, so `a_settle_from_a_device_other_than_the_holder_is_refused` had been green for the wrong reason: the refusal it asserted came from RLS hiding the row, not from the fence rejecting the epoch.
Unpinned writes made the whole device write path inert, fixed by pinning the tenant with `pin_tenant` on the twelve write paths.
The second generalises: a negative test under forced RLS proves nothing until the row it expects to see refused is one the connection can see.

The heartbeat narrows the stolen-lease anomaly rather than removing it.
`a_stolen_lease_is_caught_at_the_heartbeat_before_any_request` in `crates/tam-engine/tests/gauntlet.rs` and `a_stale_worker_is_fenced_after_a_steal` in `crates/tam-storage/tests/leases.rs` pin what holds: a device whose lease was stolen learns it at its next renew and stops, and anything it had already issued is still fenced at the ledger, but the window between the steal and the next heartbeat is bounded by the heartbeat interval rather than closed.

Step 11a, what a review of step 11 found.

An independent review of step 11 raised twelve findings, one of them blocking, and this records their disposition.

The blocking one: `LedgerCall::Renew` carried `ttl_seconds` from the device straight into `make_interval`, where an `i32` saturation turned `i64::MAX` into an expiry sixty-eight years out — a lease no reaper reclaims, holding its organisation's marketplace mutex for good.
The duration leaves the wire entirely rather than being clamped: the server mints the expiry from `LEASE_TTL_SECS`, and `deny_unknown_fields` on `LedgerCall` refuses a body that still names one.
`park_for_seconds` was unbounded in the same shape and takes the same fix, because the machine parks every gate for `PARK_TTL_MS` and the party naming the span was the party being parked.

Five were correctness fixes to what step 11 landed.
Every failure exit from `/work` after the claim now releases the lease it took, structured as one release so a new early return cannot forget.
`LeaseRepo::release` raises `StaleLease` on a zero-row update, like every other fenced write there.
The settle regained the epoch half of its fence, which it had been carrying and not reading.
The renew moved ahead of `AssertFormSchema`, the create path's first marketplace request and one that creates a probe draft on the seller's own Tes account.
And the run gate's elapsed term became observable rather than merely present.

Three were consolidations.
One `LEASE_TTL_SECS` in `tam-domain` replaces three copies of 300 and the source-scraping mirror that guarded one of them; both of a renew's instants come out of one statement rather than one from Postgres and one from the API process; and `LeaseRepo::held_by` is deleted for want of a caller.

Proves that the fixture found what a fixture finds and a reading found the rest: the fixture caught three defects by driving the path, and five more were reachable only by a caller the tests did not have.
Verification: `just pre-push` and `just check-portable`, with a test per fix that fails against the code as it stood — the storage renew asserting the minted span against a claim that asked for five seconds, a wire test refusing a renew that names a duration, `a_stolen_lease_on_a_create_is_caught_before_the_form_scrape` asserting no probe draft was left behind, `a_work_route_that_cannot_prepare_hands_the_lease_back`, `a_settle_naming_an_epoch_the_item_has_moved_past_is_refused`, and `every_write_this_endpoint_serves_lands_under_the_tenant_pin`, which is the general form of the pin defect step 11 found one instance of.
Kill gate: a device-reachable write that cannot be proved to land under `tam_app` with the fixture the api tests already build.

Step 11b, what verifying 11a found.

Verification confirmed the twelve dispositions and raised one more severe defect, which is recorded because it was invisible for the same reason the step-11 pin defect was: it needed a caller the tests did not have.

The claim route released a refused preparation back to the queue.
`claim_for_device` orders by `created_at` and a release advances nothing, so an item the preparation would never admit — a create against a bound mapping, a counterpart that will never bind — was claimed on every poll, prepared, released and claimed again without bound, holding every sibling on that marketplace behind the per-connection mutex the whole time.
The worker never had the defect, because it parks or settles such an item instead; the route had grown its own arm.
The fix is one disposition both hosts call, `prepare_and_dispose` in `tam-engine`, so there is no second arm to diverge: a blocked item parks under its gate for `PARK_TTL_MS`, a lost counterpart settles skipped, and nothing reaches the queue from either.
Five smaller items landed with it: the heartbeat's asserted instant was removed again, because a renew writes no dated row and the wire test states that a call carries an instant exactly when it writes one; the pin rationale that had been pasted eleven times moved to `pin_org` itself; `pin_tenant` and `pin_org` each got their own doc; and two overstated claims were corrected.

Proves the arm both hosts share is one arm.
Verification: `a_lost_counterpart_settles_through_the_work_route_and_the_queue_moves_on` and `a_blocked_item_is_parked_through_the_work_route_and_the_queue_moves_on`, each asserting the ledger state the disposition wrote and the second asserting the next poll is not served the same item again.
Kill gate: a disposition either host needs that the other must not have.

Step 11c, the preparation failure the release left open.

Step 11b's fix closed the livelock for an item the preparation refuses, and left the one for an item whose preparation fails.
The route released a failed preparation uncharged, which for a transient fault is right and for a deterministic one is the same unbounded loop under a different name; holding the lease to expiry instead would strand every sibling on that marketplace for the whole TTL.
Neither is the answer, so the route now charges the attempt: an item with budget left is requeued and retries on the next poll, and one without settles failed.

`LeaseRepo::charge_and_requeue` is the reaper's disposition applied to one named item, epoch-fenced and tenant-pinned because the route reaches it under `tam_app`, calling `settle_if_complete` on the settle arm for the reason the reaper does — an item settling there may be the last its job was waiting on.
The threshold cannot be one spelling, and three places is the fewest it can occupy: two in SQL, in `expire_and_steal` and `revive_expired`, because a cross-tenant set-based scan cannot call into Rust per row, and one in Rust, `tam_domain::attempt_budget_spent`, which both the charge and the driver's preflight give-up call.
`the_reaper_and_the_charge_agree_at_the_boundary` is what keeps the SQL honest against the Rust rather than a comment asking the next editor to.

Proves that a failure the route cannot see through terminates, whichever kind it is.
Verification: `a_preparation_that_never_succeeds_settles_failed_once_the_budget_is_spent`, which polls the budget through and asserts the requeue each time and the settle on the last; `a_transient_preparation_failure_costs_one_attempt_and_the_next_poll_claims_it_again`; the boundary test above; and `a_charge_from_a_run_that_no_longer_holds_the_item_is_refused`, because a charge that ignored the epoch would cost a stolen item two attempts.
Kill gate: a preparation failure that is neither transient nor deterministic — one whose recovery depends on something the attempt budget cannot count.

One residual was recorded rather than fixed, and step 11c closed it.
The interpreter renews before every network-bearing effect, so a submit begins with a full lease, and Tpt's theoretical worst case outlived one: two queue-job polls at `QUEUE_POLL_MAX` inside a single `submit` can spend 360 seconds with no renew between them, against what was then a 300-second lease.
The two remedies went to the founder as question 7 and the answer was the lease, raised to 600 seconds on 2026-09-03, rather than a clock port inside the adapter seam.
The assertion that recorded the defect now records the guarantee, on the same measured numbers.

Step 12, the three open defects.
Widen the operator view to surface `state = 'in_flight'` past the lease TTL so a stranded in-flight create attempt reaches an operator instead of staying mapping-scoped and invisible (finding 4), and add the read-back reconciliation the code names for itself if it fits inside the step, otherwise land the view alone and record the reconciliation as still owed.
Add the binding predicate to `open`'s statement for a create and return a distinct refusal the driver settles as skipped, so a resumed run and a stolen lease cannot both create (finding 22).
Give a create parked on `ReauthRequired` a give-up keyed on park age, because `REVIVABLE_GATES` excludes that gate and `attempt_count` never advances, so no attempt-budget give-up can fire (finding 23).
Proves each of the three stalls that a closed lid makes common rather than rare is terminated by something.
Verification: revert-run-restore for each, so every fix has a test that fails on the reverted code.
Kill gate: a give-up that settles an item the seller could still have revived by signing in.

Step 12 landed as ruled, in three parts, and its own review found four more defects that landed with it.

The operator view widened: `failed_writes` surfaces a stranded attempt beside the failures, and `failure_code` became optional through `FailedWrite`, `FailedWriteView`, the admin route and its page.
Stranded is defined against the lease rather than the clock, and the first attempt at it was the clock: an attempt whose `lease_epoch` is behind its item's belonged to a run that has been superseded, and one whose item is no longer in a live state belonged to a run that has ended.
A heartbeat renews for as long as a device keeps working, so elapsed time says nothing at all — a healthy run holds its attempt open well past one TTL.
`a_stranded_attempt_reaches_an_operator_and_a_live_one_does_not` drives both disjuncts and asserts the ordering by index, because stranded rows are the oldest by construction and a newest-first page would push them off the end.

Admission is re-checked at `open`.
The operation and the mapping are read from `job_item` under the lease rather than taken from the caller, the mapping is read `FOR SHARE` inside `open`'s own transaction, and the refusal is its own answer, `LedgerError::MappingAlreadyBound`, which the interpreter settles skipped rather than abandoning: the mapping was bound after this run was admitted, so the listing exists and no later lease could make it again.
The check runs only when the insert wrote a row, because a replay of a caller-minted id is a device recovering a lost response, and the bind it would be refused for may be its own.
Six existing tests in `tam-storage::bind` had to change, and that is the finding rather than a side effect: each was modelling a second create against a mapping its own first landing had bound, which is the sequence this refuses.
Three of them model a change and are revises; three model a takedown and are removals.

A create parked on `ReauthRequired` past its park age leaves the park for `awaiting_seller_signin` and nothing else moves.
The attempt stays in flight, the mapping stays fenced, no attempt is charged and nothing settles, because only a read-back under the seller's own session can decide whether the listing exists — that is step 12b, and until it lands the gate is a better label on the same stranded item rather than a remedy.
`REVIVABLE_GATES` and its two guards are untouched.
The transition is recorded as `ItemGateChanged`, a new payload variant, because `ItemParked` says a park began and when it ends and neither is true here.
The gate vocabulary reaches the client through typegen as `BlockedGate`, so the console's label map is `Record<BlockedGate, string>` and a gate added in Rust without a label fails `svelte-check`.

Four defects the review of this step found, all fixed here.
`WriteAttemptRepo::open` was taking the mapping before the attempt row while `settle` takes them the other way round, which is a deadlock with no retry and a create that may already have landed as its victim; the module now states one lock order — `job_item`, then `write_attempt`, then `mapping` — names every method it binds, and `settle` takes the item row first to obey it.
The park exit was recording a second park with an elapsed expiry and no gate.
`revive_expired` was summing relabelled parks into its revival count, so a pass that revived nothing read as though it had.
And the admission re-check was firing on the idempotent replay path.

Proves the three stalls are each terminated by something a seller or an operator can see, or in the third case named for the step that will terminate it.
Verification: revert-run-restore on each of the three; the label map proved exhaustive by deleting one entry and watching the type check fail; and the lock order proved by a forced overlap, a test-held transaction taking `settle`'s locks in `settle`'s order and waiting until the real `open` is observably blocked before reaching for the mapping.
Kill gate: a gate reaching `blocked_on` that `ALL_GATES` does not name — detectable, because `the_gate_vocabulary_covers_every_gate_the_tree_writes` fails on a one-sided addition and the ledger refuses a device-named gate outside the vocabulary.

Two things are recorded rather than fixed.
The read-back reconciliation that finding 4 named is owed to 12b.
And `open` compares no lease epoch in its own statement: it is fenced by `held_by` at the API, which is the same pre-existing gap the stolen-lease settle has, and closing both is one change rather than two.

`tam-storage` gained one dev-dependency edge in the process, `tokio` at the version and features `tam-api`'s tests already use, granted because a deadlock needs two transactions and the lock order the module documents cannot be proved with one; sqlx already brings that runtime, so it is an edge in the graph rather than a crate in it.

One process note for whoever does the next revert-run-restore here: reverting a change that touches SQL leaves the committed query cache holding the reverted statement, so `just db-prepare` has to run again after restoring.
It costs one red `check` that looks exactly like a real failure.

Step 12b, the read-back reconciliation.

The reconciliation finding 4 named, and the remedy `awaiting_seller_signin` was standing in for.
A create whose fate the ledger cannot determine is settled on what is actually on the marketplace, read under the seller's own session on the seller's own device.

The selection is at claim time rather than by an enqueue.
A stranded create — parked on `awaiting_seller_signin` with its attempt still in flight — is a candidate the claim admits, ranked ahead of ordinary queued work because each one holds a mapping's fence and clearing it unblocks everything behind it.
That needed no marker column, no bound and no new limit: the lease is the idempotency, and one item per claim is the bound by construction.
The claim also gained the predicate it turned out never to have had — a `device_marketplace_session` at `status = 'connected'` for the job's marketplace — so a device claims only work it holds a session for, which closes a pre-existing D14 mis-routing the reconcile path exposed rather than created.

The interpreter reaches the search through a new input rather than a second constructor.
`Input::ResumeStranded` adopts the standing attempt and steps to the state the ambiguous-submit row reaches, so the transition table stays the whole specification of the machine; the entry row's effects are discarded rather than run, because the first of them is the form assertion and on Tes that is a write.
`ReconcileSource` is a port the device serves, never the server: D1 puts every request to a no-API marketplace on the seller's machine and an enumeration is a request, so `tam-worker` implements it as a refusal that says so.

Two outcomes, not three.
Found links the listing and settles succeeded, with a verifying read-back behind it.
Everything else — a complete enumeration that did not contain it, or a read that could not be performed — stays stranded and surfaced, because a negative search is not evidence a create did not land, and releasing the duplicate-create fence on one is the failure this ledger cannot undo.
The two are recorded as distinct causes even so, `NoDurableIdentifier` against `ReadBackIndeterminate`, which is what would let the founder's answer to question 8 change one without changing the other.

The enumeration gate on both adapters now admits `FetchReason::VerifyAttempt` beside `FirstPartyExport`, because a reconcile is not an export and saying it was would have put a read in the code whose stated justification was not its real one.
The machine already reads back under `VerifyAttempt` for the operations that have a durable subject, so this makes the create's path consistent with the ones beside it.
It is the only non-export caller, and a third is a decision rather than a precedent.

The reaper gained a third arm, ahead of the settle and the steal.
A reaped lease whose item is a create, whose mapping is unbound and whose attempt is still in flight goes back to the park rather than to the queue, charged nothing.
Not reconcile-specific: an ordinary create whose device died between issuing the request and its read-back is the same shape, and today it is stolen back only to abandon on the fence it is itself holding until its budget settles it failed with nothing in the ledger — the second defect the reconcile path closed.

Proves that a create whose fate is unknown is decided by evidence or left visibly undecided, and never by a guess.
Verification: the two outcomes driven end to end against the in-memory ledger, the wire field's presence and absence, the adapters refusing every other fetch reason, and the claim and reaper arms driven against Postgres.
Carried forward from the 12b review rather than fixed in it: `expire_and_steal` still answers one scalar covering the settle, the steal and the park, so a caller cannot tell a parked item from a stolen one.
The worker's line was corrected to say reaped rather than stole, which makes the number true; the structured answer `revive_expired` already returns is the better shape and belongs with whatever next needs the breakdown.

One half of the found rule is stated in `docs/design/sync-machine.md` without a test behind it: a read that differs in one field settles degraded, and no run can reach `Outcome::Degraded` today because `settle` classifies from a `FieldDiffReport` and `unnormalised_report` is hard-coded empty until M1g writes the normaliser.
The sibling test belongs to M1g with the comparison it is about, and until then the rule binds the code that will be written rather than code that exists.
Kill gate: a path that releases the duplicate-create fence on anything other than a positive identification.

`docs/design/sync-machine.md` gains the transition row and the new error in the same change, and its `MachineError` table was brought back to the code on 2026-09-03 — it had been three variants behind by one before this step added the second.

The reconcile is dormant in production, deliberately and by construction.
`preparation()` configures one create strategy for every inventory, and the marker's carrier is an open founder decision, so nothing writes a marker and a search would have nothing to look for.
Rather than leave that implicit, the strategy became a single `CREATE_STRATEGY` constant in `crates/tam-engine/src/seed.rs` and `reconcile_is_available()` is derived from it by a `matches!`, so the commit that answers the founder cannot configure `CorrelationMarker` without the claim beginning to admit stranded creates in the same edit.
The flag travels on `ClaimPolicy` into the stranded arm of the claim, and while it is false a stranded create is left exactly where the reaper put it: parked, charged nothing, its attempt still fencing its mapping, and reconcilable by the build that can.

The second half is why a dormant path cannot halt an inventory.
Every other ambiguity row halts the tenant's inventory and is right to, because an ambiguity there means that tenant's automation has stopped being safe to continue; the unsearchable resume means only that this build configures no searchable strategy, which is equally true of every item in the queue and is not a reason to stop it.
That arm now settles the item ambiguous with one `Notify` and no `Halt`, leaving the attempt standing.
It is unreachable while the claim gate holds, and that is the point: the gate is the policy and the arm is the bound on being wrong about it, so a resume delivered by any route the gate does not cover costs one item rather than a tenant's queue and an operator's intervention.
`SellerEvent` has no variant for an item that ended ambiguous, so the arm reuses `ItemParked`, the closed vocabulary's "an item stopped, come and look" signal; a dedicated variant is a client-vocabulary change and belongs with whoever owns `web/`.

Step 12d, the identification a create is found by, without a marker.

The founder was asked whether a correlation marker may sit in a seller-visible field and the recommendation was that it need not be.
`docs/design/decisions.md` already records that Tes does not need the marker because draft-then-publish is idempotent there, and neither catalogue enumeration returns a description, so the only field `MarkerField` permits that a walk can see is the title — the one TPT's Seller Guidelines argue against.
The marker-free route was adopted and is what this step built: nothing is written into any listing, and no listing a buyer reads is touched.

A stranded create is identified by the title its own attempt recorded that it sent.
`intent_as_json` writes the rendered field set into the `write_attempt` row before the click, so the title is already there; the claim reads it out of that intent and it travels to the device on the work order beside the attempt.
It is deliberately not the title the product carries now, which is what the machine's own `fields` hold: the two differ whenever the seller edited the product between the strand and the resume, and searching a catalogue for the current title is not merely how a search misses — it is how it matches some other listing that has since acquired that title and binds the mapping to it.
The claim returns the attempt only where its intent names a title, so an attempt that identifies nothing is not offered as a reconcile at all.

The search is exact and narrowed, and the narrowing is what makes a title usable.
A marker is unique; a title is not, so matching one is only an identification when exactly one candidate survives.
TPT's walk exposes `name` and `status` and Tes's exposes `title` and `published`, so a candidate must additionally be in the state a draft-then-publish create leaves a listing in: unpublished on Tes, `NOT_ACTIVE` on TPT, with a status TPT has never stated classifying as neither and therefore not a candidate.
Exactly one candidate binds, zero answers a completed-and-absent `Ok(None)`, and several answers the indeterminate `Err` — because two listings answering to one title is not absence, and `Ok(None)` is the one answer anything could ever build a fence release on.

The residual risk, stated rather than left implicit.
The narrowing excludes every listing the seller has already published, which is the collision most likely to happen; two of the seller's own drafts sharing a title is two candidates and therefore ambiguous and safe.
What remains is the case where the create did not land and the seller has exactly one other unpublished listing carrying the identical recorded title: the walk finds one candidate and binds the wrong listing.
That is the price of identifying by a field that is not unique, it is the price the marker was going to buy out, and it is why the marker option stays in `CreateStrategy` rather than being deleted.

`reconcile_is_available` widened accordingly, from "the strategy embeds a marker" to "the strategy leaves a create this walk can identify", which `DraftThenPublish` satisfies and `HaltOnAmbiguity` does not.
That flips the claim gate in the same edit, so the reconcile stops being dormant here rather than in a separate change.
`ProjectedListing` gained nothing and `preparation()` gained nothing, which is the whole point of the route.

Deliberately not extended to the ambiguous submit, which is the obvious next step and is not this one.
`SyncMachine::reconcile` still searches only under `CorrelationMarker` and halts the tenant's inventory under `DraftThenPublish`, so a submit whose response was lost mid-run is treated as it was before this step.
The identification now exists for it — during a live run the machine's own fields are the recorded intent, because it is the same run that rendered them, so no wire field would be needed — but an ambiguous submit is a different signal from a device that stopped, and whether it still warrants the halt is a question of its own rather than a consequence of this one.

Proves that a stranded create is identified by what it recorded it sent, or left visibly undecided, and never by what the product says now.
Verification: the three outcomes on each adapter and both narrowings driven against recorded catalogues, the machine's two searchable strategies and its unsearchable one driven as transition rows, the conformance bodies driving the production strategy rather than a marker, and the claim's title extraction driven against Postgres including the attempt that names none.
One hop is not covered end to end and is named rather than counted as closed: nothing drives `/v1/devices/{id}/work` against a genuinely stranded create and observes the order that comes back.
`reconcile_subject` is unit-tested either side of the both-or-neither rule and the claim is driven against Postgres, so the two ends are proven and the wire between them is proven by reading.
The obstacle is the fixture rather than the code: `seed_claimable` in `crates/tam-api/tests/devices_flow.rs` builds a product that raises a taxonomy election, so `/work` parks on the election gate and answers idle, and no test in that file has ever received a work order.
Teaching that fixture to resolve an election is the task that closes this, and it is worth doing for every future test of that route rather than for this one.
Kill gate: an identification that binds on anything other than exactly one candidate in the state a create leaves.

Step 13, the entitlement gate inside the loop.
`AssertFormSchema` consumes a rate grant (finding 12), and the per-effect entitlement check sits at the four network-bearing effects and in `verify_with_backoff`'s per-try preamble, producing the shape `BudgetGrant::Exhausted` already produces rather than a new terminal outcome.
Two additions ruled during step 11 belong here.
Revocation before any request has been issued must leave the item requeue-able rather than settling `Outcome::Skipped`, which is what `exhaust_budget` does today from `AwaitingPreflight`, `PreflightAsserted` and `Parked` (`crates/tam-domain/src/lib.rs:1198`).
And the desktop test pinning the current behaviour, `a_revocation_in_flight_stops_the_run_before_it_opens_an_attempt` in `apps/desktop/src-tauri/src/work.rs`, is flipped to assert the requeue-able outcome.
Proves a mid-run revocation never settles an item on evidence the run does not have, in either direction: not abandoned with a listing committed, and not skipped with nothing attempted.
Verification: a driver test per effect site revoking immediately before it, and the flipped desktop test failing against the unflipped machine.
Kill gate: a requeue with no terminator, which is the refuted finding's point that the lease budget is the only thing ending the `AttemptInFlight` loop.

Step 14, the preparation port and the attestation.
Put `Preparation` behind a trait with `prepare_item` implementing it on the server, completing the three-port surface section 3 names.
Carry `attested_by` and `attested_at` into `AttemptIntent.body` at `RecordIntent` so the `write_attempt` row is the immutable record of the attestation the write went out under, and leave `intent_hash` alone because it feeds the idempotency key (finding 26).
Proves the last concrete storage call on the driver's path is behind a port, and that the authorship attestation survives the deletion of the crate that writes it today.
Verification: the driver crate compiling with no concrete preparation type in scope, and a ledger test asserting the attestation reaches the `write_attempt` row and not the intent hash.
Kill gate: an attestation that changes the idempotency key, which would make a re-attested create a second listing.

Landed. The gate now sits at the four network-bearing effects in the shape `BudgetGrant::Exhausted` already produces, and `AssertFormSchema` consumes a `GrantKind::FormRead` where it previously went out against nobody's allowance — a seller's window under-counted by one request per create.

The revocation half was larger than the entry describes, and the per-effect check it names turned out to be dead code: the loop-top guard sees the same cancellation one iteration earlier, so a check at the effect can never fire.
The guard now splits by whether a request can have gone out, which is the distinction `Input::BudgetExhausted` does not make.
From `AwaitingPreflight` or `PreflightAsserted` the run abandons with nothing attempted and nothing settled, which is the requeue-able outcome this step was for.
From `IntentRecorded` the open attempt is settled abandoned and the run abandons, where before it settled the item `Ambiguous` — terminally, on the claim that a write may have landed when nothing was sent, so the seller's item ended and never ran again because one device's entitlement lapsed.
Nothing halted: `exhaust_budget` emits `CaptureDiagnostics` alone, and the inventory halt belongs to the ambiguous-submit row, which is a different situation reached a different way.
An earlier draft of this entry said the tenant's inventory halted, and it was wrong; the defect is one item lost per lapse rather than a tenant stopped, which is smaller and still worth the arm.
`exhaust_budget` is untouched, because refusing from the pre-attempt states would falsify `budget_exhaustion_is_terminal_in_one_transition`; the guard gives the invariant without breaking the property.

Kill gate, asserted rather than reasoned about: `a_plan_lapsed_past_the_grace_claims_nothing` covers the claim's subscription filter and `abandoning_without_attempting_is_bounded_by_the_budget` runs the claim-abandon-reap cycle to termination.
The terminator is not a final charge but the reaper's give-up arm, which takes an item whose next attempt would exceed the budget — so the last cycle settles rather than charging, and the test says so.
A cycle that charged nothing would repeat for ever, which is how the stranded-create park arm differs and why it needed its own bound.

Step 14, landed as two halves of unequal weight.

The attestation is the substantial one, and the entry understates it. Before this change no TPT write could leave a device at all: the worker built its adapter with the seller's declaration off the connection and the desktop built it with none, and the adapter refuses without one — correctly, since the copyright declaration is the seller's statement and not a constant a connector may make for them.
So the branch D1 requires every TPT request to originate from was the branch that could not make one, and nothing caught it because the desktop's tests drive a scripted adapter rather than the real one.
The attestation now travels on the work order as an optional field, absent for a marketplace that asks for none, and reaches both the adapter and the recorded intent.
It joins `AttemptIntent.body` and never the hash: the hash feeds the idempotency key, so folding an attestation into it would make a re-attested create a second listing for one product.

The third port was not built.
`Preparation` was to be justified by the driver compiling with no concrete storage type in scope, and `just purity` already asserts exactly that — no `tam-storage`, `sqlx`, `tokio` or `reqwest` in the interpreter's dependency tree — inside `just check`, so the proof exists and fails the build if it stops holding; the seam the trait would open is one section 3 deliberately keeps shut, which would leave it with one implementation for ever.

A founder item this step raises and does not answer: what a seller sees when their connection carries no attestation.
Today the run reaches the submit — preflight, `RecordIntent`, then the adapter's refusal — and that refusal is an `AdapterError::Rejected`, so the machine settles the attempt and the item `Failed`.
That is terminal. The item spends an attempt, ends, and is never retried, so adding the attestation afterwards does not bring it back: the seller declares authorship and the work still does not happen, with nothing saying why.
The earlier wording here said the item abandons, which would have left it for a later lease; it does not, and the difference is what raises this from a courtesy to something worth doing.
Recommended: park the item on a gate naming the declaration, so the seller is asked for the one thing that would unblock it, in the same shape a lapsed session is asked for. It is a product decision about how they are asked rather than an engineering one, which is why it is here rather than in the step.

Two observations from the engine review, both pre-existing and neither addressed here.
`Effect::Reconcile` draws no rate grant: `find_listing` enumerates the seller's catalogue, which is as much a marketplace request as the scrape now charged for, and it is unbilled.
And `tam-worker` still builds a server-side TPT adapter, which is the branch D1 says should not exist — the device now carries the attestation that lets it write, so the worker's TPT leg is interim rather than needed.

Step 14 does not complete the split.
The broker deletion, the exclusivity-claim lift with its pepper re-sited off the vault key, the four crate dispositions of open question 3, and D1's structural build-failing test are all held until the desktop runs the driver against a live marketplace.
The broker deletion carries a predecessor that was not visible when it was scoped: `crates/tam-session-broker/src/vault.rs` holds the only writer of `connection.state = 'linked'` anywhere in the tree, and the device claim's candidate filter requires a linked connection, so deleting the crate stops every device claiming until a device-reported link exists to replace it.
Step 15 built that writer, so the gate is now a test rather than an absence: the deletion lands when `a_device_and_a_declaration_are_the_whole_link_a_tpt_write_needs` is green with `vault.rs`'s writer removed.
It also closes a collision step 15 opened and could not close from its own side.
`vault.rs` links with `ON CONFLICT (org_id, id)` while `connection` also carries `CONSTRAINT connection_one_per_marketplace UNIQUE (org_id, marketplace)` from migration 0006, and step 15's two writers key on the marketplace and mint their own id, so a connection they created first makes a later broker link raise on the unique index the vault's conflict target does not cover.
The order that reaches it is device-first-then-broker, which is a seller who connects a device for a marketplace before ever linking it through the broker; a row the vault made first is found and updated by all three writers, so no existing seller is affected.
Refusing the declaration for a marketplace with an official API removes the permanent half, because Etsy is the one marketplace that stays on the broker branch for good; what is left is a transition window that ends when the vault's writer does.
Closing it sooner is one conflict target widened in `vault.rs`, which step 15 deliberately did not touch.

Step 15, the device-reported link and the seller's declaration.

The predecessor step 14 named, built: `connection` now has a writer outside the crate that is being deleted, so the deletion has something to be gated on rather than being blocked outright.

The link is derived rather than declared, and the heartbeat is where it happens.
A check-in already replaces what one device holds, and it now also derives `connection.state` for every marketplace that check-in could have moved: the ones the device named, and the ones it stopped naming, because dropping a marketplace from the report is how a device says the session is gone and a derivation over the named set alone would leave a connection linked on a session nothing reports.
The rule is existential over live devices — `linked` while at least one non-revoked device reports that marketplace `connected`, `needs_reauth` when the last one stops — which is what D14 asks for, since a seller signed in on their desktop should not be told to re-link because they signed out on the laptop.
A revoked device does not count, matching the claim's own device predicate: it has been asked to forget its sessions, so holding a connection open on what it still reports would keep a machine we have disowned in the loop.
Two states are never written.
`unlinked` is not, because it is the state of a connection no device has reported at all and the heartbeat only ever runs when one has.
`revoked` is not written over, for the reason `DeviceRepo::register` refuses to clear `revoked_at`: a revocation any machine could lift by restarting would stop being the seller's decision.
The transitions reach `connection_audit` as `linked` and `needs_reauth` under `Stamp::system(SystemComponent::Device, at)` with the detail `device-reported`, and only where a row actually moved, so a device beating every thirty seconds writes nothing.
The detail column is carrying the distinction the event vocabulary cannot: `ConnectionEvent::Linked` is documented as a credential being sealed, and nothing is sealed here.

The declaration is the other half, and it is deliberately not derived from anything.
The copyright declaration TPT's product form requires is the seller's own statement, so it is made by the seller, once, and stored against the marketplace connection rather than against a device — which is what makes it survive a machine being replaced, and is the contract the console's declaration screen builds against:

- `POST /{version}/connections/{marketplace}/authorship`, cookie-session authenticated like every other route on this surface.
- The `{marketplace}` segment is the marketplace's serde name, `Tes`, `Tpt` or `Etsy`, which is the same spelling the heartbeat body and the generated client vocabulary already use; anything else is `404` with `ResourceMissing`.
- The body is `{"name": "..."}` and nothing else. The name is trimmed, must be non-empty and is capped at 200 characters, and a violation is `422` naming the bound it applied.
- The organisation is taken from `OrgContext` and is not representable in the body, so a caller cannot declare authorship into another tenant.
- The instant is stamped server-side. That is not a departure from the broker's rule that the instant is the seller's: this request *is* the declaration, so our receipt of it is when the seller made it, and an instant off the body would let a caller date their own statement.
- The answer is `{"marketplace", "name", "attested_at"}`, the declaration as it now stands, so the screen renders what was stored rather than what was sent.
- Declaring is idempotent and re-declaring replaces. It is a fact about the seller rather than about the link, so it neither links nor unlinks: a marketplace with no connection row gets one in `unlinked`, and an existing row keeps whatever state it stands in, `revoked` included.
- A marketplace whose transport class is `OfficialApi` is refused `422`, in the shape the heartbeat already refuses one. Its automation runs server-side under a sanctioned token, no device composes a write for it, and nothing would read the declaration back.
- What the screen has to say, which the route cannot: an unattested TPT connection does not fail loudly. The run reaches the submit and the adapter refuses on the declaration, which settles the item `Failed` and terminal, so declaring afterwards does not bring it back. That is the founder item step 14 raised and did not answer, and it is unchanged by this step.

One defect this step found and fixed, which is the reason the decisive test would have failed even with a writer in place.
`ConnectionFactsRepo::authorship_for` read on a pool with no tenant pin, correctly for the cross-tenant lease scan it was written for, and step 14 then called it from the API path — where the pool is `tam_app`, which is neither superuser nor BYPASSRLS and reads `connection` under forced row-level security.
An unpinned read there matches nothing on a fresh pooled connection and raises on one whose transaction-local pin has reverted to the empty string, so until this commit no work order could carry an attestation at all, whatever wrote the row.
Nothing caught it because the desktop's tests drive a scripted adapter and no test had ever driven `/work` to a real order.
Both reads now run under `pin_org`, which is inert for `tam_engine` and correct for `tam_app`.

Proves that the link and the attestation a TPT write needs are both produced by what a seller and a device actually do, with nothing seeded.
Verification, all in `crates/tam-api/tests/devices_flow.rs`: `a_device_and_a_declaration_are_the_whole_link_a_tpt_write_needs` seeds no `connection` row, registers, polls `/work` and gets idle, checks in, declares, and asserts the order comes back carrying the seller's name at the declaration's own instant rather than the order's; `a_connection_is_linked_while_any_device_holds_it_and_gated_when_the_last_stops` drives the quantifier over two machines, which is the half a wrong implementation gets wrong; `a_marketplace_dropped_from_a_report_gates_the_connection_it_held` drives the delete arm; `a_declaration_outlives_the_device_that_was_registered_when_it_was_made` revokes the declaring machine, registers another and asserts the order still carries the declaration; `a_declaration_reaches_only_the_tenant_that_made_it` covers the tenancy and the sanctioned-marketplace refusal; and `a_check_in_never_lifts_a_revoked_connection` drives both arms against a revoked row.
That fixture is also what step 12d recorded as owed: `seed_tpt_claimable` projects cleanly where `seed_claimable` parks on a taxonomy election, so `/work` returns an order in this file for the first time.
Kill gate: a connection reaching `linked` on anything other than a live device reporting a connected session.

What this does not prove, stated rather than counted as closed.
It does not exercise the deletion. The gate recorded in step 14 stands unchanged — this test green with `crates/tam-session-broker/src/vault.rs`'s writer removed — and that is a later commit.
`vault.rs` is untouched here and the rows it wrote stay valid: both new writers key on `(org_id, marketplace)`, so a row the vault made is found and updated rather than duplicated.

Two things recorded rather than fixed.
The vault's conflict target does not cover the unique index these writers key on, so a connection they created first makes a later broker link raise rather than update; it is recorded with the deletion that closes it, above.
And `DeviceRepo::revoke` does not re-derive, so signing out the last connected device from the console leaves its connection reading `linked` until some device checks in.
Nothing is served on it — the claim independently requires a non-revoked device holding a connected session — so the cost is a stale word on the connections page rather than work going anywhere it should not.

## 8. Open questions for the founder

1. Restate the Phase 1 verification as port conformance over two ledger implementations. Recommended: yes, because the current wording is unrunnable and the phase would otherwise ship without evidence.
2. Re-scope the live-lease mutex from the organisation to the connection. Recommended: yes, because the form-token race it protects against is per marketplace session and D14 grants several devices; this changes `JOBS_PER_TENANT`, which is founder-gated.
3. Record a disposition for `tam-analytics`, `tam-sync-worker`'s Tes read leg, `tam-canary` and `tam-import`, and correct the client-side note's keep-list. Recommended: the analytics capture becomes a device-pulled read item, the Tes read leg moves to the device and the crate keeps its enqueue half, `tam-canary` becomes a founder-run desktop command, and `tam-import` is restricted to manifest bytes or routed the same way.
4. Enable `blake3`'s `pure` feature so `tam-pipeline` cross-compiles, and state whether the interim arrangement is server-held bytes streamed at upload time or full client-side ingest. Recommended: enable it in Phase 1 rather than discovering it in Phase 2, and state client-side ingest as the target with server-held bytes as the interim.
5. Decide whether the rate grant is issued in bulk at claim time and whether consumption moves into the transport seam. Recommended: bulk grant at claim time with reported consumption in the settle envelope, and move consumption into the seam, because the marketplace now sees the seller's own address.
6. Decide whether `HaltScope::Org` and `HaltScope::FleetInventory` leave the effect vocabulary. Recommended: yes, narrow the enum in `tam-domain` so the driver's match cannot name a scope wider than its lease, leaving the breaker and the canary as the only fleet-halt writers.
7. Close the TTL residual on Tpt's worst-case submit, where one uninterrupted stretch of 360s runs under a 300s lease.
   Decided 2026-09-03: raise `LEASE_TTL_SECS` to 600, taken as recommended.
   One founder-gated number was the smaller change against a clock port that would have put a timer inside the adapter seam and handed every adapter a way to extend the lease it runs under; the heartbeat already distinguishes a working device from a gone one, so the cost is only that the reaper takes twice as long to notice a device that really stopped.
   Landed in step 11c, and `tpts_theoretical_worst_case_submit_fits_inside_the_lease` now asserts the guarantee where it used to assert the defect.
8. Release the duplicate-create fence on a positive absence, once the reconciliation has run for a while and the enumeration's reliability on each platform is known from live evidence rather than from its contract.
   Recommended: not yet, and not on any `Ok(None)` — only on a complete enumeration under the seller's own session, which is what `list_own_resources` already guarantees on both platforms by refusing a walk it cannot finish. The caveat that decides the timing is Tpt's: a create sits in an asynchronous processing queue for minutes, so an enumeration that lacks the marker can legitimately precede the listing appearing, and releasing on that would manufacture the duplicate the fence exists to prevent. Until then both answers stay stranded and surfaced, recorded as distinct causes so this can change without anything else changing.

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
