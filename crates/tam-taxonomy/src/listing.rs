//! The composed listing projection: one pure function from the canonical
//! product to the per-inventory rendering, or to exactly one of the four
//! publish gates — taxonomy, currency, cover, scan — so a blocked publish
//! always names what blocked it. Gates are checked in the enum's own order,
//! which makes the answer deterministic when several would fire.
//!
//! M1 projects copy verbatim (localisation is M4's) and passes grade paths
//! through with native ids intact, because Tes's age-range vocabulary is
//! account-scoped rather than inventory-scoped. A `Free` price needs no
//! currency, so free listings pass an unmeasured-currency inventory while a
//! priced one blocks honestly until the probe lands.
//!
//! Verbatim is qualified by the field registry: a cap it declares for the
//! target inventory is applied here, at projection, and never at authoring,
//! which is where the seller's own text stays whole. No Tes inventory
//! declares one, so every Tes projection is byte-identical to its product.

use std::collections::HashMap;

use tam_domain::equivalence::{
    Election, ElectionRule, Loss, PricingBranch, SettledElection, VocabularyGap,
};
use tam_domain::registry::{registry, truncate, AxisBinding, FieldSpec};
use tam_domain::{
    CanonicalProduct, CanonicalTerm, ListingProjection, ProjectionBlocked, ProjectionEdge,
    RightsDeclaration, TermKind, TermProjection, VocabularyId, VocabularyPath,
};
use tam_types::{
    CanonicalTermId, CurrencyRule, InventoryId, MappingId, OrgId, PriceIntent, ScanOutcome,
    Timestamp,
};

use crate::project::{ingest, ingest_grades, project_axis, AxisRequest};

/// The axes this projection routes, and the reason the list is shorter than
/// the registry's.
///
/// `Licence` joined when the election queue existed to hold the question:
/// Tes declares it required, so a product whose rights are unstated raises a
/// `Supply` election rather than being published under a grant nobody chose.
///
/// `ResourceType` joins now that `resource_types.rs` seeds its crosswalk. It
/// reaches Tes and not the reverse, and that is a property of the relation
/// rather than of this list: every Tes-side edge on the axis is `Broader`, so
/// no Tes `mainType` value ingests to a canonical term, so a Tes-sourced
/// product carries no resource type into a TPT create. D13 denies a
/// Type-of-Resource control on that route, and
/// `no_tes_resource_type_value_ingests_to_a_canonical_term` is what holds it.
const ROUTED_AXES: [TermKind; 5] = [
    TermKind::Subject,
    TermKind::Topic,
    TermKind::ResourceType,
    TermKind::Phase,
    TermKind::Licence,
];

fn routed_axes(inventory: InventoryId) -> impl Iterator<Item = AxisBinding> {
    registry(inventory)
        .equivalence_axes
        .iter()
        .copied()
        .filter(|binding| ROUTED_AXES.contains(&binding.axis))
}

/// The vocabularies one inventory's routed axes name, independent of any
/// product. The inbound half reads these too: an import ingests the source's
/// own ids over the source's own relation.
#[must_use]
pub fn routed_vocabularies(inventory: InventoryId) -> Vec<VocabularyId> {
    routed_axes(inventory)
        .map(|binding| VocabularyId(inventory, binding.axis))
        .collect()
}

/// Every vocabulary a projection into one inventory reads.
///
/// The target's own bound axes, plus the vocabularies the product's grade
/// declaration names, because a grade ingests from the source's relation
/// before it projects into the target's. Callers load edges for exactly these
/// and no longer restate a two-kind list the registry already holds.
#[must_use]
pub fn projection_vocabularies(
    inventory: InventoryId,
    product: &CanonicalProduct,
) -> Vec<VocabularyId> {
    let mut wanted = routed_vocabularies(inventory);
    let declared = rights_source(product).into_iter();
    for path in product.grades.raw.iter().chain(declared) {
        if !wanted.contains(&path.vocabulary) {
            wanted.push(path.vocabulary);
        }
    }
    wanted
}

/// The path a product's rights were declared as, which the licence axis
/// ingests from exactly as a grade ingests from its source vocabulary.
fn rights_source(product: &CanonicalProduct) -> Option<&VocabularyPath> {
    match &product.rights {
        RightsDeclaration::Unstated => None,
        RightsDeclaration::Declared { source } => Some(source),
    }
}

/// Everything the projection decides over beyond the product itself: the
/// tenant scope the raised items carry, the target, the instant, and the
/// relation slices the pure functions read.
pub struct ListingContext<'a> {
    pub org: OrgId,
    pub mapping: MappingId,
    pub inventory: InventoryId,
    pub now: Timestamp,
    pub terms: &'a [CanonicalTerm],
    pub edges: &'a [ProjectionEdge],
    pub no_counterparts: &'a [(CanonicalTermId, VocabularyId)],
    /// The tenant's standing election answers, so a decision already taken as
    /// a policy resolves here instead of being asked again.
    pub rules: &'a [ElectionRule],
    /// The questions this product's own seller has already settled, so an
    /// answer that revived a parked item resolves it rather than re-raising
    /// the identical question.
    pub settled: &'a [SettledElection],
}

