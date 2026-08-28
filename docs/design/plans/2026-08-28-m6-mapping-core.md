# M6: the mapping core

Goal: the cross-platform mapping framework — the binding write that records where a landed listing lives, the per-inventory field registry that makes every field identifiable as particular to a platform, and the connector contract that makes a new marketplace an adapter plus data rather than an excavation.
This plan begins the founder's 2026-08-28 sequence — M6 mapping core, then the M7 TPT connector, then M2 analytics with the operations dashboard — recorded in `decisions.md` under "The post-M1 sequence, reordered 2026-08-28".
Drift verification, the read-back that promotes a binding from stale to clean or mismatched, is the follow-up slice on top of this plan, not part of it; this plan makes bindings exist so there is something to verify, and it deliberately leaves every fresh binding stale.

## Decisions this plan makes

- The bind is folded into the attempt settle as a single transaction rather than a separate driver call, because the attempt settle is already epoch-fenced (its zero-row branch is the steal detector the driver turns into `Abandoned`), `write_attempt` already carries the mapping id, and a separate call leaves a crash window where the attempt says committed while the mapping stays unbound.
- The engine role gains `UPDATE ON mapping` in migration 0018 and nothing else: no INSERT, no DELETE, nothing on the child tables. Binding out of `ambiguous_create` needs `binding_candidate` deletes and a mismatched verification needs `field_mismatch` inserts, so both stay on the app path with reconciliation, which keeps the 0007 enumeration's no-delete rule intact for the engine.
- A fresh binding always carries `Verification::Stale { since }`. The report handed to settle is still the version-zero unnormalised report, so claiming clean would assert a comparison that never ran; stale satisfies `mapping_verify_bound` and is the honest state until the drift verifier lands.
- Both committed-class outcomes bind: `Committed` and `Degraded` each carry a receipt, and a bind keyed on `Succeeded` alone would silently drop the Degraded landing.
- The prior-state fence is `binding_state IN ('unbound', 'creating')`. A bind never overwrites an existing binding: re-landing the same remote id is an idempotent no-op that preserves `first_seen_at`, and landing a different id against a bound row is recorded as a divergent disposition, never an overwrite — the second listing stays queryable from `write_attempt`'s remote-id columns, which is exactly what they were added for.
- A refused or divergent bind never rolls back the attempt settle. The write landed; refusing to record that the attempt committed would retry the item and mint a third listing.
- Migration 0018 also adds partial unique indexes on the bound remote identity (url-kinded and numeric-kinded, `WHERE binding_state = 'bound'`), so two mappings in one org and inventory cannot claim the same remote listing; `mapping_one_per_inventory` protects the product side and nothing protected the remote side.
- `FieldKey` stays closed at six and `FieldPolicies` stays a six-field struct; the compile-error-on-new-field property is a recorded design decision and this plan does not spend it. Platform-particular fields get a second axis instead: a per-inventory registry declaring native fields (Tes licence, curriculum, mainType; Etsy who_made, when_made) beside per-field constraints on the canonical six.
- The registry is a module in `tam-domain`, not a crate: it breaks on nobody else's schedule, needs no special test environment, and holds no privilege. `tam-limits` refuses per-field caps by its admission rule, so the registry is the first home per-platform constraints have had.
- The registry records measured or documented facts only. Tes title caps are unmeasured and stay absent; Etsy's 140-character title cap is documented by Etsy and is recorded; TPT's sampled 80-character cap has an unverified counting unit and stays absent until the M7 first-contact measures it.
- `project_listing` consults the registry and applies declared caps at projection time, closing the gap where `LengthUnit` had zero consumers against a design that says caps are applied at projection and never at authoring. No cap is declared for any Tes inventory, so live behaviour is unchanged; the seam exists for the platforms that need it.
- The Tes-shaped taxonomy and grades JSON encoding moves out of `tam-engine/src/seed.rs` behind the adapter seam, because a shared engine hardcoding one platform's wire shape (and hard-failing on non-numeric native ids) blocks every platform whose category ids are not i64. The move is byte-identical for Tes: same entries, same JSON, same intent hash.
- The first-party export reads (`list_own_resources`, `download_resource_bundle`, `fetch_for_import`) promote from inherent methods on the Tes adapter to a seam trait, so cross-platform import and the drift reader address a capability rather than a Tes type.
- A projection for a mapping that is already bound parks the item rather than creating again. No update path exists yet, so every create against a bound mapping would mint an orphan listing; parking is the stall-bias answer until the update path lands, and the settle-side dispositions stay as defence in depth for the races the gate cannot see.
- The anomalous bind dispositions (divergent, claimed elsewhere, refused) are recorded in the job ledger as their own event kind rather than a log line, because the M2 dashboard reads the ledger and a landed-but-unbound listing must be visible there.
- `FieldSet` stays stringly typed until the second encoder exists. Typing the engine-adapter field contract against one implementation is speculation; the TPT adapter in M7 is the forcing case, and the deferred item below names it.

