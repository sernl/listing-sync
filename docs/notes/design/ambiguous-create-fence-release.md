# The structural ambiguous-create fence and its operator release

Today an ambiguous create is fenced by an org-inventory halt alone.
This change writes the ambiguity into the mapping binding so the fence survives a cleared or lost halt, and adds the operator-driven release that retires it.
Code is untouched by this document; it specifies the change that follows.

Three premises in the commissioning brief are corrected here, each verified by reading.
The lowering table lives at `crates/tam-storage/src/lowering.rs`, not in `tam-engine`, and its lines 214-222 are the test `a_draft_intent_on_nothing_lowers_to_a_create_alone`, not production code.
The write attempt on this path is settled, not parked: `outcome_to_attempt_state` maps `Outcome::Ambiguous` to `"ambiguous"` at `crates/tam-engine/src/driver.rs:436-440`.
And `lower()` already refuses an `ambiguous_create` row, though under the wrong name; see "The lowering refusal" below.

## What is landed, and the residual

A create-time challenge routes to `reconcile()` at `crates/tam-domain/src/lib.rs:1376-1377`, whose `Effect::Reconcile` the driver refuses at `crates/tam-engine/src/driver.rs:753-763`, turning it into `halt_ambiguous` at `crates/tam-domain/src/lib.rs:1300-1321`.
That raises `Effect::Halt` with `HaltScope::OrgInventory`, which the driver services at `crates/tam-engine/src/driver.rs:820-837` by calling `raise_org_inventory` (`crates/tam-storage/src/jobs.rs:1329-1355`).
The lease scan excludes halted org-inventory pairs at `crates/tam-storage/src/jobs.rs:812-815`, and that exclusion is the entire protection.

The residual is that nothing durable records the ambiguity on the mapping.
`landing_effect` maps `Outcome::Ambiguous` to `committed = None` (`crates/tam-engine/src/driver.rs:276-280`), and a `Create` with no committed listing lowers to `LandingEffect::None` (`crates/tam-engine/src/driver.rs:290`), which `AttemptRepo::settle` turns into `BindDisposition::NotLanded` without touching the mapping (`crates/tam-storage/src/jobs.rs:1572`).
`settle_open_attempt` states the same thing in its doc comment at `crates/tam-engine/src/driver.rs:1195-1199`.
So the binding stays `unbound`, which the anchor test asserts at `crates/tam-engine/tests/driver.rs:932`.
Meanwhile the attempt settles to `'ambiguous'` with `settled_at` set, so the partial unique index `write_attempt_one_in_flight` — `WHERE state = 'in_flight'` at `crates/tam-storage/migrations/0005_job_ledger.sql:109-111` — stops fencing.
A halt cleared by hand therefore re-admits an `unbound` mapping to a second create.

No migration is required for the state itself.
The `mapping_binding_total` CHECK already admits it at `crates/tam-storage/migrations/0004_mapping.sql:65-72`, requiring `remote_id_kind IS NULL`, `binding_attempt IS NOT NULL` and `ambiguous_since IS NOT NULL`.
`Binding::AmbiguousCreate { attempt, candidates, since }` already encodes and decodes at `crates/tam-storage/src/mapping.rs:339-350` and `crates/tam-storage/src/mapping.rs:793-807`.
An empty candidate list is admissible, which is what a challenge-borne ambiguity carries, because nothing was read.

## 1. The fence write

`AttemptRepo::settle` writes it, in the transaction it already opens at `crates/tam-storage/src/jobs.rs:1549`, in the same statement sequence as the `write_attempt` UPDATE at `crates/tam-storage/src/jobs.rs:1550-1563`.
This is the only correct point: the attempt UPDATE is fenced on `state = 'in_flight'` and the run's `lease_epoch`, and returns `StorageError::StaleLease` when zero rows change (`crates/tam-storage/src/jobs.rs:1564-1566`).
Binding the fence write to that same transaction makes it impossible to fence a mapping whose attempt a stealer already settled.

The mechanism is a fifth `LandingEffect` variant, `Ambiguous`, produced by `landing_effect` for `(ItemOperation::Create, Outcome::Ambiguous)` in place of the `None` it returns today at `crates/tam-engine/src/driver.rs:290`.
`AttemptRepo::settle` matches it into an UPDATE setting `binding_state = 'ambiguous_create'`, `binding_attempt` to the settling attempt id, `ambiguous_since` to `at`, and `binding_marker = NULL`, guarded by `WHERE binding_state IN ('unbound', 'creating')`.
The guard mirrors the bind arm's own guard at `crates/tam-storage/src/jobs.rs:1589` and keeps the fence from overwriting a `bound` row that a concurrent path already resolved.
Zero rows affected returns a new `BindDisposition::FenceRefused { state }`, which `bind_anomaly` (`crates/tam-engine/src/driver.rs:1218-1245`) must map to a recorded anomaly rather than to `None`.
The engine may perform this write: migration 0018 grants it `UPDATE ON mapping` at `crates/tam-storage/migrations/0018_mapping_bind.sql:13`.
The same settle also writes `write_attempt.ambiguity_cause`, which section 2 explains.

