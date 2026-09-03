# Ingesting the four education-standards frameworks

This note records what Phase 4's standards ingest actually landed, what the data turned out to be, and what a picker and a projection must do about it.
The specification is `docs/research/rethink/education-standards-sources.md`; the decisions it implements are D18, D19 and D25 in `docs/notes/design/vendoo-for-teachers-rethink.md`.
Every count below was measured from the committed files rather than estimated, and where a measurement contradicts the research note it is flagged as such rather than reconciled quietly.

## What was ingested, and from where

Four frameworks, one source: the Common Standards Project's open API at `api.commonstandardsproject.com`, unauthenticated, plain JSON over HTTPS, no browser and no automation.

Common Core and NGSS come from the mirror because their owners publish no machine-readable form at all.
Virginia comes from the mirror because `doe.virginia.gov` returns HTTP 403 to every non-browser client.
Texas comes from the mirror by decision rather than by necessity: TEA publishes a CASE-certified API of its own, and D19 declines it so that TEA's terms of service are never agreed to, taking the mirror's CC BY licence instead.
That decision has a measured cost, recorded under "Where the mirror is thinner than its owner" below.

Common Core and NGSS are taken whole, since their jurisdictions carry only their own frameworks.
Texas and Virginia cannot be: their jurisdictions hold every vintage the mirror ever had and every subject, including 190 Texas career-and-technical sets.
So the ingest selects the current cycle of the four core academic subjects a teaching-resource seller tags at, by matching the mirror's own subject labels.
The lists are constants in `crates/tam-standards-fetch/src/main.rs`, and one of them carries a trailing space because the mirror's label does.
A retired vintage is deliberately not ingested: a listing tagged to a retired code is the migration problem the update strategy names, and seeding one would manufacture it.

## What landed, in numbers

Fetched 2026-09-02T14:39:39Z from 254 standard sets: 38 Common Core, 64 NGSS, 88 Texas and 64 Virginia.

| Framework | Sets | Nodes | Distinct codes | Addressable rows | Addressable codes | File |
|---|---|---|---|---|---|---|
| CCSS | 38 | 3,590 | 1,812 | 2,240 | 1,536 | 1.40 MB |
| NGSS | 64 | 3,350 | 1,858 | 440 | 208 | 1.07 MB |
| TEKS | 88 | 6,893 | 4,872 | 5,425 | 4,097 | 2.20 MB |
| VA SOL | 64 | 5,936 | 5,162 | 4,388 | 4,097 | 1.71 MB |
| Total | 254 | 19,769 | 13,704 | 12,493 | 9,938 | 6.37 MB |

With `manifest.json` at 225 KB the directory is 6.60 MB; gzip -9 takes the whole of it to 1.29 MB.

Against the research note's independently measured expectations: Common Core's 1,536 addressable codes match its ~1,540, NGSS's 208 performance expectations match exactly, and Virginia's 4,097 match its ~4,200 while additionally covering Science (2018-), which the note left unaggregated.
Texas is the one framework that does not match: 4,097 against an expected ~5,000, an 18 per cent shortfall.
That expectation was measured from TEA's own CASE packages rather than from the mirror, so the shortfall is a property of D19's source choice and not of this ingest.

## Verification against the owners' own documents

Five statements per framework, checked against the owner's published document rather than against the mirror.
All twenty are present verbatim; none is paraphrased, truncated or reordered.

| Framework | Codes checked | Owner document | Result |
|---|---|---|---|
| CCSS Mathematics | 8.F.B.5, 4.NF.C.7, 5.NF.B.3, K.CC.B.4, HSA-REI.B.3 | `corestandards.org/wp-content/uploads/2023/09/ADA-Compliant-Math-Standards.pdf` | 5/5 exact |
| CCSS ELA/Literacy | RL.2.1, W.4.9, RI.5.3, SL.1.2, RST.11-12.3 | `corestandards.org/wp-content/uploads/2023/09/ADA-Compliant-ELA-Standards.pdf` | 5/5 verbatim; 4 needed manual column reassembly |
| NGSS | MS-LS1-1, 3-PS2-1, K-PS2-1, HS-ESS1-1, 3-5-ETS1-3 | `nextgenscience.org/sites/default/files/AllDCI.pdf` | 5/5 exact |
| TEKS Mathematics | K.2.A, 1.2.B, 2.4.B, 3.4.K, 5.4.F | TEA's own rule text, `tea.texas.gov/laws-and-rules/sboe-rules-tac/sboe-tac-currently-effect/ch111a.pdf` | 5/5 exact |
| VA SOL Mathematics | 3.CE.1.a, 3.NS.1.a, 3.MG.1.a, 3.PS.1.a, 3.PFA.1.a | *Mathematics Standards of Learning for Virginia Public Schools*, August 2023, via the Wayback Machine capture of `doe.virginia.gov/home/showpublisheddocument/48908/638325354847170000` | 5/5 exact |

