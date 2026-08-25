# M1b storage and tenancy implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** The full schema from `docs/design/schema.md` as forward-only migrations with forced row-level security on every tenant table, org-first repositories for the catalogue and mapping aggregates, a closed-world tenancy matrix test no future table can dodge, and the mechanisable half of migration discipline.

**Architecture:** Everything lands in `crates/tam-storage`, split into modules as it grows (`product`, `mapping`, `codec`). Tables whose DDL `schema.md` states are copied verbatim; tables it describes only in prose are derived here with the derivation noted inline. Repositories keep the M1a shape: `org: OrgId` first, tenant pinned per transaction, compile-checked queries with committed offline metadata.

**Tech Stack:** sqlx 0.8 offline-postgres, proptest against the live database for codec round-trips, the podman/ephemeral dev database from M1a.

## Global constraints

- Every migration is forward-only; a bad change is corrected by a new forward migration (schema.md "Migration discipline").
- Every tenant table (any table carrying `org_id`) gets `ENABLE` + `FORCE ROW LEVEL SECURITY` and the `app.current_org` policy, in the same migration that creates it.
- Repository methods take `org: OrgId` as the first positional parameter after `self`, always.
- Deferred aggregate invariants (payload non-empty, mismatch non-empty) are constraint triggers, exactly as schema.md specifies, because PostgreSQL cannot defer a CHECK.
- The gate stays green after every task: `just check`, and `just db-test` against a live database.
- Commit recipe (jj, signed): `jj describe -m "<msg>"`, `jj bookmark set main -r @`, `jj new`, `git push origin main`.
- Deferred out of M1b, recorded here so the omission is a decision: `billing_customer`/`subscription`/`usage_event` land with M1j (billing owns their shape); the broker-only database role for `connection_secret` lands with M1e (today the table exists under RLS with a comment naming the coming role split); `JobEventPayload` and the kind-set flake check land with M1d.

---

### Task 1: marketplace_inventory reference table

**Files:**
- Create: `crates/tam-storage/migrations/0002_marketplace_inventory.sql`

**Interfaces:**
- Produces: the `(code, marketplace)` reference rows the `mapping` and `job` FKs name: `(tes_gb, tes)`, `(tes_us, tes)`, `(etsy, etsy)`, `(tpt, tpt)`.

- [ ] **Step 1:** Write the migration: `marketplace_inventory (code text, marketplace text, PRIMARY KEY (code, marketplace))`, seeded with the four rows. Global reference data: no `org_id`, no RLS (the tenancy matrix test in Task 6 classifies it global).
- [ ] **Step 2:** `just db-migrate` applies clean on a reset database; `just db-test` still green.

---

### Task 2: catalogue tables and the aggregate ProductRepo

**Files:**
- Create: `crates/tam-storage/migrations/0003_catalogue.sql`
- Modify: `crates/tam-storage/src/lib.rs` (split: `src/product.rs`, keep conversions in `src/codec.rs`)
- Modify: `crates/tam-storage/tests/tenancy.rs`

**Interfaces:**
- Produces: `blob`, `product_file`, `product_term`, `grade_declaration`, `grade_declaration_path`, minimal `canonical_term` (global; M1g extends it), the `assert_product_has_payload` deferred triggers on both sides, and `ProductRepo` persisting the aggregate: `insert(org, &NewProduct)` where `NewProduct` now carries payload files (non-empty by construction), cover, previews, subjects, grades; `get(org, id) -> Option<ProductRecord>` rehydrates the whole aggregate.

- [ ] **Step 1:** Migration DDL. `blob` verbatim from schema.md. `canonical_term (id uuid PK, kind text, parent uuid NULL self-FK, label text)` — global, no RLS, derived minimal for the `product_term` FK. `product_file` derived from prose: compound product FK, `(org_id, hash)` FK to `blob`, `role` CHECK in (payload, preview, cover), `kind` CHECK in (pdf, pptx, docx, zip, image), scan columns rendering `ScanOutcome` with a totality CHECK, `deleted_at`. `product_term (org_id, product_id, term_id)` compound PKs and FKs. `grade_declaration` (source rendering `DeclarationSource`, derived low/high years with an ordering CHECK) and `grade_declaration_path` (position-ordered `VocabularyPath` rows: inventory, term kind, segments text[], native_id). RLS on every one of these except `canonical_term`.
- [ ] **Step 2:** The payload invariant: `assert_product_has_payload()` raising unless a live payload-role `product_file` row exists; deferred constraint triggers on `product` (insert/update) and on `product_file` (update/delete), verbatim in shape from schema.md.
- [ ] **Step 3:** Extend `NewProduct` (payload head + tail so the empty case is unrepresentable, mirroring `PayloadSet`), insert the whole aggregate in one transaction (blob upsert `ON CONFLICT DO NOTHING`, files, terms, grade declaration + paths), rehydrate in `get`. `just db-prepare` refreshes `.sqlx`.
- [ ] **Step 4:** Tests: a product without a payload row fails at commit (trigger severity, raw SQL); the repo round-trips the full aggregate; two files sharing a hash share one blob row per tenant; RLS probes cover `product_file` unfiltered.
- [ ] **Step 5:** Gate and commit: `feat(m1b): catalogue tables, payload trigger, aggregate ProductRepo`.

---

