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
Within `category: PreK-12-Subject-Area` there are 140 facets: 20 with no parent, which become canonical subjects, and 120 children, which become canonical topics; Seven of the 140 carry `isHidden: true`, and all seven are roots rather than a spread across the level: `arts`, `other-art`, `phonics`, `holidays-seasonal`, `trigonometry`, `word-problems` and `phonics-and-phonemic-awareness`, which leaves thirteen visible roots.
Ruling C settles what they seed, against this design's own first answer: nothing.
`NoCounterpart` has no write-only form, because `crates/tam-taxonomy/src/project.rs:105-110` consults it only when the term holds no edge into the target at all, so a hidden facet either holds an `Exact` edge and is posted back on the next publish — sending a retired slug on create — or holds none and arrives as an unrecognised path retained verbatim on the product.
The second loses nothing, and because a minted id derives from the slug, un-hiding a facet later mints the same id it would have had now.
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

The remaining 127 facets are `Format` (26), `theme` (39), `supports` (31), `audience` (10), `programs-and-methods` (6), `language` (3), `Price-Range` (5), `topic` (6, all hidden) and `Featured` (1, hidden).
That count was stated as 147 before it was checked against the capture: 147 is the enumeration below plus the 20 `Grade-Level` facets, which `grades.rs` already binds, so it double-counted a bound category.
The nine categories below sum to 127, and 140 + 71 + 127 + 20 is the measured total of 358.
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

## What landed, and where the captures contradicted the design

Step 1 landed on 2026-09-03: `crates/tam-taxonomy/src/subjects.rs`, its tests, the authored pairing at `docs/design/data/tpt-tes-subject-pairs.json`, and two enabling changes — `TaxonomyTag` gains `parentId` and `isHidden`, and `tes::canonical_id` becomes `pub(crate)`.
`just check` is green: fmt, clippy, purity and 991 workspace tests, of which 131 are `tam-taxonomy`'s and 11 are new.

The pairing's shape is not the one this design assumed, because the two hierarchies do not align at either level.
Eight TPT *children* denote a Tes *subject* — `biology`, `chemistry` and `physics` under `science`, `geography`, `economics` and `psychology` under `social-studies`, `drama` and `music` under `performing-arts` — while only `physical-education` denotes one from the root, and two TPT roots, `health` and `speaking-and-listening`, denote Tes *topics*.
A twenty-root-against-forty-three-subject table states none of that, and forcing it through would have collapsed all eight exact pairings into `Broader` edges onto three coarse roots.
Four founder rulings on 2026-09-03 settled it: a row may cross levels in both directions; a facet's canonical `TermKind` is a per-term modelling choice rather than its `parentId` depth, which TPT's own flat `taxonomyTags` field permits; a hidden facet seeds nothing; and `for-all-subject-areas` holds the `Exact` claim on `Cross-curricular topics` while `not-subject-specific` reaches it through `Broader`.

Two mechanisms carry the rulings that this design did not name.
A root's row states the root's whole denotation and the derivation *withdraws* from it any node one of that root's own children denotes exactly, because a term holding an `Exact` and a `Broader` edge onto two different TPT paths projects `Ambiguous` and blocks the publish; eight withdrawals fire.
And the 120 children are paired by normalised description against the Tes nodes inside their root's denotation rather than authored, which is what the design meant by deriving them against the pairing: 8 match a Tes subject, 19 a Tes topic, one of those (`handwriting`) matching two, and the remaining 93 reach nothing and become residue rather than a guessed edge.

The figures the derivation seeds, all pinned in `the_derivation_seeds_the_pinned_counts`: 601 terms (496 from the Tes half, 105 minted), 1,146 edges (986 from the Tes half, 133 `Exact` into TPT — one for each writable facet — 25 `Broader` from the Tes side, and the two Tes paths `not-subject-specific` reaches), 443 no-counterpart records, and a residue of 95 facets reaching no Tes node, 443 Tes nodes reaching no facet, 8 withdrawals and 7 hidden facets seeded neither way.

