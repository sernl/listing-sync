# Re-basing the canonical vocabulary on TPT: the residue

- date: 2026-09-03
- change: `crates/tam-taxonomy/src/grades.rs`, the mint inverted from Tes `yearGroups` onto TPT's grade options; `crates/tam-domain/src/registry/tpt.rs`, the grade cap declared
- kill gate: does the inverted direction lose any mapping the current direction holds?
- verdict: the gate holds. Zero mappings lost in either direction on the grade axis. The residue is entirely in term identity and label, and it is a re-seed cost rather than a coverage loss.

Input to Phase 3 of `docs/notes/design/vendoo-for-teachers-rethink.md`.
Evidence is `docs/design/data/tpt-vocabulary.json` and `tes-vocabulary.json`, read by the derivation itself rather than transcribed here.

## What the re-base actually changes

`project`, `project_terms`, `project_axis` and `ingest` decide on the edge set alone.
Which side a term was minted from is a fact about the rows in `canonical_term` and `projection_edge`, not about the functions, so inverting the mint is a re-seed and the functions are untouched.

Before, the canonical grade vocabulary was minted from Tes `yearGroups`: thirty rows, each carrying its own ages, with TPT's nineteen options pairing in through one authored fifteen-row table.
After, it is minted from TPT's nineteen grade options, and the fifteen Tes rows TPT has no option for are minted as extension terms carrying Tes's identity.
That split is the whole content of the base decision, and it is what makes the canonical model a TPT *superset* rather than TPT alone.

The authored artefacts survive the inversion unchanged: the fifteen-row pairing table, and the narrowest-covering rule that routes a phase onto a Tes GB age band.
Both were already expressed as derivations over the captures rather than as transcriptions, which is why they re-derive in the other direction against the same files.

## Residue counts per axis

Grade is the only axis re-based in this wave; subject, topic and licence are untouched, and resource type, tag, format, standards, custom category and status are still unrouted.

### Phase, the grade axis

The relation is isomorphic across the inversion.
Same terms, same edges, same measured absences, same uncovered rows — only fifteen identifiers and fourteen labels move.

| Figure | Before (Tes-minted) | After (TPT-minted) | Residue |
|---|---|---|---|
| Canonical terms | 40 | 40 | 0 |
| — minted from TPT | 4 | 19 | +15 |
| — minted from Tes `yearGroups` | 30 | 15 | -15 |
| — minted from Tes `ageRanges` bands | 6 | 6 | 0 |
| Projection edges | 180 | 180 | 0 |
| `(Tpt, Phase)` exact / narrower | 19 / 13 | 19 / 13 | 0 |
| `(TesGb, Phase)` exact / broader | 7 / 27 | 7 / 27 | 0 |
| `(TesUs, Phase)` exact / narrower | 30 / 27 | 30 / 27 | 0 |
| `(TesNz, Phase)` exact / narrower | 30 / 27 | 30 / 27 | 0 |
| No-counterpart records | 30 | 30 | 0 |
| — against `(Tpt, Phase)` | 16 | 16 | 0 |
| — against `(TesGb, Phase)` | 6 | 6 | 0 |
| — against `(TesUs, Phase)` / `(TesNz, Phase)` | 4 / 4 | 4 / 4 | 0 |
| Year groups no band covers | 2 | 2 | 0 |
| Terms whose canonical id changes | — | — | 15 |
| Terms whose label changes | — | — | 14 |
| Terms that become unreachable on ingest | — | — | 0 |

Zero on every coverage row is the kill-gate answer.
Every source path that ingested before ingests now, every projection that resolved before resolves now to the same target path, every loss that was disclosed is still disclosed, and every measured absence is still measured.
Nothing in the outbound or inbound direction changes except which canonical identifier the answer travels under.

### The fifteen identity moves

Each is one canonical term whose id changes from `tes-year-group:{n}` to `tpt-grade:{m}`, both derived deterministically, so the re-seed is an insert of fifteen new rows rather than an update of fifteen existing ones.