Two of those rows need their method stated rather than just their result.
The Common Core ELA standards are laid out as three-column grade tables, so text extraction interleaves three grades' sentences line by line; the four statements were reassembled by hand from the extracted columns and confirmed word for word, and the one that matched automatically did so because it occupies a single column.
Virginia is read through a Wayback capture because `doe.virginia.gov` returns HTTP 403 to every non-browser client, which the research note also observed.

The Texas check reads TEA's published rule PDF from `tea.texas.gov`, which is a one-off manual read of state law for verification.
It is not the `teks.texasgateway.org` documentation site whose terms of service D19 declines, and nothing is ingested from it.

A machine check runs beside the hand check: `crates/tam-standards/tests/committed_ingest.rs` compiles the committed files in, verifies each against the manifest's SHA-256 and row count, refuses a row naming a set the manifest lacks, and asserts the addressable counts stay inside the ranges above.

## The shape on disk

`docs/design/data/standards/` holds one JSON Lines file per framework and one `manifest.json`.

A node file is exactly what it says: one node per line, sorted, newline-terminated, nothing else.
Every per-set fact — subject, grade band, licence, the owner document, the API URL the rows came from — lives once in the manifest, and a node names its set by index.
That split is why the files are the size they are: repeating the provenance on nineteen thousand rows would have cost more than the statements do.

The manifest also carries each file's byte count and SHA-256, so `tam_standards::load_framework` can refuse a file whose bytes and provenance disagree rather than loading rows it cannot describe.
Determinism is a property of the writer rather than a convention: rows are sorted by a total key, optional fields are omitted rather than written null, and no map is serialised from an unordered container.
Two runs over the same upstream bytes and the same timestamp produce byte-identical files, which is what makes the hash a check rather than a record.

The fetch timestamp is an argument to the binary rather than a clock read, matching the discipline the workspace lint enforces elsewhere: time enters as data.

## Where the mirror is thinner than its owner

The research note expected roughly 5,000 Texas Student Expectations, measured from TEA's own CASE packages for chapters 110, 111, 112 and 113.
The mirror yields fewer, and that gap is the price of D19.
It is a coverage question rather than a fidelity one: the statements that are present matched TEA's published rule text exactly in the hand check above.

Two further facts about the Texas mirror matter more than the shortfall.
It carries no `statementLabel` at all, on any row, so "this is a Student Expectation" cannot be read off the data the way it can for Common Core and NGSS.
And it flattens the hierarchy: a Knowledge-and-Skills statement and its own Student Expectations are siblings under the strand rather than parent and children, so `5.10` and `5.10.A` share a parent.
Virginia has the same flattening, with `VS.13` a sibling of `VS.13.a`.
The addressability rule in `tam_standards::addressable` is per framework because of this, and each half of the Texas and Virginia rule is needed: the parent link alone admits `VS.13`, and the code-extension test alone admits the single-letter Virginia strand rows whose children are coded `1.C.1` rather than `R.1`.

The mirror also drops the Administrative Code chapter number from Texas codes, writing `5.10.A` where TEA writes `111.7.b.10.A`.
That is convenient for a picker, because `5.10.A` is what a teacher types, and it is the cause of the identity problem in the next section.

## A published code is not an identity

This is the finding with the widest consequences, and every candidate key was falsified by the committed data rather than by argument.

A code alone does not identify a standard: 697 of 4,872 Texas codes and 126 of 5,162 Virginia codes name a different standard under a different subject.
`1.1.A` is one thing in Texas mathematics and another in Texas science.
Adding the subject is not enough either: `(framework, subject, code)` still collides on 330 keys in Common Core, 414 in Texas and 322 in Virginia, because the mirror serves overlapping grade and course sets and repeats the anchor standards in each, and 331 of the Texas collisions carry different statements.
Adding the set is not enough either, colliding on 27 keys in Texas and two in Virginia.

So the identity is the mirror's own node identifier, and everything that binds a standard binds that.
Three things follow.
A picker must never show a bare code for Texas or Virginia without its subject and grade, because a bare code there is ambiguous.
The eventual TPT node-id table is keyed on the source identifier, not on the code.
And a lookup by code returns candidates rather than a standard, which is why `TptNodeIdTable::candidates_for_code` returns a list.

Common Core is the exception and stays clean: its `CCSS.Math.Content.` and `CCSS.ELA-Literacy.` prefixes are exactly the disambiguator Texas and Virginia lack.

## Licence obligations, and where each notice renders