The kill gate passes and did so without ever being close, because consuming `derive_crosswalk` rather than replacing it makes the property structural: every edge the Tes-to-Tes derivation holds is present, on the same term, at the same path, as `Exact`.
`no_edge_the_tes_derivation_holds_is_lost_by_the_inversion` therefore earns its place as a regression guard rather than as a discovery, and it is joined by `every_existing_canonical_id_survives_unchanged`, which pins every base term's id, kind and parent.

Step 5 landed as one change, the route and the screen together, and one thing in it is a choice rather than a derivation.
The two override kinds are shown to a seller as "Same as" for `Exact` and "Belongs under" for `Broader`, with one hint line on the second: buyers on this marketplace see the heading, not your term.
The domain names the two kinds and no source names the words for them, so this wording is the team lead's and the founder may replace it; it is recorded here as a founder item rather than left as a silent choice in a Svelte file.

Step 5 also bounds the paths a client may name, on both handlers that make one durable: the seller's override and the reconciliation queue's resolution.
Neither was bounded before, and `to_segments` carries only a non-empty CHECK, so an unbounded array reached the database on a caller's say-so.
The limits are eight segments and two hundred characters each, measured rather than picked: the deepest path in the committed vocabularies is two segments, a Tes subject and its topic, and the longest label is 69 characters, in TPT's licence list.
Both are limits the founder may replace, and both refusals are validation errors naming what was wrong.

## Step 2: the seeder, and what the database confirmed independently

The seeder now derives the subject and topic axes through `derive_subject_crosswalk` rather than `derive_crosswalk`, which it no longer calls directly.
That makes the TPT vocabulary and the authored pairing required arguments where they were optional, so the usage becomes `tam-taxonomy-seed <db-url> <gb.json> <nz.json> <tpt-vocab.json> <tes-vocab.json> <subject-pairs.json>`.
The optional form existed for a run that seeded the subject and topic crosswalk alone, and it is gone deliberately: with TPT as the base, such a run seeds a relation in which no TPT facet resolves, which is a half-seeded hub rather than a smaller one.
The market-to-market residue the seeder printed before is not lost, because `derive_subject_crosswalk` returns it unchanged on `tes_residue`, and both residues now print side by side.

The figures the seeder prints therefore change, which is the visible part of the switch and is recorded here rather than left to be noticed.
Its subject and topic line read 496 terms and 986 edges on a fresh database and named no absences at all, because the Tes-to-Tes derivation emitted none for these two axes.
It now reads 601 terms, 1,146 edges and 443 absences, and gains a second residue line for the TPT base beside the market-to-market one, whose own figures are unchanged at 2 GB-only, 0 NZ-only and 4 mismatched.
The grade and licence lines are untouched.

Idempotency holds, against a database created fresh for the purpose rather than by resetting the shared dev volume.
The first run inserted 601 terms, 1,146 edges and 443 absences with nothing existing; the second inserted nothing and reported every one of them as existing, which is the property that fails if an id moved.
The grade and licence halves behave the same way: 40 terms, 180 edges and 30 absences, then 7 terms and 21 edges, all existing on the second pass.

Three properties the database checked that no pure test can.
Every edge was accepted, so the `projection_edge_exact_reverse` index and the `projection_edge_single_valued` index both admitted the whole seed rather than rejecting it at insert.
`check_native_ids` passed over the new edge set, so no TPT path carries an identifier that marketplace did not issue.
And the seeder's own ambiguity count, which is a `GROUP BY` over the stored relation rather than a fold over the derivation, reported zero, agreeing with `the_seeded_subject_relation_holds_no_ambiguity`.

The eight withdrawal rows print individually rather than only as a count, because the pairing table is authored and awaiting founder confirmation, and each row is a claim one row made that another overrode.
They are the evidence to send with the table: Drama and Music withdrawn from `performing-arts`, Geography, Economics and Psychology from `social-studies`, and Biology, Chemistry and Physics from `science`.

