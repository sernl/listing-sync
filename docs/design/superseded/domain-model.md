# Domain model

Superseded on 2026-08-25 by [`2026-08-25-listing-sync-design.md`](2026-08-25-listing-sync-design.md), with the taxonomy projection algorithm now in [`taxonomy-projection.md`](taxonomy-projection.md) and the types in [`sketches/domain.rs`](sketches/domain.rs).
This document is retained for its reasoning and is not live; do not implement from it.

The canonical product is the system of record.
Every marketplace listing is a projection of it, derived by a pure function and never authored directly.
That single commitment decides most of what follows: it means a listing has no independent truth to defend, it means divergence between what we intended and what a marketplace holds is a measurable quantity rather than a judgement call, and it means adding a fourth destination costs one projection rather than three reconciliations.

The types below are excerpts from `sketches/domain.rs`, which compiles clean under `rustc 1.97.1` with `-D warnings`.
The file exists because an adversarial review of the first engineering charter found that its flagship artefact had never been compiled and violated its own lint table twelve times; a design document that quotes uncompiled Rust has the same defect.
Reproduce with `rustc --edition 2021 --crate-type lib -D warnings docs/design/sketches/domain.rs --out-dir <tmp>`.
Stand-in types stand where a real crate type will go, so the sketch has no dependency graph at all.

## The unit a projection targets is an inventory, not a marketplace

