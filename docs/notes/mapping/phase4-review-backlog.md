---
title: Phase 4 review backlog
---

The adversarial review of the full Phase 4 diff raised seventy findings, of which sixty-one survived verification.
Two fix slices landed against them: slice one took the must-fix set (twenty-four findings, several of them the same defect reported by more than one reviewer), and slice two took the subset that blocks the live battery (eleven).
The remaining twenty-six are accounted for here, so the deferral is a decision rather than an omission.
Two of them are closed by this slice's own documentation work rather than deferred, which leaves twenty-four carried past the battery; the decision-record correction slice two made is recorded at the end for traceability.
Each entry names the finding, its verified severity, where it lives, and why it is safe to carry.

The two slices' scope, for reference.
Slice one fixed election consumption and `apply_to_future`, loss production and its wiring, the body-format declaration end to end, the give-up arm's failure code, the severed-counterpart gate, the 16+ wire values, and the redrained migrate's removal leg.
Slice two fixed the drain's intent lowering, the TPT-source spin at both ends, the bind-driven counterpart revive and the admission-gate parks that never settled, the `JobSettled` race, the raise-park race, the concurrent same-key sync 500, and the ignored `?outcome=` on the jobs list.

## Deferred: the decision surface

*The decision surface offers candidates the trigger never offered, and an off-list answer silently never applies* — should-fix, `crates/tam-api/src/resources.rs:715` and `:720`.
`list_decisions` renders every `Exact` edge into the target vocabulary as the candidate set whatever the trigger is, and `answer_decision` validates nothing against the trigger, so a seller can be shown and can submit a year group the band does not cover.
Deferred post-battery: the battery answers elections through the values the projection itself raised, so the rendered superset is a surface defect rather than a wrong write, and an off-list answer is inert rather than corrupting.
It should be fixed before any seller touches the surface unsupervised, because an answer that silently never applies re-raises forever.

*`GET /{v}/elections/items` is unpaginated and issues two queries per open election* — note, `crates/tam-api/src/resources.rs:701`.
Deferred post-battery: the battery's decision surface holds a handful of questions, not the five hundred a bulk would raise.

*`POST /{v}/elections/rules` answers ordinary seller input with a 500 instead of a 422* — should-fix, `crates/tam-api/src/resources.rs:860`, with the same defect reported twice more at `:891` (trigger kind/key mismatch) and `:896` (a narrow or supply rule with no `trigger_key`).
`ElectionRule::new` does not validate the trigger-kind/trigger-key pairing that `election_rule_trigger_key` enforces, so the CHECK violation surfaces as a fault.
Deferred post-battery: the battery submits well-formed rules, and the three are one fix in one function — the validation belongs in `ElectionRule::new` beside the delegation bar, which is where the next slice should put it.

## Deferred: the taxonomy and the seeders

*No band-to-TPT relation is seeded, so every GB-sourced product blocks on an unanswerable gap when cross-listed to TPT* — should-fix, `crates/tam-taxonomy/src/grades.rs:419`.
`seed_bands` emits `Narrower` edges from each `ageRanges` band only into `TesUs` and `TesNz`, so a GB product's grades reach `(Tpt, Phase)` with neither an edge nor a `NoCounterpart` record and the item blocks on reconciliation.
Deferred post-battery only if the battery's GB→TPT leg uses `yearGroups`-sourced products; a GB product carrying `ageRanges` bands blocks, and the remedy is either the missing `Narrower` edges or an explicit `NoCounterpart` into `(Tpt, Phase)`.
This is the highest-risk entry in this file and the first the next slice should take.

*The import labels and age-derives `yearGroups` ids through the GB `ageRanges` table* — should-fix, `crates/tam-import/src/lib.rs:504`.
The adapter now tags `yearGroups` as `TermKind::Phase` for TesUs and TesNz, and the label lookup and `derive_interval` still resolve every id against the seven-row GB `ageRanges` table, so an id in 1..7 matches the wrong row.
Deferred post-battery: it mislabels a US or NZ *source* import, which the battery does not exercise — every battery import is GB-sourced.

