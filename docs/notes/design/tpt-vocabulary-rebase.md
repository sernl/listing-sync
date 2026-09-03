# Phase 3: re-basing the canonical vocabulary on TPT

Implementation design for Phase 3 of the re-baselined plan (`docs/notes/design/vendoo-for-teachers-rethink.md:392-396`).
Four deliverables: invert the axes still minted from Tes, add the per-seller override layer, route the axes not yet bound, and add the drift job.
Everything below is written for an implementer with no other context, and every claim about the tree carries a file and line.

## The research overstated the inversion, and the correction narrows the work

`docs/research/rethink/cross-marketplace-mapping-tpt-base.md:258-259` says the canonical grade vocabulary is minted from Tes `yearGroups` and that the direction has to invert.
That is no longer true of the code.
`crates/tam-taxonomy/src/grades.rs:326-330` names `mint_from_tpt` "the base pass": every TPT grade option becomes a canonical term under TPT's own identity and TPT's own label whether or not Tes pairs it, and `mint_tes_extensions` (`crates/tam-taxonomy/src/grades.rs:381`) adds the fifteen `yearGroups` rows TPT has no option for.
The phase axis is already TPT-based, and `crates/tam-taxonomy/src/grades.rs:71-74` says so in its own words.
Flag this contradiction to the founder rather than treating the research as current: the research file describes the tree before the grade derivation landed, and the decision record (D13, same memo line 464) is the newer of the two.

What has not inverted is subject and topic.
`crates/tam-taxonomy/src/tes.rs:534-537` mints every canonical subject and topic id as UUIDv5 over `tes:{gb_native_id}`, `crates/tam-taxonomy/src/tes.rs:204-215` derives the whole relation from the two Tes captures alone, and `crates/tam-taxonomy/src/tes.rs:429-433` gives `Market` exactly two variants, `Gb` and `Nz`.
No subject or topic edge into TPT exists anywhere in the tree.
So the inversion is one derivation, not four.

## Axis by axis

Subject and topic invert.
A new pure module `crates/tam-taxonomy/src/subjects.rs` derives them from `docs/design/data/tpt-vocabulary.json`, whose `taxonomyTags.options` holds 358 facets addressed by slug, with `parentId` naming another slug rather than a `legacyId`.
Within `category: PreK-12-Subject-Area` there are 140 facets: 20 with no parent, which become canonical subjects, and 120 children, which become canonical topics; seven of the 140 carry `isHidden: true` and are minted as terms but recorded `NoCounterpart` for write, because a hidden facet reads back and is not offered on create.
Identity is the one thing that must not move: `canonical_term.id` is referenced by `projection_edge.from_term`, `projection_no_counterpart` and `reconciliation_item` (`crates/tam-storage/migrations/0013_taxonomy_hub.sql:7-63`), so a paired term keeps the id `canonical_id` already gave it and only a TPT-only facet mints a new one, as `derived("tpt-subject:{slug}")` in the style of `crates/tam-taxonomy/src/grades.rs:765-773`.
Label authority does flip, and costs nothing, because `crates/tam-storage/src/taxonomy.rs:123-125` re-seeds with `ON CONFLICT (id) DO UPDATE SET label = EXCLUDED.label, parent = EXCLUDED.parent`.
The pairing between the 20 TPT roots and the 43 Tes GB subjects is authored, as `GRADE_PAIRS` is authored (`crates/tam-taxonomy/src/grades.rs:75`), and it is the only authored artefact the inversion adds; `Exact` where one TPT root denotes one Tes subject, `Broader` where it denotes several, nothing where the derivation cannot say.
`derive_crosswalk` stays and keeps doing the Tes-to-Tes half, because a GB-to-NZ edge is still needed and the inbound direction still ingests Tes ids; `subjects.rs` consumes its output rather than replacing it.
Every Tes subject or topic no TPT facet pairs is minted as an extension term under its existing Tes id and gets `NoCounterpart` against `(Tpt, Subject)` and `(Tpt, Topic)`, which is exactly the shape `mint_tes_extensions` already uses.