## Tasks

### Task 1: the bind

Extend `WriteAttemptRepo::settle` to a transaction that performs the existing fenced `write_attempt` UPDATE and then, when the verdict carries a landed id, the mapping bind UPDATE.
The method gains the mapping id as a parameter (the driver holds it as `lease.mapping`) and returns a bind disposition alongside success: bound, already bound to the same id, divergent landing carrying the existing id, refused in a non-bindable state, or not landed.
The bind UPDATE sets exactly: `binding_state='bound'`, the three remote-id columns via `RemoteIdColumns`, `first_seen_at`, `verify_state='stale'`, `verified_at=NULL`, `verify_stale_since`, `updated_at`, and nulls `binding_attempt`, `binding_marker`, `ambiguous_since`, `severed_at`, `sever_cause` so a bound row round-trips equal to a freshly inserted one.
Migration 0018 carries the engine grant and the two partial unique indexes.
Tests: a storage-level bind round trip against `MappingRepo::get`, the refuse-divergent and idempotent-same-id cases, a structural test that the unique index refuses a second bound claim, the driver happy path extended to assert the mapping's bound columns, the ambiguous test extended to assert the mapping stays unbound, and the gauntlet clean run extended to assert the binding carries the FakeTes-minted url.
`just db-prepare` regenerates offline metadata; the lane gates are `just check` and `just db-test`.

### Task 2: the field registry

New module `crates/tam-domain/src/registry.rs`: a per-inventory `InventoryRegistry` with a six-field canonical spec struct (cap and required-ness per `FieldKey`, mirroring the `FieldPolicies` shape so a new `FieldKey` is a compile error here too) and a slice of native field specs (name, closed vocabulary or free text, read/write direction).
Populate Tes GB, US and NZ from the measured facts on file: the ten written draft fields, the licence vocabulary, the thirteen-value curriculum vocabulary from probe 04 (read-only today), and no caps.
Populate Etsy from its published API contract (required fields, the 140-character title cap, who_made and when_made as natives) and leave TPT sparse until M7 measures it.
Wire `project_listing` to apply declared caps at the declared `LengthUnit`, with std-only truncation that never splits a UTF-8 boundary and under-approximates grapheme counting by codepoint truncation.
Tests: truncation at each unit, a pin that Tes projections are unchanged, and a pin that a declared cap truncates and an undeclared one copies verbatim.

### Task 3: the connector contract

Add `project_fields` to `MarketplaceAdapter` (projection in, `FieldSet` out) and move the Tes taxonomy, grades and licence encoding from `tam-engine/src/seed.rs` into the Tes adapter's implementation, byte-identical, with the existing flows and cassette tests pinning the wire shape.
Promote the first-party reads to a seam trait implemented by the Tes adapter, and point `tam-import` at the trait.
Record the connector recipe in this plan as the M7 runway: a new platform is the seven inventory-ripple sites plus one migration, a registry entry, an adapter implementing the seam, a transport, and a taxonomy route (bulk derivation or queue-driven authoring — the queue route needs zero code).

### Task 4: close

Lanes green (`just check`, `just db-test`, `just web-check` untouched but verified, `nix flake check`), the plan's outcome recorded in `decisions.md` when the milestone closes, and the CLAUDE.md current-state pointer already names the newest plan file by construction.

## Deferred, with owners

- The curriculum write path: the market-targeting field the GB-to-NZ wedge depends on is read on import and discarded, and is absent from the written-field manifest. Whether the uploader writes it, and how, needs a founder-supervised capture of the uploader setting a curriculum; the registry declares the field read-only until then. Owner: founder capture, then M7-adjacent code.
- Native-field carriage in `FieldSet`: the registry declares native fields but the write path cannot carry them yet. Lands with the first consumer (curriculum, or the Etsy natives). Owner: M7.
- Typing the engine-adapter field contract (replacing stringly `FieldSet`): forced by the second encoder. Owner: M7, with the TPT adapter.
- Generalising the import run: the read seam is now a capability, but the run itself still speaks Tes licence tokens, mints GBP, labels grades from the Tes age-range table, and addresses resources numerically. Owner: M7, with the second platform's import.
- The drift verifier and the normaliser: `unnormalised_report` still reports every read-back clean, so `Degraded` stays unreachable; the verifier promotes stale bindings and feeds the M2 dashboard. Owner: the M6 follow-up slice.
- Resolution paths out of `ambiguous_create` and `severed`: app-path reconciliation with child-row cleanup, beyond the engine's privileges by design. Owner: the M6 follow-up slice.
- The sketch drift: `docs/design/sketches/domain.rs` is cited as the artefact of record by three crate headers and five documents but is frozen at the scaffold commit with five concrete divergences from the crates. Re-syncing or retiring it is a founder call about an artefact of record. Owner: founder.
- Etsy personal-app registration and the Classful `wc/v3` key ask: lead-time items that gate M3 and the fourth platform, both startable now. Owner: founder.