## Step 3: routing resource type, and the half of it that cannot be done as written

Seventy-one TPT `Type-of-Resource` facets now reach the nine writable Tes `mainType` values.
Every facet mints its own canonical term holding an `Exact` edge into TPT and a `Broader` edge into each of the three Tes inventories, because seventy-one onto nine can only be many-to-one and many-to-one is legal through `Broader` alone.
The authored half is `docs/design/data/tpt-tes-resource-type-pairs.json`, twenty-one rows rather than seventy-one: ten roots, eleven child overrides, and fifty children that inherit their root.
A child is named only where inheriting would file it under a value Tes distinguishes from the right one, which is why `games` is named and `posters` is not.
The hidden facet `independent-work` seeds nothing, on the rule ruling C set.
`TermKind::ResourceType` joins `ROUTED_AXES`, and the seeder gains the derivation and a seventh argument.

The figures: 70 terms and 280 edges, 70 `Exact` into TPT and 210 `Broader` into Tes, inserted on the first run and all existing on the second.
Tes `Assembly` (99001) is reached by no TPT facet, which is a real absence rather than a defect and is printed as one.

The verification the design named passes.
A product carrying four resource-type facets that resolve to four different Tes values raises exactly one `ElectOne` election with all four as candidates and an empty resolved set, so no partially narrowed value is publishable.
Its companion tells the two apart: four facets that broaden onto one Tes value resolve to that value with no election and one merged loss naming all four terms it broadened away, which shows the election is a property of the resolved set rather than of the number of terms carried.

What could not be done as written is the read-only declaration.
The design says TPT's own binding is declared `FieldDirection::ReadOnly`, but `FieldDirection` is a field of `NativeField` rather than of `AxisBinding` (`crates/tam-domain/src/registry/mod.rs:108-117`, `:190-199`), and TPT binds all four of its axes — subject, topic, resource type and phase — to the single native field `taxonomyTags`, currently `Both`.
Setting that field read-only would stop TPT writing subjects, topics and grades too, and `AxisBinding` has nowhere to record a direction for one axis alone.
So the declaration is not made, and the fact D13 wants is instead a property of the relation: inbound resolution follows `Exact` edges alone, no canonical term claims a Tes `mainType` path as `Exact`, and therefore a Tes-sourced product ingests no resource-type term and carries none into a TPT create.
`no_tes_resource_type_value_ingests_to_a_canonical_term` holds that over every edge the derivation emits.
This is weaker than the design intended in one specific way, and the difference is worth naming: the property is enforced by the shape of the data rather than declared in the registry, so a future edge authored through the reconciliation queue could break it without any registry check objecting.
Closing that gap means adding a per-axis direction to `AxisBinding`, which touches `registry/mod.rs` and every `AxisBinding` literal in every registry file.

That is now a recorded deferral rather than an open question.
Ruled on 2026-09-03: the relation's shape carries D13 for the present, and the per-axis direction on `AxisBinding` becomes its own change once the API stream's fence over `registry/mod.rs` lifts.
Until then the fact has exactly one enforcement point, and it is a test rather than a type: `no_tes_resource_type_value_ingests_to_a_canonical_term` in `crates/tam-taxonomy/src/resource_types/tests.rs`.
Anyone authoring a resource-type edge into a Tes `mainType` path as `Exact` — through the reconciliation queue, or by hand — breaks D13 and that test is what will say so.

## Step 4: the per-seller override layer

Storage is `projection_override`, keyed `(org_id, inventory, axis, from_term)`, tenant data under the forced null-safe policy with `election_rule` as its row-level precedent rather than `projection_edge`.
It carries no reverse-uniqueness index, because the global exact-reverse index protects a property of the shared relation — inbound resolution is the reverse of the `Exact` edges and must stay a function — and an override is consulted outbound only and never inverted.
`kind` admits `exact` and `broader` and not `narrower`, because a narrower edge never participates in an outbound projection, so an override producing one would be a decision the seller could make and never observe.
Licence is refused in a CHECK and again in `ProjectionOverride::new`, in the two-layer form `election_rule` uses, because the domain check passes for anything that writes the row directly.