Phase needs no inversion and one addition: the TPT grade relation routes into the three Tes inventories (`crates/tam-taxonomy/src/grades.rs:98-100`) and nothing else, so a fourth target inherits it for free and no code changes here in Phase 3.

Licence does not invert and must not.
`crates/tam-domain/src/registry/tpt.rs:129` declares `AxisAbsent(TermKind::Licence)` because TPT holds no licence field anywhere on its wire, and `crates/tam-taxonomy/src/licences.rs:29-30` mints the seven tokens for the three Tes inventories.
A TPT-based canonical model is therefore a superset only up to this axis, and licence is the standing counterexample the design keeps visible rather than resolves.

Resource type is bound in the registry and not routed.
`crates/tam-domain/src/registry/tpt.rs:93-119` binds it to `taxonomyTags`, `crates/tam-domain/src/registry/tes.rs:203-238` binds it to `mainType` at `Cardinality::One`, and `crates/tam-taxonomy/src/listing.rs:44-49` excludes it from `ROUTED_AXES` because its crosswalk is unseeded.
Phase 3 seeds it: 71 `Type-of-Resource` facets, 10 roots and 61 children, against the nine writable Tes `mainType` values that `docs/design/data/tes-vocabulary.json` records under `resourceTypes` (33 rows filtered by the uploader to `99000 < id < 99010`).
Many-to-one is legal only through `Broader` (`docs/design/taxonomy-projection.md:36-37`), which is what 71 onto 9 has to be, and `Cardinality::One` against a resolved set of several is already the cap-overflow election path in `crates/tam-taxonomy/src/project.rs:167`.
D13 denies a Type-of-Resource control on the TPT create route (`docs/notes/design/vendoo-for-teachers-rethink.md:464`), so TPT's own binding is declared `FieldDirection::ReadOnly` (`crates/tam-domain/src/registry/mod.rs:109-116`) and the axis is read from TPT and written to Tes, never the reverse.
Adding `TermKind::ResourceType` to `ROUTED_AXES` is the last step of that work and not the first.

## The categories that stay unbound, and where each routes

The remaining 147 facets are `Format` (26), `theme` (39), `supports` (31), `audience` (10), `programs-and-methods` (6), `language` (3), `Price-Range` (5), `topic` (6, all hidden) and `Featured` (1, hidden).
None of them routes to an equivalence axis in Phase 3, and the reason is that no measured target vocabulary pairs them: Tes binds five axes and no more (`crates/tam-domain/src/registry/tes.rs:203-238`), and Etsy binds none at all and declares none absent (`crates/tam-domain/src/registry/etsy.rs:36-38`), which is the unmeasured case that blocks.
They route instead as TPT natives on `taxonomyTags`, carried on the product, written to TPT, and disclosed as a loss on any projection out of it.
`audience` is the one to watch, because `crates/tam-taxonomy/src/grades.rs:66-69` already notes that Homeschool and Staff are `audience` facets the grade selector offers, and routing them as grades would be forcing an edge; they stay natives until a target measures an audience axis.
Adding a sixth `TermKind` is deliberately not in Phase 3: `TermKind` lives in `crates/tam-types/src/lib.rs:511-518`, and a new variant widens three CHECK constraints (`canonical_term_kind` and `projection_edge_term_kind` in `crates/tam-storage/migrations/0020_product_rights_residue.sql:76-82`, `election_rule_axis` in `crates/tam-storage/migrations/0023_elections.sql:44-46`) and regenerates `web/src/lib/generated/vocab.ts`, which is a blast radius the phase does not need.

## The per-seller override layer

Storage is a new table `projection_override`, keyed `(org_id, inventory, axis, from_term)`, holding `to_segments text[]`, `to_native_id text`, `kind text` constrained to `exact` and `broader`, and the same four-column `Decider` split `projection_edge` uses (`crates/tam-storage/migrations/0013_taxonomy_hub.sql:14-17`).
It is tenant data under the forced null-safe RLS policy, unlike `projection_edge`, which is global reference data; `election_rule` (`crates/tam-storage/migrations/0023_elections.sql:21-60`) is the row-level precedent to copy, including its practice of pinning the axis spelling in a CHECK.
It carries no reverse-uniqueness index, because the global exact-reverse index (`crates/tam-storage/migrations/0013_taxonomy_hub.sql:40-42`) enforces a property of the shared relation and a seller may legitimately point two of their own terms at one target path.