Only `ItemOperation::Create` fences.
A revise re-applies the same fields and a removal re-deletes something already gone, which is the judgement `challenged()` records at `crates/tam-domain/src/lib.rs:1363-1364`.

## The lowering refusal

`lower()` already refuses an `ambiguous_create` seed: the arm list covers `"unbound" | "severed"` and `"bound"`, and everything else falls to `_ => Err(LoweringRefusal::CreateInFlight)` at `crates/tam-storage/src/lowering.rs:84-85`.
The refusal is therefore correct in effect and wrong in what it tells the seller, because `CreateInFlight` reads "wait for it to settle" (`crates/tam-storage/src/lowering.rs:36`) and an ambiguous create never settles on its own.
Add `LoweringRefusal::AmbiguousCreate` with a message naming operator review as the remedy, and an explicit `"ambiguous_create"` arm ahead of the catch-all.
This is an observable-message change, not a safety change; the safety already holds.

## 2. The two release verdicts

Release is an operator attestation in v1: a human asserts what is on the marketplace, and the system records that assertion rather than deriving it.

The `landed` verdict carries a marketplace listing id supplied by the operator.
The binding moves to `bound` with `remote_id_kind`, the id columns, and `first_seen_at` set, `verify_state = 'stale'` with `verify_stale_since` set, and `binding_attempt`, `ambiguous_since` and the `binding_candidate` rows cleared, matching the shape the bind arm writes at `crates/tam-storage/src/jobs.rs:1580-1589` and satisfying `mapping_verify_bound` at `crates/tam-storage/migrations/0004_mapping.sql:104-110`.
It must also satisfy the partial unique indexes `mapping_one_bound_url` and `mapping_one_bound_numeric_id` (`crates/tam-storage/migrations/0018_mapping_bind.sql:20-26`); a collision means the operator named a listing another mapping already claims, and the release is refused with that stated.

The `absent` verdict reverts the row to `unbound`, clearing `binding_attempt` and `ambiguous_since` so `mapping_binding_total` holds, and deleting the candidate rows.
The create requeues on the next lowering, which is the point of reverting rather than settling.

Evidence the operator screen must show, all of it already stored: the settled `write_attempt` row for `binding_attempt` — its `intent`, `correlation_marker`, `opened_at`, `settled_at`, `failure_code` and `evidence_ref` (`crates/tam-storage/migrations/0005_job_ledger.sql:82-99`); the mapping's `ambiguous_since`; the item's job and its events; and any `binding_candidate` rows (`crates/tam-storage/migrations/0004_mapping.sql:157-167`).
One field the screen needs is stored nowhere.
`write_attempt.ambiguity_cause` exists at `crates/tam-storage/migrations/0005_job_ledger.sql:98` but no production code writes it — the only other references are a live example at `crates/tam-worker/examples/live_provision.rs:788` and the migration itself.
`AttemptVerdict` carries no cause field (`crates/tam-storage/src/jobs.rs:1449-1456`), so the `AmbiguityCause` the machine computes at `crates/tam-domain/src/lib.rs:1075-1080` is discarded before it reaches the ledger.
Closing that is in scope here: an attestation made without knowing which ambiguity occurred is a blind attestation, and the column is a shipped gap between schema and code rather than new surface.
`AttemptVerdict` gains an `ambiguity: Option<AmbiguityCause>` field, the settle writes it in the `write_attempt` UPDATE beside `failure_code`, and a `const fn ambiguity_cause_to_db` joins `failure_code_to_db` in `crates/tam-storage/src/codec.rs:222-240` covering the five variants at `crates/tam-marketplace/src/lib.rs:144-150`.
The column is unconstrained `text` (`crates/tam-storage/migrations/0005_job_ledger.sql:98`), so no migration follows.

## 3. Halt interplay and ordering

Release the fence first and clear the halt second, both in one transaction on one connection.

