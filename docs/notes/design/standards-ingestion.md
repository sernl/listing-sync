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
No API is wired.

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
