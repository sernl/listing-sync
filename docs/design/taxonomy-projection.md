# The taxonomy hub, its algorithm and its open questions

The design specification argues why the model is one canonical internal taxonomy with projections rather than pairwise maps between vocabularies.
This document specifies the projection function in both directions, the response to each `TermProjection` variant, and the authoring question the hub argument leaves open.
Every type named here is defined in [`sketches/domain.rs`](sketches/domain.rs).

## Outbound: canonical to vocabulary

`project(term, vocabulary)` is a pure function over the `projection_edge` relation returning a `TermProjection`.
It selects the edges whose `from` is the term and whose `to.vocabulary` is the target, and it decides on the edge set alone with no fallback, no nearest-neighbour search and no model proposal.

| Edge set | `TermProjection` | Publish |
|---|---|---|
| exactly one `Exact` edge | `Exact { to }` | proceeds |
| one or more `Broader` edges, no `Exact` | `Broadened { to, dropped }` | proceeds |
| more than one `Exact`, or `Exact` and `Broader` disagreeing | `Ambiguous { candidates }` | blocked |
| no edges, and no `NoCounterpart` record | `Absent` | blocked |
| no edges, with a `NoCounterpart` record | the term is omitted | proceeds |

`Broadened` proceeds and this is a decision rather than a hedge: a broader term is a correct category that is less precise than the seller asked for, whereas `Ambiguous` and `Absent` have no correct answer at all.
The dropped canonical terms are recorded on `ListingProjection.loss` and surfaced in the seller's field diff before publish, so a broadening is visible and accepted rather than silent, and it raises no reconciliation item.

`Ambiguous` and `Absent` raise a reconciliation item and block the affected mapping's publish for that inventory without failing the job, so a five-hundred-product batch reports honestly rather than stopping.
Items are deduplicated on the pair of canonical term and target vocabulary, so a batch raises one item per gap rather than one per product.
Resolution writes a durable `ProjectionEdge` or records `NoCounterpart`, which is what makes the queue a drain rather than a treadmill, since the second product carrying the same term finds an edge waiting.

`ProjectionBlocked` is the failure type of the whole listing projection and carries more than taxonomy: `Taxonomy { items }`, `CurrencyUnknown { inventory }`, `CoverMissing` and `ScanIncomplete { file }`.
Those four are the complete publish-gate logic, so a blocked publish always names which of the four blocked it.

## Inbound: vocabulary to canonical

A GB catalogue import produces vocabulary paths and the catalogue stores canonical terms, so the inbound direction is a real function and not a notational convenience.
It is the reverse of the `Exact` edge relation only.
A `Broader` or `Narrower` edge is not invertible, because inverting it would silently restore or invent the distinction the edge dropped, which is exactly the class of wrong edge the design refuses to let anything but a human author.

The reverse index therefore requires uniqueness of `to` among `Exact` edges within a vocabulary, enforced by a unique index on `projection_edge (to_vocabulary, to_path) WHERE kind = 'exact'`.
Two canonical terms claiming the same target path as `Exact` is rejected at insert rather than resolved at read time, which answers the question the outbound direction leaves open: many-to-one is legal through `Broader` and illegal through `Exact`.

An imported path with no reverse edge raises a reconciliation item of the same shape, and the raw path is retained verbatim on the product regardless, so an unmapped import loses nothing and blocks only the outbound projection that would need the canonical term.
That verbatim retention is the same mechanism `GradeDeclaration.raw` uses, and it is what makes the round-trip law hold: for every vocabulary V and term t in V, `emit_V(ingest_V(t)) == t`, by construction rather than by proof about the map.
No corresponding law holds across vocabularies, which is exactly what the reconciliation queue exists to surface.

## Who authors the canonical vocabulary

The hub argument concedes that the canonical vocabulary is the product's own opinion and must be authored, and the research settles neither its size nor its author, so what follows is an assumption rather than a finding.

The assumption is that there is no big-bang authoring pass.
The vocabulary is seeded from the GB uploader vocabulary captured by the M-1 probe, because the founder's own catalogue is GB and every canonical term must exist before a GB import can produce one, and each seeded term gets an `Exact` edge to the GB path it came from with `Decider::Imported`.
The US side is then authored incrementally by the reconciliation queue: the first product carrying a term into the US inventory raises one item, the founder resolves it once, and every later product carrying that term finds the edge waiting.
Queue ownership defaults to the founder rather than the seller, because a missing edge is the product's gap, except for `Ambiguous`, where the choice between two legitimate target terms is genuinely the seller's editorial call.

The size of that work is unknown and the planning figures are an upper bound, not an estimate.
The measured 2,450 US-only and 2,390 GB-only subject and topic paths came from sitemap-derived path sets, which measure the public catalogue rather than the uploader's own selectable vocabulary, and the adapter must satisfy the second.
Obtaining that vocabulary in machine-readable form is an M-1 probe, and until it lands the incremental-authoring assumption above is what the eight-to-eleven-week M1 estimate rests on.
The M1 kill gate is the test of the assumption: if the share of canonical terms raising a new reconciliation item does not fall materially between the first customer catalogue and the tenth, the taxonomy is not converging and the hub is a treadmill rather than an asset.

## What a model may not do

`Decider` is `Imported { source } | Human { user, org }` and a model variant is deliberately absent.
The decision record confines models to listing copy and selector rediscovery, and a wrong edge is invisible, durable and applies to every future product carrying the term, which is a different risk profile from a wrong sentence the seller reads before accepting it.
Spelling is not a transform either, because an edge points at a `VocabularyPath` carrying the target vocabulary's own label, so US spelling is already in the data; normalising spelling is a candidate generator for a human to confirm and must never become an edge on its own.