One role reaches all three writes, which is what lets the release be atomic.
`tam_app` owns the schema, and the only privileges revoked from it anywhere are on `connection_secret` (`crates/tam-storage/migrations/0010_broker_custody.sql:7`), `field_audit` (`crates/tam-storage/migrations/0006_halts_connection_audit.sql:115`), `mapping_loss` (`crates/tam-storage/migrations/0024_mapping_loss.sql:48`) and `connection_audit` (`crates/tam-storage/migrations/0032_connection_audit.sql:51`).
None touch `mapping`, `binding_candidate` or the halt tables, so the mapping UPDATE, the candidate DELETE and the halt DELETE are all `tam_app`'s.
All three are org-isolated under FORCE row-level security with the same `FOR ALL` policy shape — `mapping` and `binding_candidate` at `crates/tam-storage/migrations/0004_mapping.sql:226-238`, `org_inventory_halt` at `crates/tam-storage/migrations/0006_halts_connection_audit.sql:124-130` — so one `app.current_org` pin covers all three, set transaction-locally as `pin_org` sets it at `crates/tam-storage/src/lib.rs:97-106`.
The engine's own `UPDATE ON mapping` grant (`crates/tam-storage/migrations/0018_mapping_bind.sql:13`) is not on this path; it exists for the in-run bind and for section 1's fence write.

The halt clear is conditional, because one halt row can stand for several ambiguities.
`org_inventory_halt` is keyed `(org_id, inventory, marketplace)` (`crates/tam-storage/migrations/0006_halts_connection_audit.sql:31-42`) and `raise_org_inventory` inserts `ON CONFLICT DO NOTHING` (`crates/tam-storage/src/jobs.rs:1340-1348`), so a second ambiguous create on the same pair raises no second row and an unconditional DELETE would unfreeze the queue with other fences still outstanding.
The DELETE therefore carries `AND NOT EXISTS (SELECT 1 FROM mapping WHERE org_id = $1 AND inventory = $2 AND binding_state = 'ambiguous_create')`, evaluated after the fence release in the same transaction.
Releasing the last ambiguity on a pair clears the halt; releasing any earlier one leaves it standing, which is correct because the remaining fences name writes nobody has adjudicated.

Ordering inside the transaction is what makes the condition read true, so it is not merely conventional: fence release, then the conditional DELETE, then commit.
A failure anywhere aborts the whole transaction — the attestation is not recorded and the halt is not cleared — so there is no partial-failure state to reconcile, which is what the single transaction buys.

## 4. Operator surface

The v1 surface is a host-side CLI binary, not an HTTP route.
It follows `tam-mint-session`, which takes its DSN as `argv` (`crates/tam-mint-session/src/main.rs:6`) and is invoked through a recipe as `just dev-session` invokes the mint (`justfile:156-158`).
Two subcommands: `list`, and `release <mapping> --verdict landed --listing-id <id>` or `release <mapping> --verdict absent`.

It holds two DSNs, and the split is read against write rather than mapping against halt.
`list` is cross-tenant, and every table it reads is org-isolated under FORCE row-level security, so a tenant-pinned connection cannot see the fleet at all.
It reads on the `tam_engine` DSN, which is BYPASSRLS by construction (`db/init/01-app-role.sql:17-18`) and already holds `SELECT` on everything the listing needs: `mapping` and `binding_candidate` (`crates/tam-storage/migrations/0007_engine_role_grants.sql:11-12`), `write_attempt` and the job tables (`crates/tam-storage/migrations/0007_engine_role_grants.sql:7-8`), and `org_inventory_halt` (`crates/tam-storage/migrations/0007_engine_role_grants.sql:9-10`).
The listing writes nothing, and that role holds no DELETE on any table this change touches, so the read path cannot mutate even by mistake.

`release` acts on one tenant and runs wholly on the `tam_app` DSN, org-pinned, as section 3 describes.
Refusals are exit codes and stderr rather than wire codes: the mapping is not in `ambiguous_create`, the named listing is already claimed by another mapping, or the mapping does not exist.
No `APIErrorCode` variant is added, so `web/src/lib/generated/vocab.ts` is unchanged and `just web-typegen` is not part of this change.

## 5. Crash and idempotency

Fence written, halt lost.
Safe, and it is the whole point of the change: the lowering refusal of section 1 stands on the mapping row alone and needs no halt.

Release interrupted at any point.
The single transaction of section 3 makes this total — it commits or it does not, and a crash mid-release leaves the mapping in `ambiguous_create` with the halt standing, which is the state it started in.

Double release.
The fence write is guarded on `binding_state IN ('unbound', 'creating')` and the release on `binding_state = 'ambiguous_create'`, so a second release matches zero rows and refuses rather than re-writing a row now `bound`.
The operator sees the not-in-fence refusal, which is also the correct answer for a release racing another operator's.

Release racing the engine's own fence write.
Both take a row lock on the same `mapping` row, so they serialise rather than interleave, and whichever commits second finds the other's state and its guard refuses.
This is why both guards name the states they expect rather than writing unconditionally.