The obligations are constants in `crates/tam-standards/src/notices.rs` with a test that they are present and unmodified, because they are compliance requirements rather than presentation choices.
`required_notices(framework)` returns each notice with the placement that discharges it.

The Common Core copyright notice — "© Copyright 2010. National Governors Association Center for Best Practices and Council of Chief State School Officers. All rights reserved." — must appear on any page that publishes or publicly displays a Common Core code or statement.
That is the picker, the product form, and any listing preview that renders one.
The ownership acknowledgement travels with it.
Placement is `WhereverDisplayed` and not a site footer, because the licence attaches the obligation to the act of display.

The NGSS disclaimer — the prescribed WestEd wording, with an asterisk in place of the forbidden ® and ™ — goes at the bottom of the site's home page and of every internal page that prominently uses the mark, per D18.
Putting it only in terms and conditions does not satisfy the guidance, which says so explicitly.
No NGSS logo is used anywhere, the mark stays visually subordinate to ours, and it appears in no product title, company name, domain name, metatag or purchased keyword.
The National Academies Press citation renders wherever NGSS content does.
D18 declines the WestEd sample submission, so nothing waits on a four-to-six-week turnaround.

The mirror's own CC BY attribution renders with the data.
It is built by `mirror_attribution` from the licence each set declared rather than hardcoded, because the rights holder varies across sets: D2L Corporation on the Achievement Standards Network lineage, Common Curriculum, Inc. elsewhere, and 29 Texas sets declare a null licence title with a rights holder.

Texas and Virginia impose no owner notice of their own.
The TEKS are Chapter 110-128 of Title 19 of the Texas Administrative Code, and the mirror declares no licence for them; Virginia states none on any page reachable to us.
Their obligation is the mirror's attribution alone.

One obligation is not a string and cannot be one.
The Common Core grant names four verbs — copy, publish, distribute, display — and modification is not among them, so no statement may be paraphrased, shortened for a dropdown, or rewritten into generated listing copy (D25).
It is enforced by never providing a function that does it, and by the picker rule that a code is shown with its full statement or not at all.

## Update strategy

Poll each source on a cron, diff the new snapshot against the stored one by source identifier, and emit differences as reconciliation items rather than applying them.
The cadences differ by an order of magnitude and follow the owners rather than convenience.
Common Core and NGSS were published in 2010 and 2013 and have never been revised, so an annual poll there is a liveness check on the mirror rather than an update mechanism.
Texas moves on the State Board of Education's published multi-year schedule, with social studies revising through 2026, so quarterly.
Virginia reviews every subject at least once every seven years on a rolling basis, which in practice changes one or two subjects a year, so quarterly as well.

The event that matters is a code disappearing, because some seller's listing points at it.
Retire rather than delete: keep the row, set its end date, and let the projection surface it.
Migration 0041 carries `retired_at` for exactly this, and every search index excludes retired rows.

A new vintage appearing under a subject we ingest is the other event worth naming, since the selection lists are constants.
It arrives as a set the ingest does not select, which is silent, so the poll should compare the jurisdiction's subject labels against the constants and raise an item when a new open-ended vintage appears.

## The TPT node-id table

TPT binds a standard by an opaque internal numeric id, posted as `data[ItemsCommonCoreStandard][common_core_standard_id][]` and read back aliased as `sphinxId` (`docs/research/rethink/tpt-product-model.md`).
A published code cannot post a selection, so this table is what would make the feature work on the wire, and it is empty.

Filling it means crawling `EducationStandardsQuery` one subtree at a time under a live TPT session, which is founder-gated and out of scope here.
The structure exists now so the ingest is shaped for it rather than retrofitted to it: `docs/design/data/standards/tpt-node-ids.jsonl` is committed empty, and `tam_standards::load_tpt_node_ids` loads it.

Two fields on a binding exist for the risk rather than for the join.
An identifier aliased as `sphinxId` is a search-index identifier and search indexes get rebuilt; if the ids move, a posted edge tags the wrong standard silently.
So a binding records the SHA-256 of the statement TPT returned and when the crawl last saw the triple agree, which is what lets a caller refuse to post an id outside the current crawl window and degrade to "standards not projected" rather than to a wrong tag.
Nothing repairs a mismatch: a code whose statement no longer hashes the same is a reconciliation item.

For Tes there is nothing to bind.
Tes has no per-standard field, so the correct projection is a `Broadened` edge for Common Core to `framework: Common Core` and `NoCounterpart` for the other three, which is machinery that already exists.

## Storage

Migration 0041 creates `standards_node`, global reference data like `canonical_term` and `projection_edge`: the catalogue is a published corpus rather than a tenant's, so there is no `org_id` and no row-level security.
What a seller selects from it is tenant data and belongs beside the mapping, in a later migration.