Precedence is applied by removing overridden terms from what reaches the relation rather than by adding the override beside the global edges.
That distinction is the whole correctness of the layer and is easy to get backwards: an override folded in beside an existing edge gives one term two paths in one vocabulary, which projects `Ambiguous` and blocks the publish, so a seller stating a preference would stop their own listing going out.
An overridden value is added before the cardinality election, so a seller's choice counts against the target's cap exactly as a resolved one does; and an overridden broadening still discloses its loss, because an override never suppresses one.
`AxisOutcome` gains `decided_by_seller`, so a field diff can say "you set this" rather than "the relation says this", and a later re-poll that changes the global relation cannot silently change what an overridden term published as.

The layer is additive at every call site, and that shape was a correction rather than the first attempt.
Adding `overrides` to `ListingContext` broke every literal that builds it, including two in crates other streams were editing, and a field a caller silently gains is also a caller opted into a behaviour change it never asked for.
So the overrides travel as a parameter: `project_listing_with_overrides` and `project_axis_with_overrides` carry them, and `project_listing` and `project_axis` delegate to those with an empty set.
Every construction of `ListingContext` and `AxisRequest` that compiled before still compiles and still behaves identically, and the opt-in is the call a caller makes rather than a field it fills in.

## Step 6: the drift job

The diff is `crates/tam-taxonomy/src/drift.rs` and the binary around it is `crates/tam-vocab-drift`, split that way for the reason the design gives for keeping the comparison pure: the re-capture is a marketplace request and D1 puts that on the seller's own device for a marketplace with no official API, so the alerting surface must not be able to become the thing that issues it.
The binary holds no HTTP client and no runtime, and `just purity` now asserts that by naming `tam-vocab-drift` in its subgraph, which turns the D1 boundary for this crate into a gate rather than a convention.
It is the difference from `tam-standards-fetch`, its model in every other respect: that one holds `reqwest` and touches one host.

Option sets are discovered rather than listed.
Any top-level key whose value is an object carrying an `options` object is a set, which reaches all ten in the TPT capture and all six in the Tes capture with no table to maintain, and diffs a set the platform adds tomorrow on the day it appears.

The design's three outcomes are four rows, because a stable identifier whose payload moved is two different facts rather than one.
`Added` and `Removed` are what the design describes.
A payload change splits into `Restructured`, where `parentId` or `category` moved and the facet therefore changed root or axis, and `Relabelled`, where neither did.
Only the first three block: a relabelled value with a stable identifier touches nothing structural, because labels are read out of the captures by `native_label` rather than stored beside the terms.
The split is not decoration — under a single "changed" row a re-parented facet and a renamed one would alert identically, and the derivations treat them completely differently, because a child is paired inside its root's denotation.

The report lands at `docs/design/data/drift/{inventory}-{date}.json` and the exit status is the alerting surface, as it is for `tam-canary`; `just vocab-drift` is the recipe.
The date is an argument rather than a clock read, so two runs over the same pair of captures produce byte-identical reports and a committed drift file can be compared rather than merely read.

Verified twice over.
Nine pure tests, including the step's own: one facet deleted from a copy yields exactly one row and a run that does not pass, and it fails if the diff reports clean.
And end to end against the real binary: the committed TPT capture against itself exits zero over ten sets, the same capture with `escape-rooms` deleted exits non-zero with one `Removed` row, and the Tes capture against itself exits zero over six.

What is deliberately not here is either half of the capture.
Q4 defers the device-originated TPT and Tes re-capture to the Phase 2 desktop client.
The Etsy-branch server capture is deferred too, for a reason Q4 did not anticipate: `crates/tam-domain/src/registry/etsy.rs:36-38` binds no equivalence axis and declares none absent, so there is no Etsy vocabulary to diff yet, and writing the fetch before the binding exists would be building against an unmeasured target.

## Step 5, handed over: the route and the screen