pub fn project_listing(
    product: &CanonicalProduct,
    ctx: &ListingContext<'_>,
) -> Result<ListingProjection, ProjectionBlocked> {
    // Lookup only — no iteration order ever reaches the output.
    let kinds: HashMap<CanonicalTermId, TermKind> =
        ctx.terms.iter().map(|term| (term.id, term.kind)).collect();

    // Gate one: the equivalence relation, one axis at a time, driven off the
    // registry's own bindings rather than a second hardcoded list.
    //
    // Grades arrive as the source platform's own paths rather than as
    // canonical ids, because `GradeDeclaration` keeps them verbatim so a round
    // trip loses nothing, so the axis ingests before it projects. That ingest
    // is the whole D1 fix: a grade now travels through the relation and
    // reaches the target under the target vocabulary's own native id, where
    // before it kept the source's id and only its vocabulary label changed.
    let ingested = ingest_grades(&product.grades, ctx.edges);
    // Rights enter the relation the same way grades do: as the source
    // platform's own path, translated into the target's own token. A stated
    // grant the relation does not recognise is carried out as unrecognised
    // rather than dropped, and the axis then has no value, so the seller is
    // asked for one they can state in a vocabulary we hold.
    let mut unrecognised = ingested.unrecognised;
    let mut rights_terms: Vec<CanonicalTermId> = Vec::new();
    let mut rights_sources: Vec<VocabularyPath> = Vec::new();
    if let Some(source) = rights_source(product) {
        match ingest(source, ctx.edges) {
            Some(term) => {
                rights_terms.push(term);
                rights_sources.push(source.clone());
            }
            None => unrecognised.push(source.clone()),
        }
    }
    let pricing = match product.price {
        PriceIntent::Free => PricingBranch::Free,
        PriceIntent::Paid(_) => PricingBranch::Paid,
    };

    let mut included = Vec::new();
    let mut grades = Vec::new();
    let mut natives: Vec<(TermKind, VocabularyPath)> = Vec::new();
    let mut loss: Vec<Loss> = Vec::new();
    let mut gaps: Vec<VocabularyGap> = Vec::new();
    let mut elections: Vec<Election> = Vec::new();
    for binding in routed_axes(ctx.inventory) {
        let of_kind: Vec<CanonicalTermId> = product
            .subjects
            .iter()
            .copied()
            .filter(|term| kinds.get(term) == Some(&binding.axis))
            .collect();
        let (terms, sources): (&[CanonicalTermId], &[VocabularyPath]) = match binding.axis {
            TermKind::Phase => (&ingested.terms, &ingested.sources),
            TermKind::Licence => (&rights_terms, &rights_sources),
            TermKind::Subject | TermKind::Topic | TermKind::ResourceType => (&of_kind, &[]),
        };
        // An empty set is not a reason to skip the axis: a target that
        // requires a value asks for one precisely when the source carried
        // none, which is the whole TPT-to-Tes licence case. `project_axis`
        // returns an empty outcome for every axis that requires nothing.
        let outcome = project_axis(
            AxisRequest {
                product: product.id,
                inventory: ctx.inventory,
                binding,
                terms,
                sources,
                pricing,
                rules: ctx.rules,
                settled: ctx.settled,
            },
            ctx.edges,
            ctx.no_counterparts,
        );
        match binding.axis {
            TermKind::Subject | TermKind::Topic => included.extend(outcome.resolved),
            TermKind::Phase => grades.extend(outcome.resolved),
            // An axis the seam's listing names no field for travels labelled
            // by the axis it answers. Empty until the licence axis is routed,
            // and shaped this way now so that routing is one entry in
            // ROUTED_AXES rather than a second carriage mechanism.
            TermKind::ResourceType | TermKind::Licence => natives.extend(
                outcome
                    .resolved
                    .into_iter()
                    .map(|path| (binding.axis, path)),
            ),
        }
        loss.extend(outcome.loss);
        gaps.extend(outcome.gaps);
        elections.extend(outcome.elections);
    }
    // What the target was measured to have no field for, disclosed rather
    // than dropped. `routed_axes` iterates the axes the target *binds*, so an
    // axis it declares absent is never visited and its value simply vanishes:
    // that is the Tes-to-TPT licence drop. TPT holds no licence field anywhere
    // on its wire, so no edge could ever exist and no queue item could ever be
    // answered -- which leaves disclosure as the only honest outcome, and is
    // the whole difference between a measured absence and the unmeasured one
    // Etsy has, which still blocks.
    for absent in registry(ctx.inventory).absent_axes {
        for value in carried_in(product, absent.0) {
            loss.push(Loss::NoTargetField {
                axis: absent.0,
                value,
            });
        }
    }
    // A term the catalogue does not classify cannot be projected and cannot
    // be silently dropped: it blocks as its own queue item under the subject
    // vocabulary, which is the fail-closed reading of an impossible input.
    for term in &product.subjects {
        if !kinds.contains_key(term) {
            gaps.push(VocabularyGap {
                term: *term,
                target: VocabularyId(ctx.inventory, TermKind::Subject),
                projection: TermProjection::Absent,
            });
        }
    }
    if !gaps.is_empty() || !elections.is_empty() {
        return Err(ProjectionBlocked::Blocked {
            gaps,
            elections,
            unrecognised,
            loss,
        });
    }

    // Gate two: currency. Free needs none; a priced listing into an
    // inventory whose currency is unmeasured or unverified blocks.
    if let PriceIntent::Paid(_) = product.price {
        match ctx.inventory.currency_rule() {
            CurrencyRule::Fixed(_) => {}
            CurrencyRule::SellerScoped | CurrencyRule::Unmeasured => {
                return Err(ProjectionBlocked::CurrencyUnknown {
                    inventory: ctx.inventory,
                });
            }
        }
    }

    // Gate three: the cover Tes requires.
    if product.cover.is_none() {
        return Err(ProjectionBlocked::CoverMissing);
    }

    // Gate four: every payload scanned clean before any byte leaves.
    for file in product.payload.iter() {
        if !matches!(file.scan, ScanOutcome::Clean { .. }) {
            return Err(ProjectionBlocked::ScanIncomplete { file: file.id });
        }
    }

    let declared = &registry(ctx.inventory).canonical;

    Ok(ListingProjection {
        inventory: ctx.inventory,
        title: capped(&product.title.0, &declared.title),
        body: capped(&product.body.body, &declared.description),
        body_format: product.body.format,
        price: product.price,
        taxonomy: included,
        grades,
        files: product.payload.iter().map(|file| file.id).collect(),
        natives,
        loss,
    })
}