*All seven Tes licence tokens seed identity edges, so a read-back-only licence projects cleanly and fails at the wire* — should-fix, `crates/tam-taxonomy/src/licences.rs:102`.
`TES-V1`, `TES-V2` and `TES-PAID-SCHOOL` are read-back-only and resolve as ordinary projections.
Deferred post-battery: it fails loudly at the write path rather than writing a wrong licence, and the trim is a taxonomy-seed change that wants the founder's eye on which tokens are writable.

*A second resolution of the same (term, vocabulary) now returns 500 and leaves the queue item permanently open* — should-fix, `crates/tam-storage/src/taxonomy.rs:410`.
0021's `projection_edge_single_valued` index added a unique-violation path `map_unique` does not translate.
Deferred post-battery: it needs two resolutions of one pair, which a single battery run does not produce.

*The seeder's Exact/Broader upsert retargets an existing edge without refreshing its provenance columns* — should-fix, `crates/tam-storage/src/taxonomy.rs:586`.
A re-seed can silently repoint a seller-authored edge while the row keeps naming the human who decided it.
Deferred post-battery: the battery seeds once, and the fix is a provenance decision (does a seeder overwrite a human?) rather than a mechanical one.

## Deferred: the projection's residue

*An unrecognised source path reaches no consumer, so a listing publishes with no grades and nothing says so* — note, `crates/tam-taxonomy/src/listing.rs:218`, and *the `unrecognised` bucket is produced and consumed nowhere, and has no carrier on the success path* — should-fix, `crates/tam-domain/src/lib.rs:349`.
One defect in two places: `ListingProjection` has no `unrecognised` field, and all three blocked-path consumers destructure it away.
Deferred post-battery: `AxisOutcome.unrecognised` is never written today, so the bucket is empty rather than lossy, and giving it a carrier is the same shape as the loss carrier slice one added — it should follow it.

*A TPT-sourced product carries no subjects and no grades, and nothing names the omission* — note, `crates/tam-import/src/lib.rs:493`.
Deferred post-battery: TPT-as-source is gated behind the G-O3 capture, which slice two now refuses at both ends, so no TPT import can occur until that gate opens.

*P.1's 16+ election did not land; a 16+-only listing still posts no age* — should-fix, `crates/tam-marketplace-tes/src/flows.rs:532`, partially fixed.
Slice one removed the invented `mainAge 0`, so the wire no longer states an age nobody observed; the election that would let the seller state the target ages did not land, and the departure is recorded in `phase4-amendment-corrections.md`.
Deferred by the same gate P.1 defers the wire question to: an `ElectionAnswer` names vocabulary paths and Tes holds no bounded band above 16, so the shape it needs reaches the CHECK, the codec, the API and the client.

## Deferred: the drain and the maintenance loop

*A sync request has no claim, so two drain processes canonicalise it twice* — note, `crates/tam-storage/src/sync_requests.rs:261`.
`pending()` returns requests already `draining` and `mark_draining` is an unconditional UPDATE with no row-count check.
Deferred post-battery: the battery runs one drain process.
The note the next slice should carry with it is that a claim also settles the double-canonicalisation question — the breadcrumb makes a redrain safe within one process, not across two.

*`revive_on` and `revive_by_gap` silently no-op when the item is leased rather than parked* — should-fix, `crates/tam-storage/src/jobs.rs:431`, partially fixed.
Slice two closed the election gate's instance of this window by re-checking after the park.
The same window remains for the reconciliation gate through `revive_by_gap`: an answer landing between `taxonomy.raise` and the park matches nothing and the item waits out the full day.
Deferred post-battery: the reconciliation queue is answered by an operator on a slower loop than the seller's own election answer, so the day-long wait is a latency defect rather than a stall.

*Two cross-tenant maintenance transactions lock `org_event_counter` rows in unsynchronised order* — note, `crates/tam-storage/src/jobs.rs:860`.
`expire_and_steal` and `revive_expired` each hold one transaction spanning several organisations and take a row lock per organisation in the order their `RETURNING` rows arrive, so two concurrent maintenance passes can deadlock.
Deferred post-battery: it needs two worker instances running the two passes concurrently across the same set of tenants, which the battery does not do.
Slice two's `settle_if_complete` takes the same lock earlier in its transaction, which does not change the ordering hazard — sorting each pass's per-organisation work by org id remains the fix.