Release racing an engine drain.
The halt stands for the whole transaction and is cleared only at commit, and the lease scan excludes halted pairs at `crates/tam-storage/src/jobs.rs:812-815`, so no item on that tenant and inventory can be leased while the release runs.
After commit the mapping is `bound` — whose lowering yields a revise, not a create — or `unbound`, whose requeued create is the intended outcome of the `absent` verdict.

Release racing a seller unlink.
The lease scan also requires a `linked` connection at `crates/tam-storage/src/jobs.rs:816-819`, so an unlinked tenant drains nothing.
The release is a ledger correction rather than a marketplace action and stays valid across an unlink; the requeued create waits for the re-link.

## 6. Test plan

The anchor is `a_challenge_on_a_create_holds_the_fence_and_mints_nothing_further` at `crates/tam-engine/tests/driver.rs:882`.
It must be amended, not merely extended: its assertion that `binding_state` reads `unbound` at `crates/tam-engine/tests/driver.rs:932` becomes `ambiguous_create`, and its rationale comment updates to say the fence is now the row rather than only the halt.
`a_rate_window_closing_mid_create_holds_the_duplicate_fence` (`crates/tam-engine/tests/gauntlet.rs:1053`) and `a_relinked_create_stays_parked_behind_its_own_duplicate_fence` (`crates/tam-storage/tests/leases.rs:1608`) cover the in-flight fence and should stay green untouched; if either moves, the fence write has leaked outside the create-ambiguity path.

New pg-gated tests:

- The fence survives a hand-cleared halt: fence, delete the halt row directly, run the scan, assert no create is leasable and the binding still reads `ambiguous_create`.
- `lower()` refuses an `ambiguous_create` seed with `LoweringRefusal::AmbiguousCreate`; a unit test beside the existing lowering tests at `crates/tam-storage/src/lowering.rs:213-222`, not pg-gated.
- Release `landed` binds, clears `ambiguous_since` and `binding_attempt`, deletes candidates, and leaves `verify_state = 'stale'` with `verify_stale_since` set.
- Release `landed` naming a listing another mapping holds is refused by `mapping_one_bound_url` or `mapping_one_bound_numeric_id`, with the mapping unchanged.
- Release `absent` reverts to `unbound` and the next lowering yields `vec![Create]` again.
- Double release matches zero rows and refuses, leaving the `bound` row from the first release untouched.
- A release whose halt clear fails aborts wholly: the mapping still reads `ambiguous_create` and the halt still stands.
- The halt clear is conditional: with two ambiguous mappings on one org and inventory, releasing the first leaves the halt standing and releasing the second clears it.
- The fence write records `ambiguity_cause` on the `write_attempt` row, and it survives the round trip through `ambiguity_cause_to_db`.
- The fence write is refused when the mapping is already `bound`, recording the anomaly rather than overwriting.
- Row-level security: a release pinned to org A cannot touch org B's halt, mapping or candidate rows.

## 7. Non-goals

No automatic reconciliation.
`Effect::Reconcile` stays unimplemented at `crates/tam-engine/src/driver.rs:753-763`; list-and-match is the reconciliation milestone's.

No engine access to first-party export listing.
`list_own_resources` lives on `FirstPartyExport` (`crates/tam-marketplace-tpt/src/flows.rs:1285-1294`) and not on `MarketplaceAdapter` (`crates/tam-marketplace/src/lib.rs:510-589`), and must not be reached from the engine.

No HTTP operator surface.
The routes an earlier draft of section 4 sketched are deferred to whenever an operator console exists, so this change invents no authentication principal: every route today is tenant-scoped through the `OrgContext` extractor (`crates/tam-api/src/session.rs:1-3`, `crates/tam-api/src/lib.rs:9`) and the route table (`crates/tam-api/src/lib.rs:94-149`) carries no operator path.

No seller-facing release.
The seller sees a halted inventory and its cause; the attestation is the operator's, because a seller asserting "it landed" against their own duplicate is the failure this fence exists to prevent.

## Recorded decisions

The operator surface is a CLI binary and not an HTTP route, so no operator role, admin principal or non-tenant session kind is invented here, and no `APIErrorCode` variant is added.
Threading `AmbiguityCause` into `AttemptVerdict` and writing `write_attempt.ambiguity_cause` is in scope, because an attestation made without knowing which ambiguity occurred is a blind one and the column is an already-shipped gap between schema and code.
No partial index on `binding_state = 'ambiguous_create'` is added: the operator listing is rare, operator-invoked and small at launch scale, so a sequential scan serves it.
Revisit that last one as a migration only if `ambiguous_create` rows accumulate past what a sequential scan serves.

One thing changed in the specifying rather than being ruled.
Section 3 previously argued that the fence release and the halt clear could not share a transaction because no single role reached both; verifying the grants showed that `tam_app` reaches all three writes, so the release is now one atomic transaction and the partial-failure handling that argument required is gone.