| TPT grade | slug | new label (TPT) | old label (Tes) | Tes year group |
|---|---|---|---|---|
| 1 | `preschool` | Preschool | Pre-K | 16 |
| 2 | `kindergarten` | Kindergarten | Kindergarten | 17 |
| 3 | `1st-grade` | 1st Grade | 1st | 18 |
| 4 | `2nd-grade` | 2nd Grade | 2nd | 19 |
| 5 | `3rd-grade` | 3rd Grade | 3rd | 20 |
| 6 | `4th-grade` | 4th Grade | 4th | 21 |
| 7 | `5th-grade` | 5th Grade | 5th | 22 |
| 8 | `6th-grade` | 6th Grade | 6th | 23 |
| 9 | `7th-grade` | 7th Grade | 7th | 24 |
| 10 | `8th-grade` | 8th Grade | 8th | 25 |
| 11 | `9th-grade` | 9th Grade | 9th | 26 |
| 12 | `10th-grade` | 10th Grade | 10th | 27 |
| 13 | `11th-grade` | 11th Grade | 11th | 28 |
| 14 | `12th-grade` | 12th Grade | 12th | 29 |
| 23 | `not-grade-specific` | Not Grade Specific | Age not applicable | 30 |

Kindergarten is the one label the two vocabularies already agreed on.
Two of the fourteen changes are worth more than a rename.
The old canonical set carried a term labelled `1` for GB Year 1 and a term labelled `1st` for US grade 1, which are one character apart and denote different phases; the TPT base spells the second `1st Grade`, so the pair is no longer confusable on a screen.
And `Age not applicable` becomes `Not Grade Specific`, which is what TPT's own form calls the all-grades escape.

### The fifteen extension terms

TPT is not a superset here, and this is where that is admitted.
Fifteen `yearGroups` rows name a phase TPT's nineteen options have no member for — GB Nursery, Reception, and Years 1 through 13 — and all thirty rows are selectable on the Tes US and NZ wires.
They keep their Tes identity and their Tes label, and each takes a no-counterpart record against `(Tpt, Phase)`, which is exactly what it had before.

### Where the base cannot supply the axis

TPT's `gradeLevels` carries form labels and nothing else: no ages, no bands, no ordering beyond the id.
So a TPT-minted term reaches a Tes GB age band only through the year group the pairing table names for it, and the Tes capture stays a required input to the derivation after the re-base.
The four TPT options no Tes row pairs — Higher Education, Adult Education, Homeschool and Staff — reach no band at all and keep their three no-counterpart records each.

This is the honest statement of the base's limit on this axis: TPT decides the vocabulary, and Tes still decides the ages.

### One behaviour that would have changed and was held fixed

Filling the band-covering map in mint order puts the TPT-paired US rows before the GB extension rows, which permutes the candidate list a `Narrow` election offers a seller from `5, 6, 7, 8, 19, 20, 21, 22` to `19, 20, 21, 22, 5, 6, 7, 8`.
That list is seller-facing, and reordering it was not part of the re-base, so the covering set is now sorted ascending by year group in `seed_bands` and the order is decided there rather than inherited from which pass minted the term.

### Subject, topic, licence

Untouched.
The Tes GB-to-NZ subject and topic crosswalk mints from Tes on both sides and has no TPT counterpart to invert against; a TPT-to-Tes subject crosswalk is the expensive axis and is not in this wave.
Licence mints from Tes because TPT declares the axis absent, which is the legal exemplar and the one place the superset claim is false by measurement rather than by capture gap.

| Axis | Terms | Base after this wave | Residue from the re-base |
|---|---|---|---|
| Phase (grade) | 40 | TPT, plus 15 Tes extensions and 6 Tes GB bands | 15 identities, 14 labels, 0 mappings |
| Subject | unchanged | Tes GB, `mapTo`-paired to NZ | 0, not re-based |
| Topic | unchanged | Tes GB, `mapTo`-paired to NZ | 0, not re-based |
| Licence | unchanged | Tes; TPT declares the axis absent | 0, not re-based |
| Resource type, tag, format, standards, custom category, status | none | unseeded | not applicable |

## Tests that moved, and why none was weakened

Four assertions in the grade tests and one in `native_labels.rs` named a value the re-base moves.
Each was updated to the new value at the same strength; none was relaxed, deleted or made conditional.

- `no_band_is_invented_where_none_covers` — US Pre-K's term is now `tpt-grade:1`. The assertion that no band is invented and that the absence is recorded is unchanged.
- `the_not_applicable_sentinel_takes_one_exact_band_and_not_every_bounded_one` — the sentinel is now `tpt-grade:23`.
- `a_gb_age_band_enters_the_relation_as_a_term_of_its_own` — the shared not-applicable term is now `tpt-grade:23`.
- `a_us_year_group_broadens_onto_a_gb_band_and_names_what_it_dropped` — US 4th grade ingests to `tpt-grade:6`. The broadening, the disclosed loss and the resolved GB band id are unchanged.
- `the_tpt_scales_carry_their_captured_labels` — teaching duration 6 is `1 Hour`, the title case the 2026-09-03 DOM read shows the seller.