The shape is search-first, following what a teacher does in the order they do it.
`code_fold` holds the published code reduced to its letters and digits, upper-cased, indexed with `text_pattern_ops` so a prefix query uses the index; a teacher types `5.3B` where the mirror wrote `5.3.B`, and the fold is what makes those the same query.
The fold is stored rather than computed in the index expression so that `tam_standards::fold_code` stays its only definition and the in-memory picker and the table agree by construction.
`grade_low` and `grade_high` carry the derived interval on one scale across all four frameworks, with prekindergarten at -1 and kindergarten at 0 so the ordinary grades keep their own numbers.
A GIN index over `to_tsvector('english', statement)` serves the teacher who knows the concept but not the code; it needs no extension.
`parent_guid` is indexed because expanding a node is how a picker drills, and the parent link is the mirror's rather than the code's.

The migration was applied and exercised against a throwaway Postgres 17 cluster on the full 0001-to-0041 chain, and all 19,769 rows were loaded into it.
That is how the natural-key assumption above was falsified rather than shipped: the first draft carried a unique index on `(framework, subject, code)`, and the load stopped on the first collision, `CCSS.ELA-Literacy.CCRA.L.1`, which the mirror repeats in eleven grade sets.
Counted over the whole ingest, that index would refuse 2,395 rows across 1,433 keys.
The query plans were checked too: a folded-code prefix query is an index scan, and `5.4.F` returns two Texas standards, one in mathematics and one in social studies, which is the ambiguity above showing up in the shape a picker will meet it in.

Two pieces of wiring are outstanding and were deliberately not done here, because both fall outside this change's file scope.
`crates/tam-storage/src/lib.rs` needs one line declaring a `standards` module for a repository to exist, and a repository using the `sqlx::query!` macros needs `cargo sqlx prepare` run against a live database to regenerate the committed offline metadata.
The repository itself is small: an upsert keyed on `(framework, source_guid)` that sets `retired_at` on rows the new snapshot no longer names, and reads for prefix, grade, subject, keyword and children.
The search route exists but is not connected to this data.
`crates/tam-api/src/product/standards.rs:105` answers `not_ingested` for every framework unconditionally, on the ground that no standard matched and no standard exists here yet are different answers to a seller.
Connecting that handler to the committed files is the one API change the remaining work needs, and it is a handler body rather than a new route.

`tam-standards` is pure by construction — its only dependencies are `serde`, `serde_json` and the workspace's pinned `sha2`, and it performs no I/O — and the justfile's `purity` recipe now names it alongside `tam-types`, `tam-marketplace`, `tam-domain` and `tam-taxonomy`.
So a `cargo add` of tokio, reqwest or sqlx into it fails the lane rather than resting on review.

## Decided here

Compression, 2026-09-03: the four data files are committed as plain JSON Lines, and neither crate takes a direct dependency on a compression crate.
The four files total 6.37 MB and would gzip to 1.28 MB, a fifth of the size, so the saving was real and was still declined for three reasons.
The plain files are already in main's history, so compressing now adds bytes rather than saving them.
A direct dependency edge is founder-gated even when the package is already resolved in the tree, as `flate2` 1.1.9 is through `zip` and `png`.
And the landed state is green and simpler.
A deterministic gzip codec — a pinned header and level, a manifest byte cap on the decoder, and a test that re-compressing a committed file reproduces its own manifest hash — was designed and can be revisited if a refresh cadence ever justifies it.

## Open questions

1. The Texas coverage gap against TEA's own feed. Whether to accept it, or to revisit D19 for a verification-only pull that diffs the mirror against TEA quarterly without ingesting from it.
2. Whether to ingest Texas career-and-technical education, 190 sets that TPT sellers do sell into and that the research note's scope excluded.
3. Whether the picker's canonical row for a duplicated code is chosen by rule or shown as several. The data says several exist; the product has to say which one a seller sees.
4. Whether Virginia should be diffed against the Satchel Rosetta Exchange as a second opinion, as the research note suggested. Both are third-party transcriptions of PDFs, and diffing them is the only available fidelity check.

## Filling the node-id table: who runs the crawl

Nothing posts to TPT until the table above is filled, and filling it means enumerating four jurisdiction subtrees rather than reading a published list.
Two operations on `/graph/graphql` do it: `EducationStandardsJurisdictionsQuery` returns the roots, and `EducationStandardsQuery($id: ID!, $depth: Int)` expands one node at a time, which is how TPT's own picker works — 58 calls in the capture, `depth: 1` for a jurisdiction's top level and no depth argument below it (`docs/research/rethink/tpt-product-model.md:340`).
The four subtrees are 3054 Common Core, 3055 NGSS, 3326 TEKS and 5785 VA SOL, out of 166 roots that exist (same file:360).
Each node carries `id`, `name`, `notation`, `grades`, `parentIds`, `type` and `descriptionText`, where `name` is the published code and `id` is the value the wire field takes (same file:341, :351).
TPT exposes no public enumeration of any of this; the jurisdictions query is the enumeration (same file:365).

