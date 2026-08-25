# M1g: the taxonomy hub

Goal: the canonical taxonomy with GB and NZ projections derived from the measured deterministic prefix crosswalk, the reconciliation queue that drains rather than treadmills, grade provenance kept verbatim, and the counters the reconciliation-drain kill gate reads.
Everything here is pure or storage-backed; the wiring into the duplication flow (and the composed four-gate `ProjectionBlocked`) is M1j's, and the queue UI is M1i's.

The design of record is `docs/design/taxonomy-projection.md` (the five-row projection table, inbound reverse-`Exact` law, authoring policy), `docs/notes/probes/10-taxonomy-crosswalk.md` (the measured prefix transform), and `docs/design/sketches/domain.rs` (the types promoted verbatim).

## Decisions this plan makes

- A new pure crate `tam-taxonomy` hosts the projection function, the crosswalk derivation over the captured Tes trees, and the Tes age-range table.
  It depends on `tam-types`, `serde`/`serde_json` (parsing captured trees from bytes), and `uuid` (v5 canonical ids); no I/O.
  It does not live in `tam-domain` because the sync machine and the taxonomy hub are separate bounded concerns, and not in `tam-marketplace-tes` because the hub must never pull adapter I/O dependencies into its tests.
- `InventoryId` gains `TesNz`; `marketplace_inventory` gains `('tes_nz','tes')`.
  `CurrencyRule` gains `Unmeasured`, and `TesNz` maps to it: the NZ-inventory currency has not been probed, and `ProjectionBlocked::CurrencyUnknown` is the designed consumer of exactly this state, so the publish gate fails closed instead of the enum inventing NZD or assuming GBP.
  The idempotency ordinal for `TesNz` is 4; existing ordinals are durable keys and are never renumbered.
- Canonical term ids are deterministic: UUIDv5 under a new `NAMESPACE_TAM_TAXONOMY` over the name `tes:{mapTo}`, where `mapTo` is Tes's own market-neutral prefix-9 id measured identical across GB and NZ.
  Deterministic ids make reseeding idempotent (`ON CONFLICT DO NOTHING`) and make independent GB and NZ crawls converge on the same canonical term.
- The crosswalk pairs nodes on `mapTo` equality and validates the suffix rule (GB `1`+suffix, NZ `7`+suffix) and description equality; a node violating any of the three goes to the residue with no edge on the unpaired side, so a drifted capture degrades to `Absent` plus a reconciliation item rather than a wrong edge.
  `Decider::Imported.source` carries the capture's own `_source` string, so provenance travels from the crawl into every seeded edge.
- The projection corner cases the five-row table leaves open are pinned as: one `Exact` and no `Broader` is `Exact`; two or more `Exact` is `Ambiguous`; `Broader` edges with no `Exact` are `Broadened` only when they agree on one target path, else `Ambiguous`; an `Exact` coexisting with a `Broader` naming a different path is `Ambiguous` (a `Broader` duplicating the `Exact` target is degenerate and reads as `Exact`).
  Ambiguity always defers to a human; the seeded wedge data contains only single `Exact` edges, so these corners are exercised by unit tests, not by production data.
- Inbound `ingest` is the reverse of `Exact` edges only; a duplicated reverse target in the relation (impossible under the partial unique index) reads as unmapped rather than picking a winner, so a corrupted relation fails closed into the queue.
- The derived `AgeInterval` exists only when every declared range carries both bounds; `16+` and `Age not applicable` derive `None` rather than an invented number, and the verbatim declaration remains the fact re-emission uses.
- The reverse-uniqueness law lands as a partial unique index `projection_edge (to_inventory, to_term_kind, to_segments) WHERE kind = 'exact'`, so two canonical terms claiming one target path as `Exact` is rejected at insert.
- Queue dedup lands as a partial unique index `reconciliation_item (org_id, term, target_inventory, target_term_kind) WHERE state = 'open'`, so a batch raises one item per gap.
- `reconciliation_item` is tenant data under the forced null-safe RLS pattern; `canonical_term`, `projection_edge` and `projection_no_counterpart` are global reference data (the canonical taxonomy is ours), readable by `tam_app` and `tam_engine`, writable by `tam_app` because resolution is a human decision arriving through the API; the wedge's queue owner is the founder as customer zero.
- Seeding is a one-shot operator mode in a new thin binary `tam-taxonomy-seed <db-url> <gb.json> <nz.json>` following the `tam-pipeline-worker ingest` pattern; the captured trees stay canonical in `docs/design/data/` and the crane source filter admits them for the sandboxed tests that embed them.

## Tasks

### Task 1: the inventory ripple