Two ordering assertions failed and were fixed in the derivation rather than in the test, per the previous section.

Eight tests are new: six in `crates/tam-taxonomy/src/tpt_form.rs` covering the caps, the contradicted cap, the registry-to-capture join and the roll-up exclusion; one in `listing.rs` for the cap as `project_listing` spends it; and one in `crates/tam-domain/src/registry/tpt.rs` pinning all four axes' cardinalities together.

The grade test module moved out of `crates/tam-taxonomy/src/grades.rs` into `crates/tam-taxonomy/src/grades/tests.rs` as a child module, so the derivation reads on its own and `super::` still names `grades`.
No behaviour changed and the count is unchanged.

## The caps, as data

`docs/design/data/tpt-vocabulary.json` now carries `constraints.selectionCaps`, and `crates/tam-taxonomy/src/tpt_form.rs` reads it.

| Picker | Stated | Reported by `TptForm::cap` | Why |
|---|---|---|---|
| Grades | 4 | `Some(4)` | "Select up to four grades", corroborated by the seller blog |
| Subject areas | 3 | `None` | the 2026-08-30 create posted four and TPT accepted them, so three is a claim and not a refusal |
| Tags | 6 | `Some(6)` | raised from three in TPT's March 2025 tagging overhaul |
| Formats | 3 | `Some(3)` | stated on the picker |
| Thumbnails | 4 | `Some(4)` | four `thumb1..thumb4` boxes, and help article 360042865851 |

`TptForm::cardinality` returns these in the shape `AxisRequest.binding` reads, so a caller hands the projection the measured cap rather than restating it.
An over-cap set is a question about the whole set and never a set cut to length, which `a_grade_set_over_the_measured_cap_elects_rather_than_truncating` holds directly: five grades against the cap of four raise one `OverCap` election carrying all five, and resolve nothing.

The three `Grade-Level` roll-ups — `elementary`, `middle-school`, `high-school` — carry `sellerWritable: false`.
They are the parents of the seventeen checkboxes the create form renders, they drive buyer-facing browse filters, and the form offers no control for any of them.
This is a different fact from `isHidden`, which means retired and still readable; a roll-up is current and unwritable.
`no_seeded_edge_projects_into_a_facet_the_form_withholds` checks every one of the thirty-two edges into `(Tpt, Phase)` against that set.

## The registry now enforces the grade cap

`crates/tam-domain/src/registry/tpt.rs` declares `Phase` as `Many { cap: Some(CountCap { limit: 4 }) }`.
`Subject`, `Topic` and `ResourceType` stay `None`, for three different reasons: the subject cap is contradicted by a capture, and the other two have no picker on the form and therefore no measurement at all.
`only_the_grade_axis_declares_a_cardinality_cap` asserts all four together so the asymmetry cannot be tidied away, and `the_declared_grade_cap_is_the_one_the_capture_states` holds the const against `constraints.selectionCaps.grades` from the side that can read both.

`project_listing` now enforces it end to end.
`a_grade_set_over_tpts_declared_cap_blocks_on_one_election_and_publishes_nothing` puts five mapped grades through a TPT projection and takes the blocked result: one election, `OverCap { cap: 4 }`, carrying all five paths, resolving nothing.
Reverting the registry line to `cap: None` fails that test, so it is severe rather than merely present.

## What this wave does not do

The two Tes language facets in `tpt-vocabulary.json` carry mojibake from the 2026-08-29 poll — `En espaxf1ol` and `En franxe7ais` for `En español` and `En français`.
Repairing them was not in this wave's scope and they are display-only, so they are named here rather than changed.

## The per-organisation override slice

Design only; nothing outside this note was built for it.
This is the section the orchestrator schedules after the driver split.

### Why it cannot live in the global relation

`projection_edge` carries no organisation, its rows are global and permanent, and its uniqueness indexes refuse a corrected row.
A seller who wants "my TPT Math tag always becomes Tes Mathematics / Number, not Mathematics / Algebra" is not asserting a global equivalence and must not be able to write one.
So the override is a separate relation consulted before the global one, and the projection prefers it and records which won.

### Type sketch