TPT is a no-API marketplace, so under D1 the server may not issue one of these requests, and the crawl has exactly two lawful homes: a job on a seller's device, or a founder-run capture on the founder's own machine under the founder's own session.

The recommendation is the founder-run capture.
The table is TPT-global reference data rather than a tenant's: every seller who tags `8.F.B.5` posts the same node id, so a device crawl would repeat one global enumeration once per seller and return nothing seller-specific for the traffic it spends.
Bulk enumeration is also the behaviour most likely to draw the response the marketplace-terms memo names as the pivot, and a founder capture concentrates that exposure on a single account the founder controls rather than spreading it across every customer's session.
The cadence agrees with the choice: the table moves when TPT reindexes, not when a seller publishes.

Device-side work remains, but it is verification rather than enumeration, and that asymmetry is the point.
`EducationStandardsByIds($ids: [ID]!)` resolves a saved set back to names, and it is the call TPT's own edit form makes to re-render an existing product's selections (`docs/research/rethink/tpt-product-model.md:346`).
A device about to post standards asks that one question about exactly the ids it is about to post, which is indistinguishable from ordinary use of the form, costs one request, and refuses an id the marketplace no longer agrees with.
So enumeration stays off every seller's device and the freshness check stays on it.

The crawl is not performed here.
Its binary belongs beside `tam-standards-fetch` and reuses that crate's existing `reqwest` and `tokio` dependencies, so no new dependency edge is opened for it (`crates/tam-standards-fetch/Cargo.toml:8`), and it never ships in the server image.

## The kill gate: node ids that move

`sphinxId` names a search index, and search indexes get rebuilt (`docs/research/rethink/tpt-product-model.md:353`).
If the ids move, a stored binding posts a silently wrong tag, which is worse than posting nothing at all.
The gate is therefore whether storing an id and posting it later is a viable design, and it is settled by measurement rather than by argument.

The protocol is a second capture taken at least thirty days after the first and diffed against it by `source_guid`, with two checks that must not be conflated.
The identity check asks whether node id N still returns a `name` equal to the bound `code`; a changed `name` means the id now denotes a different standard, and that is the failure this gate is about.
The drift check asks whether `statement_sha256` still matches; a changed statement under an unchanged `name` means TPT reworded its prose, which is a reconciliation item and a refreshed hash rather than a failure.
Both fields already exist on `TptBinding` for exactly this (`crates/tam-standards/src/tpt.rs:62`, `:64`).

The decision rule reads the identity check across the whole re-crawl.
No moved ids means store and post, with the per-post verification above as the standing check.
Fewer than one per cent moved means the same, with the capture cadence tightened to quarterly.
One per cent or more kills stored ids, and the fallback is to resolve the id at post time on the device by expanding the subtree from the code, which costs a handful of requests for one product's standards against the picker's own 58.
That fallback is a degradation rather than a project kill, and naming it now is what makes the gate safe to fail.

A binding outside the current crawl window is not posted under any branch.
The caller omits the standards field and emits the loss record the projection already carries, so the seller reads "these standards are not carried to this marketplace" in the field diff before publish rather than discovering a wrong tag afterwards (`docs/notes/design/vendoo-for-teachers-rethink.md:217`).

## Founder decisions

Seven, each with a recommended answer.
The first four are the open questions above, restated with an answer rather than repeated.

1. The Texas coverage gap against TEA's own feed.
   Accept it, and do not pull TEA.
   D19 holds that a written question to Texas becomes necessary the moment we pull TEA's feed directly, so even a verification-only diff spends the thing D19 bought; revisit only if a Texas seller reports a code the catalogue lacks.

2. Texas career-and-technical education, the 190 sets the ingest excludes.
   Not now.
   The selection lists are constants in `crates/tam-standards-fetch/src/main.rs`, so deferring costs a re-run rather than a redesign, and CTE sellers are not the launch cohort.

3. The canonical row for a duplicated code.
   Show several rather than choose by rule.
   The data says several exist and 331 of the Texas collisions carry different statements, so a rule would silently pick one of them; `candidates_for_code` already returns a list for this reason (`crates/tam-standards/src/tpt.rs:97`).

4. Virginia against the Satchel Rosetta Exchange.
   Yes, once, as a one-off fidelity check rather than a standing job.
   Both are third-party transcriptions of the same PDFs, so a single diff either finds transcription errors or retires the question permanently.