The taxonomy and grade vocabulary (`TermKind` through `AgeInterval`) was already promoted verbatim into `tam-domain` during M1a, so this task is the inventory ripple alone: `InventoryId::TesNz`, `CurrencyRule::Unmeasured`, `NAMESPACE_TAM_TAXONOMY` in `tam-types`; `tes_nz` in the storage codec both directions and in the round-trip proptest strategy; the idempotency ordinal `TesNz => 4` with the pinned-ordinal test extended; and migration `0012_tes_nz_inventory.sql` inserting the inventory row so the proptest's foreign key holds.
The hub schema therefore lands as `0013_taxonomy_hub.sql`, and `tam-taxonomy` depends on `tam-domain` for the vocabulary rather than duplicating it.

### Task 2: the pure hub

`tam-taxonomy`: `project(term, vocabulary, edges) -> TermProjection` implementing the pinned table; `project_terms` producing `{included, loss, blocked, omitted}` with order-preserving dedup and merged `dropped`; `ingest(path, edges) -> Option<CanonicalTermId>`; `tes` module parsing the captured tree shape and `derive_crosswalk(gb, nz) -> {terms, edges, residue}` with the three-way validation; the Tes main-age-range table (seven rows from the captured vocabulary) and `derive_interval`.
Tests: one unit test per projection table row and per pinned corner; crosswalk unit tests on synthetic trees for pairing, residue and validation refusal; the exhaustive test over the embedded real captures asserting all 43 subjects pair with zero description mismatches, the topic residue is exactly the measured handful, and for every derived edge `ingest(edge.to) == Some(edge.from)` and `project(edge.from) == Exact{edge.to}` — the totality-and-round-trip property the charter wanted proven, discharged over the full finite domain.
`flake.nix`: admit `docs/design/data/*.json` to the crane source filter.

### Task 3: the durable hub

Migration `0012_taxonomy_hub.sql`: `projection_edge` (PK `(from_term, to_inventory, to_term_kind, to_segments, kind)`, decider-totality CHECK, the reverse-`Exact` partial unique index), `projection_no_counterpart` (PK `(term, to_inventory, to_term_kind)`, decider CHECK), `reconciliation_item` (org-scoped, mapping FK, state CHECK, settled CHECK, open-dedup partial unique index, forced null-safe RLS), the `tes_nz` inventory row, and the grants above.
`tam-storage/src/taxonomy.rs`: `TaxonomyRepo` with `seed` (idempotent upsert returning inserted-versus-existing counts), `edges_into`, `no_counterparts_into`, `raise` (returning new-versus-already-open per item), `open_items`, `resolve_with_edge` (transaction: edge insert plus state flip), `resolve_no_counterpart`, `drain_stats`.
Tests (`tests/taxonomy.rs`, pg lane): seed idempotence; the drain (raise, resolve with an edge, re-project finds `Exact`, second raise reports already-drained); dedup (two raises, one row); reverse-uniqueness insert rejection; two-tenant RLS isolation on `reconciliation_item`; `drain_stats` counts.

### Task 4: grade provenance

Corrected in flight: M1b's `ProductRepo` already writes and reads the grade declaration as part of the product aggregate — one unit by design — so a separate `GradeRepo` would split what belongs together and is not built.
The pure derivation (`derive_interval` and the Tes age-range table) ships in Task 2's crate.
This task is the law coverage the aggregate round-trip's seller fixture does not reach: `tests/grades.rs` pins the imported, multi-path, native-id declaration round-tripping verbatim and ordered, and the open-ended declaration deriving nothing rather than an invented bound.

### Task 5: seeding and the drain report

`tam-taxonomy-seed`: parse both captures, derive, refuse on file-level validation failure, seed via `TaxonomyRepo`, print the seed report (terms, edges, inserted, existing, residue).
End-to-end pg test: seed the real captures into the test database, load edges through the repo, project a subject GB-to-NZ to `Exact`, project a residue topic to `Absent`, raise, resolve, and confirm the drain; `drain_stats` reflects the run.
Documentation: the phase table and `CLAUDE.md` current-state note.

## Deferred, with owners

- The composed `ListingProjection` and four-gate `ProjectionBlocked` (currency, cover, scan alongside taxonomy): M1j, where the duplication flow assembles them.
- Inbound catalogue-import wiring (reading the founder's Tes catalogue, retaining unmapped paths verbatim, raising inbound items): M1j.
- Resource-type vocabulary seeding: M1j, when the duplicator consumes it.
- The reconciliation queue UI: M1i.
- Drain instrumentation across the first ten migrations (the kill-gate measurement itself): M1j; M1g ships the counters it reads.
- A Kani harness for the crosswalk: superseded by the exhaustive test over the captured finite domain, exactly the trade the engineering charter anticipated.
- Probing the NZ-inventory currency: an operator probe before M1j publishes; `CurrencyRule::Unmeasured` keeps the gate closed until it lands.
