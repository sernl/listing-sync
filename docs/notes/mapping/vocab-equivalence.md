# Cross-platform vocabulary equivalence: TPT and Tes

- date: 2026-08-29
- method: live poll of every create and edit dropdown vocabulary on both platforms against the founder's own seller sessions
- evidence: docs/design/data/tpt-vocabulary.json, docs/design/data/tes-vocabulary.json, docs/design/data/tes-taxonomy-GB.json, tes-taxonomy-NZ.json, tes-taxonomy-refs.json

Input to the field-mapping work.
Concrete about the axes and deliberately not exhaustive on leaves.

## The shape of the problem

The two platforms disagree at the level of structure, not just labels.

TPT has one flat slug namespace of 358 facets where a grade, a subject, a resource type, a file format and a language are all the same kind of thing, distinguished only by a `category` field that is metadata about the slug rather than a separate field on the product.
A TPT product carries `taxonomyTags: [...]` and the reader partitions that list by category to recover what the seller meant.

Tes has separate, typed fields: `mainType` for resource type, `ageRanges` or `yearGroups` for audience age, `categories` plus `primaryCategory` for subject and topic, `licence`, and a three-level curriculum cascade.
Each is its own vocabulary with its own ids.

So the mapping is not vocabulary to vocabulary.
It is one TPT field fanning out into five Tes fields on the way out, and five Tes fields collapsing into one TPT tag array on the way back.
The collapse direction is lossy in a way the fan-out is not, and that asymmetry belongs in the mapping types rather than being discovered at runtime.

## Axis 1: age and grade, mappable with a country fork

The cleanest axis, and the one that carries a trap.

TPT is grade-native: 19 create-form options, PreK through 12th, plus Higher Education, Adult Education, Homeschool, Staff and Not Grade Specific.

Tes is age-native only in GB.
The editor picks its age field by country: `ageResourceFieldName = (country === "GB") ? "ageRanges" : "yearGroups"`.
`ageRanges` is 7 coarse bands (3-5, 5-7, 7-11, 11-14, 14-16, 16+, not applicable).
`yearGroups` is 30 rows split into GB years (Nursery, Reception, 1 through 13) and US grades (Pre-K, Kindergarten, 1st through 12th, not applicable).

The US `yearGroups` rows are a near-exact one-to-one with the TPT grade list.
For a US-market resource this axis is a lookup table rather than an approximation: TPT grade 1 (PreK) to Tes 16 (Pre-K), 2 (Kindergarten) to 17 (Kindergarten), 3 through 14 (1st through 12th) to 18 through 29, and 23 (Not Grade Specific) to 30 (Age not applicable).

TPT's Higher Education (15), Adult Education (16), Homeschool (17) and Staff (19) have no `yearGroups` counterpart and fall off the edge.
Homeschool and Staff are `category: audience` facets on TPT rather than grades, which is a hint that they route elsewhere entirely.

For a GB-market resource the mapping goes through `humanAges`.
Every Tes row on both sides carries a `humanAges` array and the TPT grade list can be given one, so band overlap on integer ages is a principled join rather than a hand-written table.
That is the right implementation for both directions: join on ages, then pick the narrowest covering band.
Going TPT to Tes GB is lossy by construction, because 14 grades collapse into 6 bands, and going back cannot recover the original grade.

The suggested model is `Grades -> {UsYearGroups | GbAgeBands}` with the country as an input, and the GB direction returning a widened range rather than pretending to be exact.

## Axis 2: subject, mappable in principle and expensive in practice

TPT carries `taxonomyTags` with `category: PreK-12-Subject-Area`: 140 facets under 20 roots, max depth 2.
Tes carries `categories: [{id}]` plus `primaryCategory`, drawn from a 43-root and roughly 450-topic tree per country.

Both are two-level subject and topic trees of comparable size, so the axis is real.
There is no shared identifier and no shared naming convention, so this needs a hand-built crosswalk at the root level (20 TPT roots against 43 Tes roots) with topic-level refinement where it pays.

Two things make it tractable.
Tes ships `mapTo` on every taxonomy node, which is its own cross-country equivalence key, so a crosswalk is built against `mapTo` once rather than against each country separately.
TPT facets carry `algoliaFacetIds` and `legacyId`, which gives a stable anchor on the TPT side across their taxonomy revisions.

Cardinality differs.
Tes caps selection at roughly 10 subject and topic pairs and requires exactly one `primaryCategory`; TPT imposes no observed cap and has no notion of a primary tag.
TPT to Tes must therefore choose a primary, and that choice is not recoverable from TPT data.
Deriving it from the first tag by `sort` order is a defensible default, but it is a decision rather than a translation.

## Axis 3: resource type, mappable and small enough for a hand table

TPT carries `taxonomyTags` with `category: Type-of-Resource`: 71 facets under 10 roots — Classroom Decor, Clip Art, Forms, Hands-on Activities, Independent Work, Instruction, Printables, Student Assessment, Student Practice and Teacher Tools.
Tes carries `mainType`, exactly 9 selectable values — Assembly, Assessment and revision, Game/puzzle/quiz, Audio music and video, Lesson (complete), Other, Unit of work, Visual aid/Display and Worksheet/Activity.