5. Who runs the node-id crawl.
   The founder-run capture, with device-side per-post verification, for the reasons in the section above.
   The alternative ships a bulk enumerator to every customer's session and buys nothing per seller.

6. What the console does while the table is empty.
   Let a seller tag standards and store them, and let publish to TPT omit the field with a visible loss record.
   The tag is worth carrying in our own catalogue before it can post, and the loss record is the machinery that makes the omission honest rather than silent.

7. When the second capture runs.
   Before the standards feature is enabled for any seller.
   The gate exists to decide whether stored ids are postable at all, and running it after sellers depend on the answer converts a design decision into an incident.

## Build order and path ownership

Five steps.
None touches `crates/tam-engine`, `crates/tam-engine-driver` or `crates/tam-storage`, and one touches `crates/tam-api` at a handler body rather than a route.

1. Connect the search route to the ingested catalogue: `crates/tam-api/src/product/standards.rs`.
   Reading through `tam_standards::search` over the committed files keeps `crates/tam-storage` out of this stream entirely, and migration 0041 stays where it is for the day the corpus outgrows a file read.

2. The crawl's request shaping and response parsing: a new `crates/tam-marketplace-tpt/src/standards.rs` and one module line in that crate's `lib.rs`.
   It stays pure — it builds the two operations' bodies and parses their nodes — and the GraphQL error classification it needs already exists (`crates/tam-marketplace-tpt/src/classify.rs:89`).

3. The diff and the gate arithmetic: a new `crates/tam-standards/src/crawl.rs` and one module line in `crates/tam-standards/src/lib.rs:24`.
   Pure, so the identity check, the drift check and the decision rule are all testable against fixtures without a session, which is what lets the gate be exercised before the capture exists.

4. The founder capture binary: a new `crates/tam-standards-crawl/`, added to the workspace members at `Cargo.toml:10`.
   It reuses `tam-standards-fetch`'s dependency set, writes `docs/design/data/standards/tpt-node-ids.jsonl`, and must not be named by the `purity` recipe, since it is the one crate in this stream that holds a transport (`justfile:38`).

5. The console surfaces: `web/src/lib/StandardsPicker.svelte` and `web/src/lib/tpt-form.ts`.
   The picker shows subject and grade beside every Texas and Virginia code and never a bare one, renders whatever `required_notices` returns for the frameworks on screen (`crates/tam-standards/src/notices.rs`), and the publish path shows the unbound-standards loss record before publish.

Steps 2, 3 and 4 share no files with step 1 or with each other and can run in parallel; step 5 depends on step 1.
Step 4 is a new crate rather than a new dependency, which is the distinction that keeps it inside the enforcement rule rather than against it.

## What step 1 landed

The search route reads the committed corpus rather than answering `not_ingested`.

Three decisions are recorded here because none of them is derivable from the code that implements them.

The corpus is compiled into the server binary with `include_str!` and `include_bytes!`, the same five files and the same reasoning as `crates/tam-standards-crawl/src/main.rs`: a search answers from the ingest the binary was built from rather than from whatever a working copy holds.
A corpus update is therefore a rebuild rather than a file swap, which is accepted.
The parsed index lives in a `OnceLock` primed by `crates/tam-server/src/main.rs` during setup, through a call the binary refuses to start on, so a corpus this build cannot read fails the boot rather than the first seller's search and no request pays the parse.
That is a departure from how this crate reaches a handler, since the pool, the clock and the binary's other decisions all travel through `AppState`; the reason is that the corpus is compiled in and identical for every tenant and every deployment, so there is nothing per-deployment about it to carry.
What is per-deployment, the crawl window, travels through `AppState` like everything else.

`tpt_node_id` is absent on every result today, by construction rather than by omission.
`docs/design/data/standards/tpt-node-ids.jsonl` is committed empty, so `postable` answers `Unbound` for every code, and it stays that way until the founder runs the step 4 crawl and commits the table.
The window itself is a `Config` field, and absent means withhold every id: without a window there is no evidence any id still resolves, and posting one TPT has since rebuilt puts a listing under a standard nobody chose.

The handler serves both halves of the licence obligation, and the reason is worth stating because following this note's own handover literally would have dropped one.
`required_notices` answers the notices a framework's *owner* imposes, and Texas and Virginia impose none — `notices.rs` returns an empty slice for both and says the list is empty by fact rather than by omission — so a handler reading only that function would have displayed mirrored Texas and Virginia standards under no attribution at all, where the constant it replaced did carry one.
Their whole obligation is the mirror's CC BY attribution, which `mirror_attribution` builds per set because the rights holder differs across sets, so the handler emits the owner's notices where they exist and the mirror's beside them, deduplicated; the Texas and Virginia case is pinned by a test that asserts both facts together.