This is the sharpest modelling decision in the wedge and it is easy to get wrong.
Tes runs disjoint GB and US inventories under one marketplace and one author login.
Two independent measurements found the identifier spaces strictly disjoint: zero intersection across 106,000 sampled ids, and within the shared recent id window 11,087,305 to 13,549,249, `latest-gb` holds 4,999 ids and `latest-us` holds 4,995 with an intersection of exactly zero, reproduced on the 48,000-entry bulk segments ([Tes sitemap index](https://www.tes.com/aws-sitemap-index-teaching-resources.xml)).
Currency follows the inventory rather than the viewer: fetching two US-inventory resources with GB-currency cookies still returned USD offers.

So a model keyed on `Marketplace` cannot express the first chargeable product at all, because both ends of a GB-to-US duplication are Tes.
The projection target is an `InventoryId`, and `Marketplace` is derived from it.

```rust
pub enum InventoryId { TesGb, TesUs, Etsy, Tpt }

pub enum CurrencyRule {
    /// Fixed by the inventory itself.
    Fixed(Currency),
    /// Set by the seller at shop level. Unverified for Etsy and TPT; must be
    /// established before either connector is built.
    SellerScoped,
}

impl InventoryId {
    pub const fn marketplace(self) -> Marketplace { /* ... */ }
    pub const fn currency_rule(self) -> CurrencyRule { /* ... */ }
}
```

The `SellerScoped` variant is not a hedge, it is the honest state of the evidence.
The research establishes inventory-fixed currency for Tes and establishes nothing about how Etsy or TPT decide a listing's currency.
Encoding the unknown as a variant means the Etsy connector cannot be built without someone answering the question, whereas a `Currency` constant would let it be guessed silently.

An account and an inventory are different scopes and the model keeps them apart.
A `Connection` is per marketplace, because one Tes login reaches both inventories, and Tes's own FAQ treats an author holding resources in both as an ordinary state.
A `Mapping` is per inventory, because that is where a listing exists.

## The canonical product and its files

```rust
pub struct CanonicalProduct {
    pub id: ProductId,
    pub org: OrgId,
    pub title: Title,
    pub body: ListingCopy,
    pub payload: PayloadSet,
    pub cover: Option<ProductFile>,
    pub previews: Vec<ProductFile>,
    pub subjects: Vec<CanonicalTermId>,
    pub grades: GradeDeclaration,
    pub price: PriceIntent,
}
```

Three illegal states are removed rather than validated.

A product with no payload cannot be listed anywhere, so `PayloadSet` is a non-empty structure holding a head and a tail rather than a `Vec` with a runtime check.
A price of zero is not a price, so `PriceIntent` is `Free | Paid(Money)` and `Money::new` rejects any amount at or below zero.
The distinction is load-bearing rather than tidy: TPT's Seller Guidelines forbid charging more on TPT for a resource "offered for free or less elsewhere", so free-elsewhere is a case the parity check must see, and a zero in a numeric column does not carry that meaning reliably ([TPT Seller Guidelines, article 360042626591](https://help.teacherspayteachers.com/hc/en-us/articles/360042626591-What-are-TPT-s-Seller-Guidelines)).
A title's per-inventory cap is applied at projection time and never at authoring time, because the caps differ, one of them is measured rather than documented, and one of them counts in an unknown unit.

That last unknown is carried in the type rather than in a comment.

```rust
/// How a marketplace counts a title against its cap. Unverified on TPT, whose
/// 80-character cap was established from a sample containing no astral-plane
/// characters and therefore cannot distinguish these cases.
pub enum LengthUnit { Bytes, Utf16CodeUnits, Codepoints, GraphemeClusters }
```

Files are content-addressed by blake3 hash with a role, a kind and a scan outcome.
The kind enumeration is closed and deliberately excludes `.rar`, because the `unrar` crate advertises `MIT/Apache` on crates.io while statically bundling RARLAB C++ under the non-OSI UnRAR licence, so a `cargo-deny` allow-list keyed on crates.io metadata passes it and ships a compliance problem.
Cover generation is a hard requirement rather than a nicety on Tes specifically, because Tes generates neither a cover nor a preview for ZIP uploads, so a failed cover generation blocks the publish rather than shipping a listing with no visual.

## The binding to a remote listing

A binding ties one canonical product to one remote listing through that marketplace's durable identifier.
The identifiers are structurally different and the type says so, one variant per marketplace, so a TPT identifier cannot be stored where a Tes one belongs.

```rust
pub enum RemoteListingId {
    /// The resource URL, which Tes states stays tied to the original resource
    /// title even after the author retitles it.
    Tes { url: String },
    /// The numeric product id. The slug is decorative: a product path carrying
    /// the wrong slug still serves the correct product.
    Tpt { product_id: u64 },
    Etsy { listing_id: u64 },
}
```

Two facts from the research make this safe and both are worth restating because a plausible alternative fails on each.
On Tes, since 27 November 2025, the URL remains unchanged and tied to the original resource title even when the author later edits the title, so a URL is a stable key and a title rewrite does not sever the binding.
On TPT, requesting a product path with a deliberately wrong slug returns HTTP 200 and serves the correct product, so the numeric id is the key and the slug is decoration.
Never use a marketplace CDN image URL as a sync key: TPT's image paths embed a cache-busting epoch and change on every re-upload.

The binding's state machine carries the hardest problem in the system as a first-class variant.

```rust
pub enum Binding {
    Unbound,
    /// A create is in flight. There is no durable identifier yet, which is
    /// exactly what makes an ambiguous create the hardest state in the system.
    Creating { attempt: AttemptId, marker: Option<CorrelationMarker> },
    Bound { id: RemoteListingId, first_seen: Timestamp, verified: Verification },
    /// A create may or may not have landed and reconciliation could not decide.
    /// Never retried; escalated, with this inventory halted for this tenant.
    AmbiguousCreate { attempt: AttemptId, candidates: Vec<RemoteListingId>, since: Timestamp },
    Severed { was: RemoteListingId, noticed: Timestamp, cause: SeverCause },
}
```

Both durable keys are assigned by the marketplace at creation, so on an ambiguous create the key does not exist yet, which is precisely what the ambiguity consists of.
Reconciliation then means matching on title plus content fingerprint against the author's recent resources, and the fingerprint will frequently not match because marketplaces sanitise HTML, trim whitespace, transcode images and rewrite descriptions.
The governing rule is that an ambiguous attempt is never retried: it is reconciled, and if reconciliation cannot decide it is escalated with that inventory halted for that tenant, because the Tes Author Code says "Please do not upload duplicate copies of your resources" and a duplicate-upload storm is not recoverable while a stalled queue is ([Tes Author Code](https://www.tes.com/teaching-resources/author-code)).
The `CorrelationMarker` is the second-choice mitigation, a token embedded in a seller-visible field so an ambiguous create can be reconciled at the cost of pollution the seller must be told about.
The first-choice mitigation is save-as-draft, which splits one dangerous write into a harmless create and an idempotent publish, and whether either marketplace supports it is an open probe rather than an assumption.

## Verification is a field diff, not an existence check

`Bound` carries a `Verification` rather than a boolean, because "a write landed" and "the right write landed" are different questions and only the second one matters to a seller whose storefront is their livelihood.

```rust
pub enum Verification {
    Stale { since: Timestamp },
    Clean { at: Timestamp },
    Mismatched { at: Timestamp, first: FieldMismatch, rest: Vec<FieldMismatch> },
}
```

`Mismatched` is non-empty by construction, so the state cannot claim a mismatch it cannot name.
Each mismatch is classified, and the response is a total function of the class rather than a runbook paragraph, so an agent adding a class cannot leave it without an action.

| Class | Response | Why |
|---|---|---|
| `Normalised` | Accept | Entity re-encoding, curly quotes and whitespace collapse are the marketplace behaving normally |
| `Truncated` | Degrade | The listing is live and shorter than intended; resending makes it worse |
| `Missing` | Halt inventory | A field we wrote is absent, so the form or the adapter has changed |
| `WrongField` | Halt and page | A value landed somewhere else, which is how a price reaches a quantity box |
| `Unexpected` | Halt and page | A value appeared that we never wrote |

Without a per-field normaliser handling entity re-encoding, Unicode normalisation, curly quotes, whitespace collapse, description truncation, tag reordering, slug generation and price rounding, this control emits mismatch alerts continuously, the founder learns to ignore them within a week, and the control becomes worse than absent because it is believed to exist.
The normaliser carries a version, and that version is recorded in the audit row, so a change to normalisation is distinguishable from a change in the marketplace.

## The mapping and its sync policy

```rust
pub struct Mapping {
    pub id: MappingId,
    pub org: OrgId,
    pub product: ProductId,
    pub inventory: InventoryId,
    pub binding: Binding,
    pub policies: FieldPolicies,
    pub price_rule: PriceRule,
    pub publish: PublishMode,
    pub lifecycle: RemoteLifecycle,
}
```

`FieldPolicies` is a struct with one field per `FieldKey` rather than a map, so a policy can be neither missing nor unknown, and adding a syncable field is a compile error at every construction site rather than a silent default.
Each field is `Managed`, meaning we own it and a difference is a defect corrected next run; `Frozen`, meaning the seller edited it on the marketplace and we read it back but never write it; or `Propose`, meaning we compute a value and present it for a per-field accept.
Auto-publish is earned per seller, per field, per inventory after a run of clean accepts, which is why the policy is per field rather than per mapping.

`PublishMode` is `DryRun | Propose | Publish` and defaults to `DryRun`, because under TPT's Virtual Assistant terms the seller is liable for the platform's errors "as if those actions were taken by you directly" while TPT gives them no detailed change history, and because Tes routes its own indemnity onto exactly the target cohort of VAT-registered authors and authors earning over £10,000.
Destructive operations are absent from the vocabulary in phase one rather than disabled.

`RemoteLifecycle` is a state machine and not a boolean, because Tes states that "due to security measures, your resource may not be published on Tes resources for up to 3 working days", so submitted, in review, live and rejected are four different things a seller needs to see differently.
The `Draft` variant exists only if a marketplace supports save-as-draft, which the M-1 probe settles.

Price derivation and price parity are kept apart because they are different questions.
`PriceRule` says how this inventory's price is computed, either converted from the canonical price at a rate recorded on the mapping or set explicitly by the seller.
Parity is a cross-inventory invariant checked over the set of bound mappings, and in the wedge it is inert, because the research establishes a parity rule for TPT against other channels and establishes no such rule between Tes GB and Tes US.
Building the check now and leaving it unarmed is cheap; discovering it after a TPT connector ships is not.
Note the passive breach mode: any promotion, currency movement or price edit that puts one channel below TPT places the seller in breach with nobody taking an action, so parity is a scheduled evaluation and not only a write-time guard.

## Reads are a capability, not a permission

The three-tier read hierarchy is enforced in types rather than in a policy document.

```rust
pub enum FetchReason {
    /// Tier one: the marketplace's own first-party export of the seller's data.
    FirstPartyExport { inventory: InventoryId },
    /// Tier two: one fetch of one listing by a durable identifier already held,
    /// caused by and immediately following an authorised write.
    VerifyWrite { receipt: WriteReceipt },
    /// Tier two, stretched: a listing whose moderation outcome is still pending
    /// is polled by the same durable identifier on a bounded schedule.
    PollLifecycle { receipt: WriteReceipt },
    /// The hourly read-only structural probe, which resolves the current
    /// selectors against a form and creates, submits and deletes nothing.
    StructuralProbe { grant: CanaryGrant },
}
```

`WriteReceipt` has private fields and a crate-scoped constructor called from exactly one place, the settle step of the write path.
Every variant names a reason the read is permitted, and there is no constructor for any other reason, so link-following, listing pages, search and pagination are unrepresentable rather than merely forbidden.
The `PollLifecycle` variant stretches the rule and the name says so: a Tes resource under moderation must be polled for up to three working days, which is caused by an authorised write and addressed by a durable identifier but is not immediately consequent on it.
`CanaryGrant` is issued per marketplace from a recorded permission decision rather than being ambient, which is what keeps the structural probe available on Tes and unreachable on TPT until written permission exists.

This matters most on TPT and it is why TPT is gated.
TPT's Community Guidelines prohibit using "any automated means such as bots, spiders, or crawlers to download or otherwise obtain data from our services", with no volume threshold, no ownership carve-out and no purpose limitation ([Guidelines for All TPT'ers, article 360043018571](https://help.teacherspayteachers.com/hc/en-us/articles/360043018571--Guidelines-for-All-TPT-ers)).
The correctness discipline this design rests on is itself automated means obtaining data, so on TPT compliance is in tension not only with access but with verification, and no engineering choice resolves that.
The defensible argument is different in kind and applies to tier one: TPT itself provisions a Product Statistics CSV export, a Sales Details download and a Privacy Center access-request flow, which demonstrates the platform's own distinction between a seller obtaining their own data and a third party extracting marketplace data.

## Three outcomes, not two

```rust
pub enum WriteOutcome {
    Committed(WriteReceipt),
    Rejected { code: FailureCode, detail: String },
    Ambiguous { attempt: AttemptId, evidence: EvidenceRef },
}
```

The commit boundary is intent recorded, not response received: a `write_attempt` row is written before the click as a fencing token and outbox record, then the attempt is settled from ranked evidence.
A network status line is not authoritative, because a marketplace returning HTTP 200 with a JSON error body would score committed if status alone decided it.
A DOM success banner is never terminal, and neither is the driver's own signal.
A read-back by durable identifier is the only thing that settles genuine ambiguity.

`FailureCode` is closed, versioned and low-cardinality with an explicit other-case, following the discipline that an error type "SHOULD be predictable, and SHOULD have low cardinality" ([OpenTelemetry semantic conventions](https://opentelemetry.io/docs/specs/semconv/registry/attributes/error/)).
The reason is operational rather than aesthetic: free-text-only reasons make a failure list unfilterable at two hundred items, which is exactly the scale that matters.
`SelectorResolvedViaFallback` is the member that earns its place first, because a selector degraded to its fallback is a break that has not surfaced yet, which makes fallback share a leading indicator available in week one against a kill criterion that otherwise takes two quarters to evaluate.

## The taxonomy problem

This is the hardest modelling work in the wedge, and unlike most of the system it cannot be deferred, because a GB-to-US duplication is a taxonomy remap with a file attached.

### What was measured

Tes GB and Tes US are not a relabelling of one tree.
The research measured 2,450 subject and topic paths that exist in US and not GB, 2,390 that exist in GB and not US, a four-phase GB structure against a five-phase US structure, and US spelling running through the tree.
GB against AU, by contrast, is a pure top-segment relabel, which is what makes the GB-to-US case the genuine one.
The addendum did not overturn any of this.

Those counts came from sitemap-derived path sets and should be treated as measurements of the public catalogue's paths rather than of the uploader's own selectable vocabulary, which is the thing the adapter must actually satisfy.
Obtaining the uploader's vocabulary in machine-readable form is an M-1 probe, and the numbers above are the planning figures until it lands.

### Why an N-projection model beats N(N-1) pairwise maps

Model one canonical internal taxonomy and give each vocabulary a projection into and out of it, rather than mapping vocabularies to each other in pairs.

Count first, within a single term kind such as subject.
Pairwise directed maps between N vocabularies number N(N-1); projections number 2N, one ingest and one emit per vocabulary.
At the four inventories this product plans for, that is 12 pairwise maps against 8 projections, and the fifth vocabulary costs 8 more pairwise maps against 2 more projections.
The gradient matters more than the level, because the whole premise of the wedge is that every component is reused verbatim when a second marketplace arrives.

Consistency matters more than count.
Pairwise maps admit incoherence that nothing detects: GB to US, GB to TPT and TPT to US need not compose, and there is no place to notice that they disagree.
Through a hub, composition is automatic, and a disagreement becomes a single-term question about one canonical term rather than a triangle nobody owns.
Review cost follows the same shape: a canonical term's meaning is asserted once, so the review surface is proportional to terms rather than to terms times pairs.

The honest cost is threefold and should be stated rather than argued away.
The canonical vocabulary is the product's own opinion and has to be authored and maintained by someone.
A hub is lossy wherever a source distinction has no canonical counterpart, which is why loss is recorded as data rather than discarded.
And at N equal to two, which is exactly the wedge, pairwise is cheaper: two maps against four projections.
The hub does not pay for itself until the third vocabulary, which is Etsy.
Building it during the wedge anyway is a deliberate trade: the wedge is where the tables and the reconciliation queue get exercised against real data at real volume, and retrofitting a hub after a pairwise GB-to-US map has shipped means re-deriving every edge from a map that never recorded why it existed.
That is a judgement rather than a finding, and it is the judgement the founder's own reasoning already implies when it calls the wedge's components reusable verbatim.

### The shape of the model

```rust
pub struct CanonicalTerm {
    pub id: CanonicalTermId,
    pub kind: TermKind,                    // Subject | Topic | ResourceType | Phase
    pub parent: Option<CanonicalTermId>,
    pub label: String,
}

/// A vocabulary is per inventory, not per marketplace, because Tes GB and Tes
/// US were measured as structurally different trees.
pub struct VocabularyId(pub InventoryId, pub TermKind);

pub struct ProjectionEdge {
    pub from: CanonicalTermId,
    pub to: VocabularyPath,
    pub kind: EdgeKind,                    // Exact | Broader | Narrower
    pub decided_by: Decider,
    pub decided_at: Timestamp,
}
```

`Decider` is `Imported { source } | Human { user, org }`.
A model variant is deliberately absent: the decision record confines large language models to listing-copy generation and to rediscovering a selector after a markup change, so a model may not author a taxonomy edge.
The bar this clears is that a wrong edge is invisible, durable, and applies to every future product that carries the term, which is a different risk profile from a wrong sentence in a description that the seller reads before accepting it.

Spelling is not a transform and treating it as one is a mistake worth naming.
An edge points at a `VocabularyPath` that carries the target vocabulary's own label, so US spelling is already in the data and needs no rewriting at emit time.
Where orthography does work is at ingest, as a candidate generator: normalising spelling on both sides is a cheap way to propose edges for a human to confirm, and it must never become an edge on its own, because a spelling-equal label is evidence of a match and not proof of one.
The other place spelling matters is listing copy, which is the US-English rewrite in the copy pass and a separate concern from the taxonomy entirely.

### What happens to a term with no edge

A projection of one canonical term into one target vocabulary has four outcomes and `Absent` is one of them.

```rust
pub enum TermProjection {
    Exact { to: VocabularyPath },
    Broadened { to: VocabularyPath, dropped: Vec<CanonicalTermId> },
    Ambiguous { candidates: Vec<VocabularyPath> },
    Absent,
}
```

`Broadened` is the honest lossy case: the nearest target term is an ancestor, so the listing keeps precision it can support and records the precision it dropped.
`Ambiguous` is a genuine editorial choice between target terms that the data does not settle.
`Absent` means the concept has no counterpart at all.

`Ambiguous` and `Absent` both raise a reconciliation item and block the affected mapping's publish for that inventory.
They do not fail the job: other items proceed, and the blocked item settles as `Blocked { on: Reconciliation }` so a five-hundred-product batch reports honestly rather than stopping.
Items are deduplicated on the pair of canonical term and target vocabulary, so a batch raises one item per gap rather than one per product.

There is no silent default and no default at all.
No "Other" term, no "Miscellaneous", no nearest-neighbour write, no model proposal.
The reason is that a wrong category is a listing that never gets found, and the seller has no way to discover it because the listing looks fine.

Resolution writes a durable artefact rather than patching a listing.

```rust
pub enum ReconciliationState {
    Open,
    /// Resolved by writing a durable edge, so the queue drains rather than
    /// re-raising the same term on the next product.
    Resolved { edge: ProjectionEdge },
    /// The term genuinely has no counterpart and the mapping must omit it.
    NoCounterpart { decided_by: Decider, at: Timestamp },
}
```

That is what makes the queue a drain rather than a treadmill: the second product carrying the same term finds an edge waiting.

Ownership of the queue is a judgement the research does not settle, and the assumption taken here is that a missing edge is the product's gap rather than the seller's, so the default queue is the founder's and the seller sees only that some products are waiting on a category being added.
The exception is `Ambiguous`, where the choice between two legitimate target terms is genuinely the seller's editorial call and asking them is better than guessing on their behalf.

### Grades carry the declaration, not only the derived value

```rust
pub struct GradeDeclaration {
    pub source: DeclarationSource,          // Imported { vocabulary } | Seller
    pub raw: Vec<VocabularyPath>,
    pub derived: Option<AgeInterval>,
}
```

The declaration is the fact and the age interval is derived from it.
Emitting back to the vocabulary a declaration came from uses the recorded declaration verbatim rather than round-tripping through the interval, which gives an algebraic law worth property-testing over the measured path corpus:

> for every vocabulary V and every term t in V, `emit_V(ingest_V(t)) == t`

It holds by construction rather than by proof about the map, because ingest stores the source path.
That distinction is the point: the law is enforced by the data model, so a mapping table that later grows a wrong edge cannot break a seller's own declaration on their own inventory.

No corresponding law holds across vocabularies, and that asymmetry is exactly what the reconciliation queue exists to surface.
A four-phase GB structure and a five-phase US structure are not in bijection, so the remap is many-to-many and an age interval is the only common denominator available.
The interval is a bridge and a lossy one, which is why it never replaces the declaration.

`AgeInterval::new` rejects an inverted interval, and the endpoints are data rather than constants.
The research does not establish the age boundaries of either marketplace's phase or grade labels, so those boundaries are sourced from each marketplace's published vocabulary at M-1 and stored in `vocabulary_term`, never hard-coded.
Writing them into code from memory is the failure mode this paragraph exists to prevent.