This is the most favourable axis for a hand-written table, because the Tes side is 9 values wide and has an `Other` escape hatch.
The relation is many-to-one going TPT to Tes and one-to-many coming back.
Tes takes exactly one `mainType` while TPT accepts several Type-of-Resource tags at once, so TPT to Tes picks one and Tes to TPT emits one tag and lets the seller add more.

## Axis 4: licence, no counterpart, and the axis that matters most

Tes has a real licence vocabulary: CC-BY, CC-BY-SA and CC-BY-ND for free resources, TES-PAID and TES-PAID-SCHOOL for paid, plus legacy TES-V1 and TES-V2 that read back on old resources.

TPT has no licence field at all.
The nearest levers are `copyrightDeclaration` (ORIGINAL_WORK or USED_COPYRIGHTED_MATERIALS) and the multiple-license price, which is a pricing mechanism rather than a rights grant.

There is no honest mapping in either direction.
Tes to TPT loses the licence outright: a CC-BY Tes resource becomes an ordinary priced TPT product with no CC grant recorded anywhere, which is a rights-semantics loss rather than a formatting loss.
TPT to Tes has to supply a licence, because Tes requires one; the price determines the branch, since paid implies TES-PAID and free implies one of the three CC values, but which CC licence a free TPT product should carry is not derivable from any TPT field.

The suggested policy is not to default this.
TPT to Tes for a free product requires an explicit seller choice, and Tes to TPT surfaces a warning that the licence is being dropped.
This is the one axis where a silent default has legal content.

## Axis 5: curriculum and standards, structurally analogous and disjoint in content

Both platforms have a hierarchical curriculum-alignment feature and neither aligns with the other.

TPT has 166 education-standards jurisdictions — Common Core, NGSS, TEKS, Virginia SOL, state frameworks — expanded via `EducationStandardsQuery($id, $depth)` into subject, domain and eventually standard, with products posting per-standard leaf ids.
The set is overwhelmingly US.

Tes has a three-level cascade: `orientation` (11 selectable national curricula), then `framework` (48, filtered by orientation and by the resource's ages), then `authority` (23 awarding bodies, filtered by framework).
This describes which curriculum the resource targets rather than which specific standards it meets.

These are different kinds of statement.
TPT says a resource meets CCSS.MATH.CONTENT.5.NBT.A.1; Tes says a resource targets the English National Curriculum, GCSE, AQA.
Neither is derivable from the other.

One narrow bridge exists: Tes `orientation: American` with `framework: Common Core` is the same territory as TPT's Common Core jurisdiction (id 3054), so a TPT product carrying CCSS standards can justifiably set that pair, losing the specific standards but preserving the alignment claim.
There is no reverse direction, because Tes carries no standard ids.
The coarse mapping exists and the fine mapping does not.

## Axis 6: platform-specific with no counterpart at all

TPT-only, and uncaptured on the Tes side because Tes has no such concept: `teachingDuration` (23 options, N/A through Lifelong tool), `answerKey` (6 options), `audience` (8 options plus a free-text `audienceText`, and a separate scalar from the `audience`-category facets), `videoType` (8 options, video products only), the 5 US digital-goods tax codes, and the seller's own custom store categories.
Tes has `collections`, which is the nearest thing to those store categories but is not a create-form dropdown.

Tes-only: `authority`, the awarding body, 23 values, where TPT has no exam-board concept; TES-PAID-SCHOOL, a school-licence tier with no TPT equivalent; per-country price floors (US 1.50, everywhere else 1.00, max 300.00) against TPT's single 0.95 floor, which a migration can violate and which therefore needs a guard; and `mapTo`, the cross-country subject equivalence internal to Tes.

## Hierarchical against flat, summarised

| Vocabulary | TPT | Tes |
|---|---|---|
| Grades and ages | flat, 19 | flat, 7 GB bands or 30 year groups across two countries |
| Subject | 2-level tree, 140 facets under 20 roots | 2-level tree, roughly 450 topics under 43 roots, per country |
| Resource type | 2-level tree, 71 facets under 10 roots | flat, 9 |
| Licence | absent | flat, 5 selectable plus 2 legacy |
| Curriculum | 166 jurisdictions over a deep standards tree | 3-level cascade, 11 then 48 then 23 |
| Duration, answer key, audience, video type | flat scalars of 23, 6, 8 and 8 | absent |
| Tax code | flat, 5 | absent |

## Suggested build order

1. Grades against US year groups: a 19 by 30 table with 16 exact rows and 4 explicit no-counterpart entries. The cheapest and highest-confidence axis, and it unblocks any US-to-US migration.
2. Resource type: 71 to 9, hand-written, with Other (99006) as the documented fallback.
3. Licence policy: not a table but a decision procedure with a required seller input on the TPT-to-Tes free-product branch. Worth settling before any bulk migration runs.
4. Subject crosswalk: the expensive one. Build it against Tes `mapTo` ids and TPT `legacyId`, root level first, accepting that topic-level coverage will be partial for a long time.
5. Curriculum: coarse only, mapping CCSS-tagged TPT products to Tes American and Common Core. No standard-level translation in either direction.