Step 1 pulled `subject` forward from step 5 and left `grade_band` behind, which is worth recording because the asymmetry is what let a defect through.
The note's step 5 asks the picker to show subject and grade beside every Texas and Virginia code and never a bare one; step 1 served the subject half because it cost one field, and deferred the grade half as a wire change.
Serving half of a requirement stated as a pair is what stopped anyone reading step 1 against step 5's text, and the missing piece found later was not the grade at all but the identifier: 814 TEKS codes name more than one addressable node, so a client keying its list on the code can attach one standard's row to another's, and `source_guid` is now served for that reason.

The keyword scan is bounded and supervised. A query is refused above 120 characters or 12 words before anything scans, because the index reads the whole corpus once per term and an unbounded query was about 370 ms of CPU at 8 KB and 1.5 s at 32 KB, on a request any signed-in member could repeat; the bounded worst case measures 12.4 ms.
It then runs through `spawn_supervised_blocking` in `crates/tam-api/src/blocking.rs`, which is the wrapper `clippy.toml` names in its ban on `tokio::task::spawn_blocking` and which nothing in this repository had built, so the first caller to need it found a ban pointing at a function that did not exist.
The wrapper awaits the handle and turns a `JoinError` into an error the caller must handle, which is the ban's own reason satisfied at one sanctioned site rather than at every call.

The loss derivation for standards a marketplace will not carry landed in `web/src/lib/tpt-form.ts` with this change rather than with step 5, because the identifier that fixed the picker had to reach `StandardPick` in the same file.
That derivation reads the absent `tpt_node_id` rather than the engine's own record, because `StandardsProjection` and `NotCarried` in `crates/tam-marketplace-tpt/src/standards.rs` are computed by nothing: no engine, importer or API path refers to them, so the record exists and is inert.
The server withholds the node id for both reasons the record distinguishes, `Unbound` and `OutsideCrawlWindow`, so the console cannot tell them apart and does not try; a seller's situation is the same in both, and the sentence states the outcome rather than guessing the cause.
When something computes the record, the derivation is replaced without what a seller reads changing.

`StandardView` now carries `grade_band` beside `subject`, which completes the picker requirement this note states: subject and grade beside every Texas and Virginia code, and never a bare one.
It is rendered server-side from the derived interval where one parses and from the source's own level codes otherwise, because a client given the levels and the interval would reimplement the ordering that turns them into a phrase and two clients would render one set two ways.

The corpus parse costs 0.43 seconds for 19,769 rows across 6.3 MB, measured over three runs: the manifest, four framework files each verified against its recorded content hash, and the node-id table.
Paid once at boot and by no request, which is the whole argument for priming at startup rather than behind the first search.

`crates/tam-api` gained a path dependency on `crates/tam-standards`, which the team lead ruled inside the enforcement rule rather than against it, and the founder may reverse.
The rule's "adding a dependency" is the `deny.toml` class — third-party crates carrying licences and advisories — beside limits and lints; a workspace-internal edge onto a pure crate touches none of those gates, and `just purity` is unaffected because it checks that the pure crates do not acquire tokio, reqwest or sqlx, which this edge does not do.
The line in step 4's record about a new crate rather than a new dependency was about that same containment, and does not speak against this edge.

## What steps 2, 3 and 4 landed

Steps 2, 3 and 4 of the build order are built and green; steps 1 and 5 are not, and what they need is at the end of this section.

Step 2 is `crates/tam-marketplace-tpt/src/standards.rs`.
It shapes the two crawl operations, parses their nodes, and maps a resolved node id onto the create form's wire fields.
The count field and the id parts come back as separate groups because the form posts them apart, the count early among the thumbnail fields and the parts late after the categories, so a caller splices each at its own recorded position.
A caller holding no postable id gets a record naming what will not be carried and why, which is decision 6 on the wire.
Two limits are worth stating rather than discovering.
Neither operation appears in a HAR capture, so the query texts are reconstructed from the selection sets recorded at `docs/research/rethink/tpt-product-model.md:340` and the two `children` argument names are inferred from the recorded variable signature; the founder's first capture is what confirms them.
And the module holds no dependency edge to `tam-standards`, so a node id reaches it as a local newtype: an adapter that could read the standards catalogue could resolve a code by itself, and that resolution belongs to the crawl.

Step 3 is `crates/tam-standards/src/crawl.rs` for the capture and the gate, and `crates/tam-standards/src/bind.rs` beside it for the join, because walking TPT's tree and binding to the mirror are different responsibilities and only the crawl binary needs both.
The join exists in this dispatch at all because writing `tpt-node-ids.jsonl` means writing rows keyed on the mirror's `source_guid`, which is ours rather than TPT's.