The domain type is `ProjectionOverride` in `crates/tam-domain/src/equivalence.rs`, beside `ElectionRule`, and `ListingContext` (`crates/tam-taxonomy/src/listing.rs:102-118`) gains one field, `overrides: &'a [ProjectionOverride]`, alongside `edges`, `no_counterparts`, `rules` and `settled`.
Precedence is: an override for `(org, inventory, axis, term)` wins over every global edge for that pair; absent one, the relation decides exactly as it does today.
`project_axis` records which won on `AxisOutcome` (`crates/tam-domain/src/equivalence.rs:234`) as a `decided_by_seller: bool`, so the field diff can say "you set this" rather than "the relation says this", and a later re-poll that changes the global relation cannot silently change a published listing.
An override never suppresses a loss record and never converts a block into a proceed on a non-delegable axis: `resolution_for` (`crates/tam-domain/src/equivalence.rs:274-281`) already makes `Delegation::Never` win over any opt-in, and the override table inherits that by refusing `axis = 'licence'` in a CHECK, in the same two-layer form `election_rule_licence_never_delegated` uses.

The API is one route, `POST /{version}/mappings/overrides`, registered beside `/{version}/elections/rules` (`crates/tam-api/src/lib.rs:252`) and handled by an `upsert_override` modelled on `crates/tam-api/src/resources.rs:1041`.
Body is `{ inventory, axis, from_term, to: { segments, native_id }, kind }`; the handler runs `check_native_ids` on the single edge before it writes, exactly as the reconciliation queue's resolution does (`crates/tam-taxonomy/src/provenance.rs:11-16`).
A `GET` on the same path lists the calling org's overrides.

The console screen is Templates, `web/src/routes/templates/+page.svelte`, which today is a thirteen-line placeholder whose own description already names this feature: "Reusable sync presets — licence choices, category mappings, pricing rules".
It becomes a per-marketplace list of the seller's own overrides with an add, edit and remove, reading the new route through `web/src/lib/api.ts`.
It is not Reconciliation (`web/src/routes/reconciliation/+page.svelte`), because that screen drains global gaps the founder answers once, and a seller override is neither global nor a gap.

The per-seller shelf mapping the rethink memo calls non-optional (`docs/notes/design/vendoo-for-teachers-rethink.md:224`) is a different table and is out of Phase 3: Etsy shop sections and TPT `sellerCustomCategories` are populated by reading the target, and `docs/design/data/tpt-vocabulary.json` records under `sellerCustomCategories` that no option set exists to seed.

## The drift job

A new binary crate `crates/tam-vocab-drift`, modelled line for line on `crates/tam-standards-fetch/src/main.rs:1-12`, which is the tree's existing precedent for a crate that acquires data and writes a committed capture.
It compares a freshly captured vocabulary against the committed one under `docs/design/data/` and diffs by native identifier, which is the mechanism `Residue` and `MismatchReason` already implement (`crates/tam-taxonomy/src/tes.rs:90-155`).
Three outcomes, and each has a settled response: a facet the target has added appears as a term with no outbound edges and enters the reconciliation queue as an operator question; a value that has disappeared appears as an edge pointing at an identifier the new capture does not hold, which stops the edge being used and raises an item, never guesses a replacement; a relabelled value with a stable identifier touches nothing structural, because labels are read out of the captures by `native_label` (`crates/tam-taxonomy/src/native_labels.rs`) rather than stored beside the terms.

Where it runs is decided by D1 and not by convenience.
The diff itself is pure and runs server-side against captured vocabularies, on a cron-shaped schedule, and it contacts nothing.
The re-capture is a marketplace request, so for TPT and Tes it originates on a seller's device under the seller's own session and is uploaded to the control plane as a capture artefact; the server never issues it.
For an API-branch marketplace such as Etsy the re-capture runs server-side under the issued token, which is the same fork the registry already encodes as a transport class (`crates/tam-domain/src/registry/mod.rs:237-241`).
The report lands as a file under `docs/design/data/drift/{inventory}-{date}.json` plus a non-zero exit status, so a `just` recipe is the alerting surface, exactly as `tam-canary` uses its exit status as the alert.
Cardinality and requiredness drift is invisible to this diff, because those are properties of the form rather than of the vocabulary; the design's answer is to treat the first failed create after a re-poll as a registry question rather than a retry, and that belongs to the adapter, not here.