/// The source's own paths for one axis, which is what a `NoTargetField` loss
/// names.
///
/// Only the two axes a product declares by path can produce one. The three it
/// declares by canonical id carry no source path at all, and no inventory
/// declares one of those absent -- pinned by this module's own test, because
/// the day one does, the loss it needs is the term's label and this is the
/// function that would have to grow a catalogue lookup to say it.
fn carried_in(product: &CanonicalProduct, axis: TermKind) -> Vec<VocabularyPath> {
    match axis {
        TermKind::Licence => rights_source(product).cloned().into_iter().collect(),
        TermKind::Phase => product.grades.raw.clone(),
        TermKind::Subject | TermKind::Topic | TermKind::ResourceType => Vec::new(),
    }
}

fn capped(text: &str, spec: &FieldSpec) -> String {
    spec.cap
        .map_or_else(|| text.to_owned(), |cap| truncate(text, cap))
}

#[cfg(test)]
mod tests {
    use super::{project_listing, ListingContext};
    use tam_domain::equivalence::{
        ElectionAnswer, ElectionRule, ElectionTrigger, ElectionTriggerKind, Loss, PricingBranch,
        SettledElection,
    };
    use tam_domain::registry::registry;
    use tam_domain::ListingProjection;
    use tam_domain::{
        CanonicalTerm, Decider, EdgeKind, ProjectionBlocked, ProjectionEdge, RightsDeclaration,
        TermKind, VocabularyId, VocabularyPath,
    };
    use tam_types::{
        CanonicalTermId, ContentHash, CopyFormat, Currency, FileId, FileKind, FileRole,
        InventoryId, ListingCopy, MappingId, Money, OrgId, PayloadSet, PriceIntent, ProductFile,
        ProductId, ScanOutcome, Timestamp, Title, Uuid,
    };

    const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
    const MAPPING: MappingId = MappingId(Uuid([0x31; 16]));
    const TERM: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
    const GRADE: CanonicalTermId = CanonicalTermId(Uuid([0x79; 16]));
    const RIGHT: CanonicalTermId = CanonicalTermId(Uuid([0x7B; 16]));
    const NOW: Timestamp = Timestamp(1_000);

    fn file(scan: ScanOutcome) -> ProductFile {
        ProductFile {
            id: FileId(Uuid([0x21; 16])),
            role: FileRole::Payload,
            kind: FileKind::Pdf,
            hash: ContentHash([0x51; 32]),
            byte_len: 4,
            scan,
        }
    }

    fn cover() -> ProductFile {
        ProductFile {
            id: FileId(Uuid([0x22; 16])),
            role: FileRole::Cover,
            kind: FileKind::Image,
            hash: ContentHash([0x52; 32]),
            byte_len: 4,
            scan: ScanOutcome::Clean { at: NOW },
        }
    }