### Task 3: mapping, field_mismatch and the total binding codec

**Files:**
- Create: `crates/tam-storage/migrations/0004_mapping.sql`, `crates/tam-storage/src/mapping.rs`, `crates/tam-storage/tests/mapping_roundtrip.rs`

**Interfaces:**
- Consumes: `tam-domain` `Mapping`, `Binding`, `Verification`, `FieldPolicies`, `PublishMode`; `tam-marketplace` `RemoteLifecycle`, `RemoteListingId`.
- Produces: `MappingRepo::{insert, get, list_for_product}` (org-first) with a total codec between the domain `Mapping` and the columns; `tam-storage` gains its first `tam-domain`/`tam-marketplace` dependency.

- [ ] **Step 1:** Migration: `mapping` and `field_mismatch` verbatim from schema.md (three CHECK constraints, `mapping_mismatch_nonempty` trigger, compound FKs, RLS on both).
- [ ] **Step 2:** The codec: `Binding` ↔ (`binding_state`, remote id triple, `binding_attempt`, `first_seen_at`, `severed_at`, `sever_cause`) and `Verification` ↔ (`verify_state`, `verified_at`, `field_mismatch` child rows), total in both directions, decode errors as `CorruptRow` naming the field. `FieldPolicies` ↔ six columns.
- [ ] **Step 3:** Property test (live database, bounded cases): an arbitrary well-formed `Mapping` inserted and re-read is equal, including `Mismatched` verifications with their child rows.
- [ ] **Step 4:** Negative tests: a bound row without a remote id is refused by `mapping_binding_total`; a `mismatched` verify without a `field_mismatch` row is refused at commit by the trigger.
- [ ] **Step 5:** Gate and commit: `feat(m1b): mapping storage with a total binding codec, property-tested`.

---

### Task 4: job ledger, fencing and outbox tables

**Files:**
- Create: `crates/tam-storage/migrations/0005_job_ledger.sql`
- Modify: `crates/tam-storage/tests/tenancy.rs`

**Interfaces:**
- Produces: `job` (derived: org, id, inventory+marketplace FK, created_at — status stays computed, never stored), `job_item`, `job_event`, `org_event_counter (org_id PK, next_seq)`, `write_attempt` with its `write_attempt_one_in_flight` partial unique index, `outbox_message`. Tables only; the repositories are M1d's.

- [ ] **Step 1:** Migration with the schema.md DDL verbatim for `job_item`, `job_event`, `write_attempt`, `outbox_message`; derived minimal `job` and `org_event_counter`; RLS on every one.
- [ ] **Step 2:** Negative tests: two `in_flight` write_attempt rows for one mapping are refused by the partial unique index (the duplicate-upload storm, structurally); a second `job_item` with the same `(org, idempotency_key)` is refused.
- [ ] **Step 3:** Gate and commit: `feat(m1b): job ledger, write-attempt fencing and outbox tables under RLS`.

---

### Task 5: halts, budgets, connection, audit

**Files:**
- Create: `crates/tam-storage/migrations/0006_halts_connection_audit.sql`
- Modify: `crates/tam-storage/tests/tenancy.rs`

**Interfaces:**
- Produces: `inventory_halt` (global: inventory+marketplace key, raised_by, raised_at — no org, fleet scope), `org_halt`, `org_inventory_halt` (both tenant, RLS), `rate_budget` keyed `(org_id, connection_id, window_start)` per the schema.md keying decision, `connection` (state CHECK over the five `ConnectionState` values, `UNIQUE (org_id, marketplace)` recording the one-login assumption), `connection_secret` (wrapped DEK, nonce, ciphertext, AAD, key version; RLS; comment naming the M1e broker-role split), `field_audit` append-only.

- [ ] **Step 1:** Migration with the derived DDL; RLS on every tenant table.
- [ ] **Step 2:** Append-only enforcement: `REVOKE UPDATE, DELETE ON field_audit FROM tam_app`; test that an UPDATE as the application role is denied while INSERT and SELECT succeed.
- [ ] **Step 3:** Gate and commit: `feat(m1b): halt scopes, rate budget, connection and append-only audit tables`.

---

### Task 6: the closed-world tenancy matrix and migration-discipline lanes

**Files:**
- Create: `crates/tam-storage/tests/rls_matrix.rs`
- Modify: `justfile`

**Interfaces:**
- Produces: a structural test that fails when any table is added without a tenancy decision, and a `db-verify` lane asserting the committed offline metadata matches the source.

- [ ] **Step 1:** `rls_matrix.rs`: query `pg_class`/`pg_policy` for every public table; assert the set of table names equals an explicit allowlist partitioned into tenant and global; assert every tenant table has `rowsecurity`, `relforcerowsecurity` and at least one policy, and every tenant table's first policy references `app.current_org`. A new table not in either list fails the test, which is the closed world.
- [ ] **Step 2:** justfile: `db-verify` = `cargo sqlx prepare --check` inside `crates/tam-storage` against the live database; chain it into `db-test`. CI cannot run it (the sandbox has no database) — recorded here; it runs in the local lane where migrations are authored.
- [ ] **Step 3:** Full gate (`just check`, `just db-test`, `nix flake check` on the committed tree) and commit: `feat(m1b): closed-world RLS matrix and offline-metadata verification lane`.