## The kill gate and the exact test that decides it

The kill gate is a residue report showing the TPT-to-Tes re-derivation loses a mapping the current direction holds.
`derive_subject_crosswalk` returns `Residue { tpt_only, tes_only, mismatched }` in the shape `crates/tam-taxonomy/src/tes.rs:143-156` already defines, and the seeder prints it beside the two figures it prints today (`crates/tam-taxonomy-seed/src/main.rs:80-84` and `:126`).

The deciding test is `no_edge_the_tes_derivation_holds_is_lost_by_the_inversion`, in `crates/tam-taxonomy/src/subjects/tests.rs`, and it is a pure test over the committed captures with no database.
It runs `derive_crosswalk` over `tes-taxonomy-GB.json` and `tes-taxonomy-NZ.json`, runs `derive_subject_crosswalk` over the same two plus `tpt-vocabulary.json`, and asserts that every `(from, to.vocabulary, to.segments)` triple in the first appears in the second with an edge kind no weaker than the original — `Exact` may not become `Broader`, and neither may become absent.
It fails under any incorrect implementation that drops a pairing, renumbers a canonical id, or narrows an edge, which is what makes it severe rather than decorative.
The counts it pins are the ones the captures state: 43 subjects and 453 topics in GB, 43 and 451 in NZ, against 20 TPT subject roots and 120 TPT subject children.
If it fails and the loss is real rather than a defect, Phase 3 stops and the founder decides; the plan says so at `docs/notes/design/vendoo-for-teachers-rethink.md:394`.

## The projection tests re-run in the inverted direction

`crates/tam-taxonomy/tests/crosswalk.rs` has six tests that pin the current direction, and each has an inverted counterpart in the new module rather than an edit in place, so both directions stay asserted while the re-seed is in flight.
`every_subject_pairs` becomes `every_tpt_subject_root_pairs_or_is_recorded_absent`, mirroring `every_tpt_grade_is_paired_or_recorded_absent_and_never_both` (`crates/tam-taxonomy/src/grades/tests.rs:173`).
`the_derivation_seeds_the_pinned_counts` gains the TPT figures above.
`every_derived_edge_round_trips` is the one that must hold unchanged, because it is the round-trip law `docs/design/taxonomy-projection.md:40` states, and inversion may not weaken it.
`the_contested_path_is_withdrawn_from_both_markets` and `the_terms_insert_before_their_parents_do_not` carry over as they are, and the second matters more after inversion because TPT topics now have TPT subject parents.
`the_seeded_grade_relation_holds_no_ambiguity` and `one_term_holds_at_most_one_projecting_edge_into_each_vocabulary` (`crates/tam-taxonomy/src/grades/tests.rs:482,501`) get subject and topic twins, because the single-valued index (`crates/tam-storage/migrations/0021_projection_edge_single_valued.sql:37-38`) rejects the seed otherwise.

## Every path the implementation touches

New: `crates/tam-taxonomy/src/subjects.rs`, `crates/tam-taxonomy/src/subjects/tests.rs`, `crates/tam-vocab-drift/` (Cargo.toml and src/main.rs).
Edited in the pure core: `crates/tam-taxonomy/src/lib.rs`, `crates/tam-taxonomy/src/tes.rs`, `crates/tam-taxonomy/src/listing.rs`, `crates/tam-taxonomy/src/project.rs`, `crates/tam-taxonomy/tests/crosswalk.rs`, `crates/tam-domain/src/equivalence.rs`, `crates/tam-domain/src/registry/tpt.rs`.
Edited elsewhere: `crates/tam-taxonomy-seed/src/main.rs`, `justfile`, `web/src/routes/templates/+page.svelte`, `web/src/lib/api.ts`, `Cargo.toml` workspace members.
`docs/design/data/tpt-vocabulary.json`, `tes-taxonomy-GB.json`, `tes-taxonomy-NZ.json` and `tes-vocabulary.json` are read and not modified.

