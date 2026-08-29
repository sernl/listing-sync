# Phase 4 amendment corrections, found implementing commits 8 to 14

- date: 2026-08-29
- scope: the seven commits from the grade crosswalk to the mapping-loss record
- evidence: docs/design/data/tpt-vocabulary.json, docs/design/data/tes-vocabulary.json, crates/tam-storage/migrations/0021_projection_edge_single_valued.sql through 0024_mapping_loss.sql

The Phase 4 amendments document stays as written; this note is the correction
record for what implementation found wrong in it, so the Phase 4 design commit
folds these in rather than rediscovering them.

Two of the four are structural contradictions between two amendment sections
that both land inside the same run of commits, so neither could be deferred.

## The projection-edge key forbids the relation the grade axis needs

§A.3 makes the seeder's refresh an upsert and narrows `projection_edge`'s
primary key to `(from_term, to_inventory, to_term_kind, kind)`, stating the
purpose plainly: one term holds at most one edge of each kind into each
vocabulary, which is what makes `Ambiguous` unreachable from the seeder.

§B seeds `Narrower` edges from each Tes GB age band to each year group it
covers. Band 3, ages 7 to 11, covers eight year groups, so one term needs
eight `Narrower` edges into `(TesUs, Phase)`. Under the narrowed key that is a
constraint violation, and the grade-ingest commit cannot land on top of the
crosswalk commit as both are written.

The resolution keeps the existing primary key and adds a partial unique index:

```sql
CREATE UNIQUE INDEX projection_edge_single_valued
    ON projection_edge (from_term, to_inventory, to_term_kind, kind)
    WHERE kind <> 'narrower';
```

`project` ignores narrower edges entirely (`crates/tam-taxonomy/src/project.rs`,
the `EdgeKind::Narrower => {}` arm), so a narrower edge can never produce
`Ambiguous`. The narrower-excluding index therefore delivers §A.3's stated
purpose exactly, while the blanket narrowing over-delivers into a constraint
§B needs violated. It is also strictly less destructive than dropping and
rebuilding the primary key, and it keeps the capability removal P.4 worried
about scoped to the two kinds that project.

The seeder upsert infers that index as its `ON CONFLICT` arbiter for exact and
broader edges, and upserts narrower edges on their own path. P.4's duplicate
pre-check is carried in the migration and passed live against the seeded
relation, both with and without the narrower exclusion.

## The seventh band term collides with the sentinel

§B mints seven `ageRanges` band terms with identity `Exact` edges into
`(TesGb, Phase)`. §2.3 of the mapping design separately seeds an `Exact`
sentinel edge from the `yearGroups` 30 term onto `ageRanges` 7, both being
"not applicable". Both claim the same target path as `Exact` from two
different terms, which `projection_edge_exact_reverse` refuses.

Six band terms land, and band 7 *is* the `yearGroups` 30 term: the two
sentinels denote one thing, and the sentinel edge already seeded is that
band's identity edge. `ingest_by_native_id("7", (TesGb, Phase), ..)` resolves
through it unchanged.

## The TPT grade label join is not unique

§A.4 records that TPT's grade labels live behind a `legacyId` to
`taxonomyTags` join and that the label for id 1 is Preschool. Both hold. What
it does not record is that `legacyId` is unique only *within* a facet
category: id 1 also names the `Price-Range` facet `free`, and id 19 names
three facets across three categories.

The derivation scopes the join to `Grade-Level` and `audience` — the two
categories TPT's own `gradeLevels` note says the grade selector draws from —
and refuses a legacy id that resolves to none or to more than one, rather than
taking whichever facet the capture happened to list first.

## The carriage needs storage the amendments did not price

§J.2 puts `body_format` on `ProjectedListing`, on `ListingProjection` and on
`ListingCopy`, and gives that commit `just check` alone as its lane.
`ListingCopy` is part of `CanonicalProduct`, so the declaration has to be
durable or it is a guess recomputed at write time — which is the byte-sniffing
the TPT write model already ruled out. The commit therefore carries a
migration and gains `just db-test`.

The backfill is `markdown` because every product on file was imported from
Tes, whose draft body posts `descriptionRawType: "md"`; the first HTML body
arrives with the TPT import and declares itself.

## Two smaller departures, recorded so they are not read as drift

`AxisKey` is not introduced. Its stated ground was that `tam-domain` depends on
`tam-marketplace`, so a shared axis label must sit in the crate both depend on;
`TermKind` itself now lives in `tam-types` for exactly that reason, so the
mirror and its total conversion function would be two names for one enum in
one crate.

The grade derivation carries its own output type rather than widening
`tes::Crosswalk` with a `no_counterparts` field. The two derivations refuse
differently: a GB subject with no NZ counterpart is a capture gap and stays
residue for the reconciliation queue, while a year group no age band covers is
a measured absence in a vocabulary held whole. Widening the shared type would
invite seeding the first as the second.

## Migration numbering

Phase 4's migrations do not start where the amendments' §A.6 table says,
because the crosswalk commit needs one the table does not allocate. The
sequence as landed and as the remaining commits should continue it:

| migration | contents | commit |
|---|---|---|
| `0020` | product rights and residue, the two CHECK widenings | 5 |
| `0021` | `projection_edge_single_valued` | 8 |
| `0022` | `product.body_format` | 12 |
| `0023` | `election_rule`, `election_item` | 13 |
| `0024` | `mapping_loss` | 14 |
| `0025` | `sync_request`, `sync_request_resource` | 16 |
| `0026` | the counterpart gate and `sever_generation` | 17 |
| `0027` | the publish operation | 18 |

`rls_matrix.rs`'s `TENANT_TABLES` stands at 26 after commit 14 and reaches 28
at commit 16.

## One behaviour change worth stating in the design record

A grade path the relation does not recognise is now carried out as
`unrecognised` and does not reach the wire, where before it was relabelled into
the target vocabulary and posted. That is the grade-relabelling fix working,
and it means a product whose grades were never seeded publishes with no grades
until the seeder has run. `unrecognised` does not block, by design, so nothing
parks; the fact travels in `ProjectionBlocked::Blocked` and belongs in the job
report. It is the one place the fix trades a wrong value for an absent one.