```rust
/// One organisation's own answer for one canonical term on one axis,
/// preferred over the global relation and never merged into it.
pub struct ProjectionOverride {
    pub org: OrgId,
    /// The inventory and axis the override answers for. Keyed by the whole
    /// `VocabularyId` rather than by inventory alone, because a seller who
    /// corrects a grade has said nothing about their subjects.
    pub target: VocabularyId,
    pub term: CanonicalTermId,
    /// What this organisation publishes instead. Empty is legitimate and
    /// means "drop this term for me", which is the per-seller form of a
    /// no-counterpart record and must not be confused with an absent row.
    pub to: Vec<VocabularyPath>,
    pub decided_by: Decider,
    pub decided_at: Timestamp,
}

/// Which relation produced a resolved path, so the field diff can say
/// "you set this" rather than "the relation says this".
pub enum ResolvedBy {
    Relation,
    Override { decided_at: Timestamp },
}
```

`ListingContext` gains one slice beside `edges`, `no_counterparts`, `rules` and `settled`:

```rust
pub overrides: &'a [ProjectionOverride],
```

`AxisOutcome.resolved` becomes `Vec<(VocabularyPath, ResolvedBy)>`, or gains a parallel `provenance: Vec<ResolvedBy>` if changing the field's type costs too much at the call sites.
The first is better and the choice is the implementer's.

`project_axis` consults the override before `project_terms`: a term with an override for this target resolves to the override's paths and never reaches the edge relation, so a later re-poll that changes the global relation cannot silently change a seller's published listings.
An override producing an empty set omits the term, which is why `to` must distinguish empty from absent.
Loss reporting is unchanged: an override resolving to one path where the relation would have broadened is simply not a broadening, and an override that itself broadens is the seller's own disclosed decision.

### Storage shape

One table, one migration, after `0040`.

```sql
create table projection_override (
  org_id       uuid        not null references organisation(id) on delete cascade,
  inventory    inventory_id not null,
  axis         term_kind   not null,
  term_id      uuid        not null references canonical_term(id),
  paths        jsonb       not null,   -- [] means "drop this term for me"
  decided_by   jsonb       not null,
  decided_at   timestamptz not null,
  primary key (org_id, inventory, axis, term_id)
);
```

Row-level security on `org_id` like every other tenant table, so `tam_app` sees one organisation's rows and the cross-tenant lease scan does not read this table at all.
The primary key is the whole preference key, which makes an override idempotent to write and gives the lookup its index for free.
`paths` is `jsonb` rather than a child table because a `VocabularyPath` is already stored that way in `projection_edge.to` and splitting it here would be a second encoding of one shape.

Two follow-ons the design admits but does not build.

A per-seller shelf table is a different thing and is not optional on three targets: Etsy `shop_section_id`, Boom store folders and Shopify collections are per-seller numeric identifiers that must exist on the target before they can be used, so they are populated by reading the target rather than by any crosswalk.
That table is keyed by `(org_id, inventory, remote_id)` and holds no canonical term at all.

And an override is a natural home for the answer a `Narrow` or `ElectOne` election settles as a standing policy, which `ElectionRule` already does per trigger.
Whether the two collapse into one relation is a question for the implementer; they are kept apart here because a rule answers a question shape and an override answers a term, and only the second can say "always this, for me".

## No migration 0040, and when one becomes necessary

Founder decision, 2026-09-03: no data migration is written for the re-base.
No production database exists, the development databases are rebuilt with `just db-reset`, and `tam-taxonomy-seed` produces the whole relation from the captures on a green field, so today the re-seed needs nothing moved.
The number `0040` is released back to whatever lands next.

What a migration would have had to do, recorded so the decision can be revisited rather than rediscovered.
The fifteen moved terms take new deterministic canonical ids, so a re-seed against a database already holding the Tes-minted relation inserts fifteen rows rather than updating fifteen, leaving the old `canonical_term` rows in place along with every row that references them: their `projection_edge` rows in both directions, their `no_counterpart` records, any `reconciliation_item.term`, and any election or settled-election keyed on them.
The move is a pure relabelling — the mapping each term carries is unchanged, as the counts table above shows — so the correct migration is a re-point rather than a delete: update every dependent reference from `tes-year-group:{16..30}` to the corresponding `tpt-grade:{1..14,23}` using the fifteen-row pairing table, then remove the fifteen orphaned terms.
Deleting and re-seeding instead would lose the seller-visible history that hangs off those ids, which is exactly the reconciliation and election rows.

This becomes necessary the day a database must be preserved across the re-base: the first deployed environment that is not rebuilt from scratch, or any development database an operator wants to keep.
Until then the seeder is the whole story.