The exclusions hold for three of the four named crates: nothing here touches `crates/tam-engine`, `crates/tam-engine-driver` or `crates/tam-api` beyond the two points below, and the purity gate (`justfile:35-52`) keeps `tam-taxonomy` and `tam-domain` free of tokio, reqwest and sqlx, which the new module and the new domain type both respect.

Contact with `crates/tam-api`, listed separately as required: `crates/tam-api/src/resources.rs` and the one route line in `crates/tam-api/src/lib.rs` for the override endpoint, plus `web/src/lib/generated/vocab.ts` if and only if a Rust enum in the typegen set moves, which under the recommendation in Q3 below it does not.
`crates/tam-api/src/bin/typegen.rs` itself is unchanged.

`crates/tam-storage` cannot be excluded, and this is the one deviation from the stated scope: a per-seller override is durable tenant state, so it needs `crates/tam-storage/migrations/0046_projection_override.sql` and a repository module `crates/tam-storage/src/overrides.rs`, plus the two-tenant RLS case in `crates/tam-storage/tests/rls_matrix.rs`.
There is no way to build the override layer without them, so it is raised as Q1 rather than resolved by interpretation.

## Founder decisions this design needs

Q1. Does the per-seller override layer take its two `crates/tam-storage` paths, against the stated exclusion of that crate?
Recommended: yes, because the alternative is either no durable override or an override stored outside RLS, and the second is worse than the scope breach.

Q2. Does subject and topic inversion preserve the existing Tes-derived canonical ids for paired terms, or re-mint everything under TPT identity?
Recommended: preserve, because `canonical_term.id` is referenced from four tables and every stored product, and re-minting is a data migration the phase does not otherwise need.

Q3. Do the 147 unbound TPT facets get a new `TermKind` in Phase 3?
Recommended: no; carry them as natives on `taxonomyTags`, because no measured target pairs them and a sixth variant widens three CHECK constraints and regenerates the client vocabulary for no present gain.

Q4. Does the drift job's TPT and Tes re-capture wait for the Phase 2 desktop client, given D1 forbids the server issuing it?
Recommended: yes; land the pure diff and the Etsy-branch server capture in Phase 3, and wire the device-originated capture when the desktop client exists.

Q5. Is the authored TPT-root-to-Tes-subject pairing table the founder's to author, as `GRADE_PAIRS` was?
Recommended: yes for the 20 roots, which is a single sitting; the 120 topic children derive against the pairing rather than being authored one by one.

## Build order

Step 1, the pairing table and `derive_subject_crosswalk`, pure and unseeded.
Verification: `no_edge_the_tes_derivation_holds_is_lost_by_the_inversion` under `cargo nextest run -p tam-taxonomy`, which fails if any current edge is dropped or weakened.

Step 2, the seeder prints the new residue and the re-seed is idempotent.
Verification: two consecutive `tam-taxonomy-seed` runs against a fresh dev database, the second reporting every term and edge as existing rather than inserted, which fails if an id moved.

Step 3, route resource type: seed the 71-onto-9 relation, declare TPT's binding read-only, add `TermKind::ResourceType` to `ROUTED_AXES`.
Verification: a projection of a TPT product carrying four resource-type facets into Tes raises one over-cap election with all four as candidates and publishes nothing narrowed, which fails under any implementation that truncates.

Step 4, the override table, the domain type, and `project_axis` precedence.
Verification: `just db-test` two-tenant RLS isolation, plus a pure test that an override for org A changes A's projection and not B's, which fails if the override is read outside the tenant scope.

Step 5, the override endpoint and the Templates screen.
Verification: `just check` and `just web-check`, the second failing if `vocab.ts` is stale (`justfile:286-289`).

Step 6, the drift job and its `just` recipe.
Verification: run it against the committed captures with one facet deleted from a copy, and assert a non-zero exit and one mismatch row; it fails if the diff reports clean.