*The drain report still does not count refusals, so the new refusing paths are invisible in the recorded job* — should-fix, `crates/tam-import/src/main.rs:192`.
`DrainTotals` has no refusal field, so a run in which every row refused records as indistinguishable from a run with no rows.
Deferred post-battery: the battery reads its refusals from the process's own stderr, and slice one's body-format refusals made this more visible rather than less.

## Deferred: tests and documentation

*O.18 landed as a key-derivation unit test that never exercises the drain* — should-fix, `crates/tam-storage/tests/sync_requests.rs:117`, and *tam-sync-worker has no tests: O.11, O.12 and O.18 are all unverified* — should-fix, `crates/tam-sync-worker/src/lib.rs:77`, both partially fixed.
The crate now has two database-backed tests: slice one's resumed-migrate removal leg and slice two's live-intent lowering.
O.18 as specified — enqueue a migrate, assert two distinct job ids with one create item and one removal item — and K.4's foreign-org refusal are still absent.
Deferred post-battery: the redrain test covers the property O.18 was written to protect from the direction that actually failed, and the battery itself exercises the enqueue path end to end.

*O.3 landed without the oracle the amendments specified for it* — should-fix, `crates/tam-engine/tests/seed.rs:449`.
The three counterpart tests assert `prepare_item`'s return value and never observe whether the adapter's `remove` was called, so `FakeState.deletes == 0` — the assertion that a gated removal does not reach the marketplace — is unasserted.
Deferred post-battery: the battery observes the same property live, against a real account, which is a stronger oracle than the fake; the fake's version should still land, because the battery is not run per commit.

*Missing and weakened tests from the §I/§O plan* — note, `crates/tam-taxonomy/src/listing.rs:316`, partially fixed.
O.1's wedge half (a source-declared licence surviving a standing rule into the field set), K.5's two-caller broker lease test, and O.16 are absent.
O.7's `requires_bound_on` half is now asserted on the drain path by slice two's live-intent test and remains unasserted on the API path.
Deferred post-battery: these are coverage rather than defects, each one already having a named specification to land against.

*The half-declared-rights test passes on a NOT NULL violation, never reaching `product_rights_total`* — note, `crates/tam-storage/tests/tenancy.rs:196`.
The INSERT omits `body_format`, which 0022 makes NOT NULL, so the statement fails on 23502 and the test would stay green if the CHECK were deleted.
Deferred post-battery: the CHECK is present and correct; the test is the thing that is wrong, and it is a two-line repair (add the column, assert the constraint name).

*F2's documentation obligation did not land* — note, `docs/design/schema.md:368`.
Fixed rather than deferred, being three sentences about a settled decision: the read-leg section now states that `publish_mode` is the mapping's standing policy about whether this system writes to that marketplace at all, that the seller's per-request intent travels as the item's own `ItemOperation`, and that a `DryRun` mapping under a live-intent job is therefore not a contradiction.

*Commit 15's live-run lane item is neither run nor recorded as deferred* — note, the commit-15 lane in the Phase 4 amendments.
Recorded rather than deferred: `phase4-amendment-corrections.md` now names the D2 licence value and the D3 `mainType` absence as unverified against the live API and puts them on the pre-Phase-5 capture list beside the 16+ wire question they travel with.

*decisions.md misstates the milestone's engine grants* — should-fix, `docs/design/decisions.md:322`.
Fixed rather than deferred: a factual error in the decision record is not a deferral candidate.
The entry now states what landed — no engine *write* grant on any catalogue table, `native_residue` under 0016's existing SELECT, `election_item` and `mapping_loss` mirroring the reconciliation grant, `election_rule` deliberately read-only — and the TPT-custody sentence beside it, which slice two's source refusal made false, is restated as the forward commitment it now is.