    fn product(
        price: PriceIntent,
        with_cover: bool,
        scan: ScanOutcome,
    ) -> tam_domain::CanonicalProduct {
        tam_domain::CanonicalProduct {
            id: ProductId(Uuid([0x01; 16])),
            org: ORG,
            title: Title("A worksheet".to_owned()),
            body: ListingCopy {
                body: "Body text.".to_owned(),
                format: CopyFormat::Markdown,
            },
            payload: PayloadSet::new(file(scan), vec![]),
            cover: with_cover.then(cover),
            previews: vec![],
            subjects: vec![TERM],
            grades: tam_domain::GradeDeclaration {
                source: tam_domain::DeclarationSource::Imported {
                    vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Phase),
                },
                raw: vec![VocabularyPath {
                    vocabulary: VocabularyId(InventoryId::TesGb, TermKind::Phase),
                    segments: vec!["5-7".to_owned()],
                    native_id: Some("2".to_owned()),
                }],
                derived: None,
            },
            price,
            rights: tam_domain::RightsDeclaration::Unstated,
            native_residue: vec![],
        }
    }

    fn terms() -> Vec<CanonicalTerm> {
        vec![CanonicalTerm {
            id: TERM,
            kind: TermKind::Subject,
            parent: None,
            label: "Maths".to_owned(),
        }]
    }

    fn nz_edge() -> ProjectionEdge {
        ProjectionEdge {
            from: TERM,
            to: VocabularyPath {
                vocabulary: VocabularyId(InventoryId::TesNz, TermKind::Subject),
                segments: vec!["Maths".to_owned()],
                native_id: Some("7000001".to_owned()),
            },
            kind: EdgeKind::Exact,
            decided_by: Decider::Imported {
                source: "test".to_owned(),
            },
            decided_at: NOW,
        }
    }

    fn phase_edge(inventory: InventoryId, label: &str, native: &str) -> ProjectionEdge {
        ProjectionEdge {
            from: GRADE,
            to: VocabularyPath {
                vocabulary: VocabularyId(inventory, TermKind::Phase),
                segments: vec![label.to_owned()],
                native_id: Some(native.to_owned()),
            },
            kind: EdgeKind::Exact,
            decided_by: Decider::Imported {
                source: "test".to_owned(),
            },
            decided_at: NOW,
        }
    }

    /// The value the seller named, in the shape the answer endpoint stores it.
    /// `AnswerPath` defaults the target's own native id to absent, so an
    /// answer given as segments alone -- which is what a client that names the
    /// value rather than echoing the candidate's id sends -- round-trips
    /// carrying none, and the token is the relation's to supply.
    fn elected(token: &str) -> VocabularyPath {
        VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesNz, TermKind::Licence),
            segments: vec![token.to_owned()],
            native_id: None,
        }
    }

    /// The target's own licence vocabulary as the relation holds it, which is
    /// where an elected value's token is read back from.
    fn licence_edges() -> Vec<ProjectionEdge> {
        ["CC-BY", "CC-BY-SA", "TES-PAID"]
            .into_iter()
            .map(|token| ProjectionEdge {
                from: RIGHT,
                to: licence(InventoryId::TesNz, token),
                kind: EdgeKind::Exact,
                decided_by: Decider::Imported {
                    source: "test".to_owned(),
                },
                decided_at: NOW,
            })
            .collect()
    }

    /// The tenant's standing licence policy. Every fixture product below is
    /// `Unstated`, which Tes refuses -- so without a policy each of these
    /// tests would assert the licence election rather than what it is about.
    /// The seller answered once and the rule is what makes that durable.
    fn licence_policy() -> Vec<ElectionRule> {
        // One rule per pricing branch, because Tes accepts a different set of
        // licences on each side of the free/paid gate and a policy that
        // ignored the branch would be unwritable.
        [
            (PricingBranch::Free, "CC-BY-SA"),
            (PricingBranch::Paid, "TES-PAID"),
        ]
        .into_iter()
        .map(|(pricing, token)| ElectionRule {
            org: ORG,
            inventory: InventoryId::TesNz,
            axis: TermKind::Licence,
            trigger_kind: ElectionTriggerKind::Supply,
            trigger_key: Some(pricing.as_str().to_owned()),
            answer: ElectionAnswer::Value {
                path: elected(token),
            },
            decided_by: Decider::Imported {
                source: "test".to_owned(),
            },
            decided_at: NOW,
        })
        .collect()
    }

    fn ctx<'a>(
        terms: &'a [CanonicalTerm],
        edges: &'a [ProjectionEdge],
        rules: &'a [ElectionRule],
    ) -> ListingContext<'a> {
        ListingContext {
            org: ORG,
            mapping: MAPPING,
            inventory: InventoryId::TesNz,
            now: NOW,
            terms,
            edges,
            no_counterparts: &[],
            rules,
            settled: &[],
        }
    }

    #[test]
    fn a_free_scanned_covered_mapped_product_projects() {
        let catalogue = terms();
        let policy = licence_policy();
        let edges = [nz_edge()];
        let projection = project_listing(
            &product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW }),
            &ctx(&catalogue, &edges, &policy),
        )
        .expect("everything is in order, so it projects");
        assert_eq!(projection.title, "A worksheet", "copy is verbatim in M1");
        assert_eq!(
            projection.taxonomy[0].native_id.as_deref(),
            Some("7000001"),
            "the taxonomy landed in the target vocabulary"
        );
        assert!(
            projection.grades.is_empty(),
            "the relation holds no phase edge, so the GB band id is carried out as \
             unrecognised rather than re-labelled into the NZ vocabulary, where 2 means \
             Reception and not the 5-7 age band"
        );
    }

    /// D2. Tes declares the licence required and the source stated none, so
    /// the projection asks rather than publishing under a grant nobody chose.
    /// The question is about this product against this target, which is what
    /// makes it an election rather than a vocabulary gap.
    #[test]
    fn an_unstated_licence_into_a_target_that_requires_one_asks_rather_than_defaulting() {
        let catalogue = terms();
        let edges = [nz_edge()];
        let blocked = project_listing(
            &product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW }),
            &ctx(&catalogue, &edges, &[]),
        )
        .expect_err("a required axis the source never carried blocks");
        let ProjectionBlocked::Blocked {
            gaps, elections, ..
        } = blocked
        else {
            panic!("the licence is an election, not another gate: {blocked:?}");
        };
        assert!(
            gaps.is_empty(),
            "nothing here is a question about a vocabulary pair"
        );
        assert_eq!(elections.len(), 1, "one product, one question");
        assert_eq!(elections[0].axis, TermKind::Licence);
        assert_eq!(
            elections[0].trigger,
            ElectionTrigger::Supply {
                pricing: PricingBranch::Free
            },
            "the branch travels because Tes gates the write on it"
        );
    }

    /// The elected licence as the seam reads it: the value the seller named,
    /// under the target's own token. `None` is what the Tes adapter refuses
    /// as a projection carrying no licence at all.
    fn elected_licence(projection: &ListingProjection) -> Option<String> {
        projection
            .natives
            .iter()
            .find(|(axis, _)| *axis == TermKind::Licence)
            .expect("the elected licence travels labelled by the axis it answers")
            .1
            .native_id
            .clone()
    }

    /// The founder's do-not-re-ask requirement, as an assertion: two products
    /// under one standing rule raise nothing at all, and both carry the value
    /// the seller elected. A rule is consulted before anything is enqueued,
    /// so the common path writes no queue row.
    ///
    /// The rule is stored as the answer endpoint stores it, naming the value
    /// and not the token, so this also asserts the second half: an answer that
    /// resolves an election but reaches the seam without the target's own id
    /// is refused there as a listing with no licence, which is the same
    /// question standing under a different name.
    #[test]
    fn a_standing_rule_answers_the_licence_for_every_later_product() {
        let catalogue = terms();
        let mut edges = vec![nz_edge()];
        edges.extend(licence_edges());
        let policy = licence_policy();
        for price in [PriceIntent::Free, PriceIntent::Free] {
            let projection = project_listing(
                &product(price, true, ScanOutcome::Clean { at: NOW }),
                &ctx(&catalogue, &edges, &policy),
            )
            .expect("the standing rule answers the only question this product raised");
            assert_eq!(
                elected_licence(&projection).as_deref(),
                Some("CC-BY-SA"),
                "and it travels as the target's own token, which is what the wire takes"
            );
        }
    }

    /// D1 and D5 of the cross-platform battery, which a TPT-sourced product
    /// syncing to Tes runs every time: the licence is unstated, Tes requires
    /// one, the seller answers the raised question for this product, and the
    /// next projection of the same product must carry what they said.
    ///
    /// The answer settles the question -- that much a standing rule would do
    /// too -- but a settled answer is read back from the queue row, where the
    /// endpoint stored the value the seller named and no token. Resolving the
    /// election while carrying no token leaves the seam refusing the listing
    /// for want of a licence, so the item loops on an answered question.
    #[test]
    fn an_answered_licence_carries_the_targets_token_on_the_next_projection() {
        let catalogue = terms();
        let mut edges = vec![nz_edge()];
        edges.extend(licence_edges());
        let subject = product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW });
        let settled = [SettledElection {
            product: subject.id,
            inventory: InventoryId::TesNz,
            axis: TermKind::Licence,
            trigger_kind: ElectionTriggerKind::Supply,
            trigger_key: Some(PricingBranch::Free.as_str().to_owned()),
            chosen: vec![elected("CC-BY")],
        }];
        let mut context = ctx(&catalogue, &edges, &[]);
        context.settled = &settled;
        let projection = project_listing(&subject, &context)
            .expect("the seller answered this product's own question, so nothing stands");
        assert_eq!(
            elected_licence(&projection).as_deref(),
            Some("CC-BY"),
            "the answer names the value and the relation names its token, so the seam \
             receives a licence rather than refusing the listing for want of one"
        );
    }

    #[test]
    fn a_grade_reaches_the_target_under_the_targets_own_id_rather_than_the_sources() {
        let policy = licence_policy();
        let catalogue = [
            terms().remove(0),
            CanonicalTerm {
                id: GRADE,
                kind: TermKind::Phase,
                parent: None,
                label: "Kindergarten".to_owned(),
            },
        ];
        let edges = [
            nz_edge(),
            phase_edge(InventoryId::TesUs, "Kindergarten", "17"),
            phase_edge(InventoryId::TesNz, "Kindergarten", "17"),
        ];
        let mut source = product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW });
        source.grades.raw = vec![VocabularyPath {
            vocabulary: VocabularyId(InventoryId::TesUs, TermKind::Phase),
            segments: vec!["Kindergarten".to_owned()],
            native_id: Some("17".to_owned()),
        }];

        let projection = project_listing(&source, &ctx(&catalogue, &edges, &policy))
            .expect("the grade is mapped");
        assert_eq!(
            projection.grades[0].vocabulary,
            VocabularyId(InventoryId::TesNz, TermKind::Phase),
            "the grade lands in the target vocabulary"
        );
        assert_eq!(
            projection.grades[0].native_id.as_deref(),
            Some("17"),
            "and it gets there by ingesting into a term and projecting out again, which is \
             what makes the id the target's own rather than the source's"
        );
    }

    /// The registry's one measured cardinality cap, spent where a listing
    /// actually crosses. TPT's create form takes four grades, so a fifth is a
    /// question about which four rather than a set quietly cut to length, and
    /// `project_listing` is where a seller meets that question.
    #[test]
    fn a_grade_set_over_tpts_declared_cap_blocks_on_one_election_and_publishes_nothing() {
        const SLUGS: [&str; 5] = [
            "1st-grade",
            "2nd-grade",
            "3rd-grade",
            "4th-grade",
            "5th-grade",
        ];
        let phase = VocabularyId(InventoryId::Tpt, TermKind::Phase);
        let term = |index: u8| CanonicalTermId(Uuid([0xC0 + index; 16]));
        let path = |index: usize| VocabularyPath {
            vocabulary: phase,
            segments: vec![SLUGS[index].to_owned()],
            native_id: Some(SLUGS[index].to_owned()),
        };
        let catalogue: Vec<CanonicalTerm> = (0..SLUGS.len())
            .map(|index| CanonicalTerm {
                id: term(u8::try_from(index).expect("five fits")),
                kind: TermKind::Phase,
                parent: None,
                label: SLUGS[index].to_owned(),
            })
            .collect();
        let edges: Vec<ProjectionEdge> = (0..SLUGS.len())
            .map(|index| ProjectionEdge {
                from: term(u8::try_from(index).expect("five fits")),
                to: path(index),
                kind: EdgeKind::Exact,
                decided_by: Decider::Imported {
                    source: "test".to_owned(),
                },
                decided_at: NOW,
            })
            .collect();

        let mut source = product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW });
        source.subjects = vec![];
        source.grades.source = tam_domain::DeclarationSource::Imported { vocabulary: phase };
        source.grades.raw = (0..SLUGS.len()).map(path).collect();

        let blocked = project_listing(
            &source,
            &ListingContext {
                inventory: InventoryId::Tpt,
                ..ctx(&catalogue, &edges, &[])
            },
        )
        .expect_err("five grades against a cap of four is a question, not a projection");
        let ProjectionBlocked::Blocked {
            gaps, elections, ..
        } = blocked
        else {
            panic!("the cap is an election, not another gate: {blocked:?}");
        };
        assert!(
            gaps.is_empty(),
            "every grade maps; only the count is at issue"
        );
        assert_eq!(elections.len(), 1, "one axis over its cap, one question");
        assert_eq!(elections[0].axis, TermKind::Phase);
        let ElectionTrigger::OverCap { cap, from } = &elections[0].trigger else {
            panic!(
                "a set over the cap is an OverCap trigger: {:?}",
                elections[0]
            );
        };
        assert_eq!(
            *cap, 4,
            "the registry's declared cap reaches the seller as the number the form states"
        );
        assert_eq!(
            from.len(),
            5,
            "the whole resolved set goes into the question, so no four-grade subset exists \
             anywhere for a caller to publish by accident"
        );
    }

    #[test]
    fn an_unmapped_term_blocks_as_a_vocabulary_gap() {
        let catalogue = terms();
        let policy = licence_policy();
        let blocked = project_listing(
            &product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW }),
            &ctx(&catalogue, &[], &policy),
        );
        let Err(ProjectionBlocked::Blocked {
            gaps, elections, ..
        }) = blocked
        else {
            panic!("no edge means the equivalence gate blocks, got {blocked:?}");
        };
        assert_eq!(
            (gaps.len(), gaps[0].term, gaps[0].target),
            (1, TERM, VocabularyId(InventoryId::TesNz, TermKind::Subject)),
            "one gap names the term and the vocabulary pair it is a question about"
        );
        assert!(
            elections.is_empty(),
            "a missing equivalence is a question about a vocabulary, not about this product"
        );
    }

    #[test]
    fn a_priced_nz_listing_projects_now_that_the_currency_is_fixed_and_a_free_one_always_did() {
        let catalogue = terms();
        let policy = licence_policy();
        let edges = [nz_edge()];
        let paid = PriceIntent::Paid(Money::new(300, Currency::Gbp).expect("a price"));
        assert!(
            project_listing(
                &product(paid, true, ScanOutcome::Clean { at: NOW }),
                &ctx(&catalogue, &edges, &policy),
            )
            .is_ok(),
            "the NZ currency is fixed to GBP, so a priced listing clears the currency gate"
        );
        assert!(
            project_listing(
                &product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW }),
                &ctx(&catalogue, &edges, &policy),
            )
            .is_ok(),
            "a free listing needs no currency and passes the same gate"
        );
    }

    #[test]
    fn a_priced_listing_into_a_seller_scoped_inventory_still_blocks() {
        // The gate's block path survives the NZ measurement: Etsy is
        // SellerScoped and its currency is unverified until its connector,
        // so a priced listing into it must still refuse rather than guess.
        let catalogue = terms();
        let etsy_edge = ProjectionEdge {
            from: TERM,
            to: VocabularyPath {
                vocabulary: VocabularyId(InventoryId::Etsy, TermKind::Subject),
                segments: vec!["Maths".to_owned()],
                native_id: Some("e-1".to_owned()),
            },
            kind: EdgeKind::Exact,
            decided_by: Decider::Imported {
                source: "test".to_owned(),
            },
            decided_at: NOW,
        };
        let etsy_ctx = ListingContext {
            org: ORG,
            mapping: MAPPING,
            inventory: InventoryId::Etsy,
            now: NOW,
            terms: &catalogue,
            edges: std::slice::from_ref(&etsy_edge),
            no_counterparts: &[],
            rules: &[],
            settled: &[],
        };
        let paid = PriceIntent::Paid(Money::new(300, Currency::Gbp).expect("a price"));
        let blocked = project_listing(
            &product(paid, true, ScanOutcome::Clean { at: NOW }),
            &etsy_ctx,
        );
        assert!(
            matches!(
                blocked,
                Err(ProjectionBlocked::CurrencyUnknown {
                    inventory: InventoryId::Etsy
                })
            ),
            "a seller-scoped inventory's currency is unverified, so a priced listing blocks: {blocked:?}"
        );
    }

    #[test]
    fn a_tes_projection_is_verbatim_because_no_tes_cap_is_declared() {
        let catalogue = terms();
        let policy = licence_policy();
        let edges = [nz_edge()];
        let long = "A worksheet ".repeat(400);
        let mut source = product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW });
        source.title = Title(long.clone());
        source.body = ListingCopy {
            body: long.clone(),
            format: CopyFormat::Markdown,
        };
        let projection = project_listing(&source, &ctx(&catalogue, &edges, &policy))
            .expect("everything is in order");
        assert_eq!(
            (projection.title.as_str(), projection.body.as_str()),
            (long.as_str(), long.as_str()),
            "the registry declares no Tes cap, so projection copies every byte"
        );
    }

    #[test]
    fn a_declared_cap_truncates_at_projection() {
        // Etsy is the only inventory with a cap on file, and its currency is
        // seller-scoped, so a free listing is the one that reaches the cap
        // rather than blocking at the currency gate first.
        let catalogue = terms();
        let etsy_edge = ProjectionEdge {
            from: TERM,
            to: VocabularyPath {
                vocabulary: VocabularyId(InventoryId::Etsy, TermKind::Subject),
                segments: vec!["Maths".to_owned()],
                native_id: Some("e-1".to_owned()),
            },
            kind: EdgeKind::Exact,
            decided_by: Decider::Imported {
                source: "test".to_owned(),
            },
            decided_at: NOW,
        };
        let etsy_ctx = ListingContext {
            org: ORG,
            mapping: MAPPING,
            inventory: InventoryId::Etsy,
            now: NOW,
            terms: &catalogue,
            edges: std::slice::from_ref(&etsy_edge),
            no_counterparts: &[],
            rules: &[],
            settled: &[],
        };
        let long: String = std::iter::repeat_n('é', 200).collect();
        let mut source = product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW });
        source.title = Title(long.clone());
        let projection = project_listing(&source, &etsy_ctx).expect("a free listing projects");
        assert_eq!(
            projection.title.chars().count(),
            140,
            "Etsy's documented cap counts codepoints, not the 400 bytes these occupy"
        );
        assert!(
            long.starts_with(&projection.title),
            "truncation keeps a prefix and never rewrites it"
        );
        assert_eq!(
            projection.body, "Body text.",
            "Etsy declares no description cap, so the body is untouched"
        );
    }

    #[test]
    fn a_missing_cover_blocks() {
        let catalogue = terms();
        let policy = licence_policy();
        let edges = [nz_edge()];
        assert!(
            matches!(
                project_listing(
                    &product(PriceIntent::Free, false, ScanOutcome::Clean { at: NOW }),
                    &ctx(&catalogue, &edges, &policy),
                ),
                Err(ProjectionBlocked::CoverMissing)
            ),
            "Tes requires the cover"
        );
    }

    #[test]
    fn an_unscanned_payload_blocks_naming_the_file() {
        let catalogue = terms();
        let policy = licence_policy();
        let edges = [nz_edge()];
        let blocked = project_listing(
            &product(PriceIntent::Free, true, ScanOutcome::Pending),
            &ctx(&catalogue, &edges, &policy),
        );
        assert!(
            matches!(
                blocked,
                Err(ProjectionBlocked::ScanIncomplete { file }) if file == FileId(Uuid([0x21; 16]))
            ),
            "no byte leaves before its scan settles clean: {blocked:?}"
        );
    }

    #[test]
    fn the_gates_answer_in_enum_order() {
        // Everything wrong at once: no edge, priced into unmeasured, no
        // cover, unscanned. The answer is the taxonomy gate, deterministically.
        let catalogue = terms();
        let policy = licence_policy();
        let paid = PriceIntent::Paid(Money::new(300, Currency::Gbp).expect("a price"));
        let blocked = project_listing(
            &product(paid, false, ScanOutcome::Pending),
            &ctx(&catalogue, &[], &policy),
        );
        assert!(
            matches!(blocked, Err(ProjectionBlocked::Blocked { .. })),
            "several gates would fire; the enum's first names the block: {blocked:?}"
        );
    }

    fn licence(inventory: InventoryId, token: &str) -> VocabularyPath {
        VocabularyPath {
            vocabulary: VocabularyId(inventory, TermKind::Licence),
            segments: vec![token.to_owned()],
            native_id: Some(token.to_owned()),
        }
    }

    /// 5.6. TPT holds no licence field anywhere on its wire, so a licence
    /// projected into it can never be a question — no edge could ever answer
    /// one — and must not be a silent drop either. It is a disclosed loss,
    /// which is the whole distinction between the absence TPT was measured to
    /// have and the unmeasured one Etsy has, which still blocks.
    #[test]
    fn a_licence_into_a_target_measured_to_have_no_field_is_a_disclosed_loss() {
        let catalogue = terms();
        let tpt_edge = ProjectionEdge {
            from: TERM,
            to: VocabularyPath {
                vocabulary: VocabularyId(InventoryId::Tpt, TermKind::Subject),
                segments: vec!["math".to_owned()],
                native_id: Some("math".to_owned()),
            },
            kind: EdgeKind::Exact,
            decided_by: Decider::Imported {
                source: "test".to_owned(),
            },
            decided_at: NOW,
        };
        let mut source = product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW });
        source.grades.raw = vec![];
        source.rights = RightsDeclaration::Declared {
            source: licence(InventoryId::TesGb, "CC-BY"),
        };
        let projection = project_listing(
            &source,
            &ListingContext {
                org: ORG,
                mapping: MAPPING,
                inventory: InventoryId::Tpt,
                now: NOW,
                terms: &catalogue,
                edges: core::slice::from_ref(&tpt_edge),
                no_counterparts: &[],
                rules: &[],
                settled: &[],
            },
        )
        .expect("a measured absence discloses; it never blocks");
        assert_eq!(
            projection.loss,
            vec![Loss::NoTargetField {
                axis: TermKind::Licence,
                value: licence(InventoryId::TesGb, "CC-BY"),
            }],
            "the seller's Creative Commons grant does not reach TPT, and the record of \
             that is what makes the drop disclosed rather than silent"
        );
        assert!(
            projection
                .natives
                .iter()
                .all(|(axis, _)| *axis != TermKind::Licence),
            "nothing was carried onto a field the target does not have"
        );
    }

    /// The counterpart, and the reason the two are one commit: an axis a
    /// target merely declares no binding for is unmeasured, so its value is a
    /// question rather than a loss.
    #[test]
    fn an_unmeasured_absence_is_not_a_loss() {
        let catalogue = terms();
        let mut source = product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW });
        source.grades.raw = vec![];
        source.rights = RightsDeclaration::Declared {
            source: licence(InventoryId::TesGb, "CC-BY"),
        };
        let projection = project_listing(
            &source,
            &ListingContext {
                org: ORG,
                mapping: MAPPING,
                inventory: InventoryId::Etsy,
                now: NOW,
                terms: &catalogue,
                edges: &[],
                no_counterparts: &[],
                rules: &[],
                settled: &[],
            },
        )
        .expect("Etsy binds no equivalence axis at all, so nothing gates on one");
        assert!(
            projection.loss.is_empty(),
            "Etsy records no measured absence, so claiming the licence was dropped would \
             assert something the capture never established"
        );
    }

    /// The fence behind `carried_in`: only two axes arrive as the source's own
    /// paths, so a measured absence declared on one of the other three would
    /// disclose nothing at all.
    #[test]
    fn every_measured_absence_is_an_axis_the_projection_can_name_a_value_for() {
        for inventory in InventoryId::ALL {
            for absent in registry(inventory).absent_axes {
                assert!(
                    matches!(absent.0, TermKind::Licence | TermKind::Phase),
                    "{inventory:?} records {:?} absent, and a product declares that axis \
                     by canonical id rather than by a source path, so the loss would name \
                     no value",
                    absent.0
                );
            }
        }
    }
}