Step 5 is the only part of the build order this stream did not build, because the override endpoint lives in `crates/tam-api` and the Templates screen in `web/`, and both belong to the stream that owns them.
Everything behind them is built, tested and landed, so what follows is the contract rather than a sketch.

This paragraph recorded, while it was true, that nothing called the override layer: `crates/tam-engine/src/seed.rs` and `crates/tam-import/src/lib.rs` both called `project_listing`, which delegates with an empty override set, so an override a seller recorded changed no projection.
Both callers are wired now, as 12d1ca50 and 37b72408, so the layer is live end to end and a seller's override reaches the projection.
It is kept rather than deleted because it is why step 5 was sequenced behind the wiring: the alternative was shipping a control that silently did nothing, and the landing order is what avoided it rather than a note on the screen apologising for it.

The route is `POST /{version}/mappings/overrides`, registered beside `.route("/{version}/elections/rules", post(resources::upsert_rule))` at `crates/tam-api/src/lib.rs:256`, with `GET` on the same path listing the calling org's overrides and `DELETE` withdrawing one.
The handler takes the shape `upsert_rule` takes at `crates/tam-api/src/resources.rs:1161`: `State(state): State<AppState>`, `context: OrgContext`, `Json(body)`, returning `Result<StatusCode, APIError>`.
The body carries `inventory`, `axis`, `from_term`, `to: { segments, native_id }` and `kind`, which is `exact` or `broader`.

Four things the handler must do, each for a stated reason.
The organisation and the user come from `OrgContext` and never from the body, so `decided_by` is `Decider::Human { user: context.user, org: context.org }` exactly as the rule handler builds it.
`ProjectionOverride::new` is the only way to construct the value, because it is the domain half of the licence refusal and the database CHECK is the other half; its two errors, `LicenceNeverOverridden` and `EmptyPath`, map to validation errors rather than to a five-hundred.
`check_native_ids` runs before the write, over a one-element slice holding the `ProjectionEdge` the override would produce, which is the same check the reconciliation queue's resolution runs at `crates/tam-taxonomy/src/provenance.rs:11-16` and the reason a wrong tag never reaches a live listing.
And the write is `OverrideRepo::upsert`, the read `OverrideRepo::for_org`, the withdrawal `OverrideRepo::remove`, all of which pin `app.current_org` themselves, so the handler passes `context.org` and does not open its own transaction.
Two behaviours the handler has beyond that list, both tested.
A withdrawal matching no override answers 204 rather than 404: the seller's intent is that no override stand for that term, and after the call none does, so answering not-found would make a client distinguish two states it should treat identically and turn a double-click into an error.
And a term the taxonomy no longer holds is refused as a validation error rather than surfacing as a fault, recognised by the `projection_override_from_term_fkey` constraint name: the screen picks from a cached term list, so a tab left open across a taxonomy change names a term that has since gone, and a seller meeting that is owed "reload and pick it again" rather than a five-hundred.

The screen is `web/src/routes/templates/+page.svelte`, today a thirteen-line placeholder whose own description already names the feature: "Reusable sync presets — licence choices, category mappings, pricing rules".
It becomes a per-marketplace list of the seller's own overrides with add, edit and remove, reading the route through `web/src/lib/api.ts`.
It is not the Reconciliation screen, and the distinction is worth keeping: that screen drains global gaps the founder answers once for everyone, and a seller override is neither global nor a gap.
Two details the screen should carry: the axis selector must exclude licence, because the constructor and the database will both refuse it and a control that always errors is worse than no control; and an override's effect is visible in the field diff through `AxisOutcome.decided_by_seller`, which is what lets the diff say "you set this" rather than "the relation says this".

Out of scope and deliberately so: the per-seller shelf mapping the rethink memo calls non-optional is a different table, because Etsy shop sections and TPT `sellerCustomCategories` are populated by reading the target, and `docs/design/data/tpt-vocabulary.json` records under `sellerCustomCategories` that no option set exists to seed.