The join binds a crawled node to every catalogue row whose code corresponds and whose statement agrees, and reports every other node as residue with its reason attached.
Two invariants shape the result and they are not symmetric.
One node binding many rows is allowed and expected: the mirror repeats a Common Core anchor standard across eleven grade sets, and each of those rows is a row a seller tags.
One row bound by two nodes is refused, because a row bound twice has no answer to which id to post, so a contested row is dropped and both claimants are named in the residue.
Scope is what keeps the second invariant reachable, since verbatim adoptions put one statement under more than one jurisdiction and TPT carries a tree for every state beside the Common Core one.
Every candidate is therefore scoped twice over: the catalogue index a walk consults holds one framework's rows and no other, and a node whose own ancestor chain does not carry the walked root is refused before its code is read.
The correspondence from a TPT subtree to a mirror jurisdiction is an input the caller supplies rather than something the join derives; the coarse one is the four roots the form offers, and a finer one, from a TPT subject or domain node to a mirror set, does not exist until a capture does.
Code correspondence admits two spellings and no more, the existing `fold_code` equivalence and a namespace prefix the mirror carries and TPT does not, and the second is guarded so that `5.10.A` can never be named by `10.A`: the removed prefix must carry no digit, because TPT drops letters and never drops a numbered level.
Statement agreement absorbs four differences and no more, whitespace, letter case, the Unicode quotation marks and dashes a publishing pipeline substitutes, and a single trailing full stop; anything wider is a real difference in what the two sides say a standard says, and it stays in the residue where a human reads it.

Two things in step 3 depart from this note as written above, both because the committed data falsified a premise.

`TptBinding` gained `tpt_name`, the name TPT itself returned for the node at crawl time.
The kill-gate section says the identity check asks whether node id N still returns a `name` equal to the bound `code`, and those are not the same string: the mirror codes the anchor standard `CCSS.ELA-Literacy.CCRA.L.1`, TPT names the node `CCRA.L.1`, and the mirror's own `alt_code` is a third form, `CCR.L.1`.
Re-deriving TPT's name from our code would have reported every Common Core binding as moved and killed the feature on an artefact of our own spelling, so the crawl records what TPT said and the identity check reads that.
It costs nothing: the data file is committed empty, so no row migrates.

And `gate` has a fourth verdict, `Inconclusive`, for a second capture that answered for none of the bindings in force.
Folding that into "no moved ids means store and post" would let a walk that stopped short read as evidence that nothing moved, which is the one reading that makes the gate unsafe to trust.
The three named verdicts and their thresholds are unchanged, and the arithmetic is integer cross-multiplication rather than a division.

Step 4 is `crates/tam-standards-crawl/`, which the founder runs by hand and nothing else runs.
It refuses to start without `--i-am-the-founder`, takes the crawl timestamp as an argument rather than reading a clock, reads the founder's own exported cookie jar the way this repo's supervised live examples do, and walks each jurisdiction the way the picker does — one level at the root, then each of its children whole — pausing between calls.
It compiles the committed corpus in rather than reading it, so the join runs against the ingest the binary was built from.
It writes the bindings sorted and newline-terminated, and prints a residue report naming every crawled node that bound nothing, because that report is what the founder reads before committing the file.
Nothing the residue names is a defect in the file: an unbound node stays unbound, and a seller's tag for it is carried in our catalogue and omitted from a TPT publish with a visible loss record.

None of the three is verified against a real TPT standards-tree response, because no capture of one exists in this repository.
The founder's first capture is therefore also the first test of the request shaping and of the join, and statement transcription differences between the mirror and TPT will put some codes in the residue that a human would call the same standard.
The ancestry scope has the same character: it reads `parentIds` as the full ancestor chain the research records, and a capture whose chain omitted the walked root would put every node in the residue under that one reason, which is a visible refusal to read in the report rather than a silent mis-scope.

The kill gate has no runner, and that is the next step after step 4.
`diff_capture` and `gate` decide the second capture's verdict and are tested against fixtures, but nothing takes the second capture and feeds it to them: the crawl binary writes the table and does not re-crawl against one already written.
Decision 7 says the second capture runs before the standards feature is enabled for any seller, so the runner is needed before the feature ships and not before the table is filled.
It is small — the same walk this binary already performs, against the committed bindings rather than against the catalogue — and it is named here so that the gap is a planned next step rather than something discovered when the gate is first needed.

Step 1 needs `tam_standards::search` over the committed files for the handler body, and, for `StandardView.tpt_node_id`, `load_tpt_node_ids` with `crawl::postable` against a `CrawlWindow`, so an id no current capture vouches for is withheld rather than served.
It should also read its four attribution strings from `notices::required_notices` rather than from the constant it now holds, which is a second copy of the same obligations.
Step 5 needs the search response from step 1, and the loss record from step 2 rendered in the field diff before publish.
