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

use tam_domain::equivalence::{Election, Loss, PricingBranch, VocabularyGap};
use tam_domain::registry::{registry, truncate, AxisBinding, FieldSpec};
use tam_domain::{
    CanonicalProduct, CanonicalTerm, ListingProjection, ProjectionBlocked, ProjectionEdge,
    TermKind, TermProjection, VocabularyId, VocabularyPath,
};
use tam_types::{
    CanonicalTermId, CurrencyRule, InventoryId, MappingId, OrgId, PriceIntent, ScanOutcome,
    Timestamp,
};

use crate::project::{ingest_grades, project_axis, AxisRequest};

/// The axes this projection routes, and the reason the list is shorter than
/// the registry's.
///
/// `Licence` is bound by every Tes inventory and is deliberately absent: its
/// queue is the seller's election surface, and routing it before that surface
/// exists would block every product whose rights are unstated on a gate with
/// nowhere to record the question. `ResourceType` waits for the same reason
/// its own crosswalk does.
const ROUTED_AXES: [TermKind; 3] = [TermKind::Subject, TermKind::Topic, TermKind::Phase];

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
    for path in &product.grades.raw {
        if !wanted.contains(&path.vocabulary) {
            wanted.push(path.vocabulary);
        }
    }
    wanted
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
            TermKind::Subject | TermKind::Topic | TermKind::ResourceType | TermKind::Licence => {
                (&of_kind, &[])
            }
        };
        if terms.is_empty() {
            continue;
        }
        let outcome = project_axis(
            AxisRequest {
                product: product.id,
                inventory: ctx.inventory,
                binding,
                terms,
                sources,
                pricing,
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
            unrecognised: ingested.unrecognised,
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

fn capped(text: &str, spec: &FieldSpec) -> String {
    spec.cap
        .map_or_else(|| text.to_owned(), |cap| truncate(text, cap))
}

#[cfg(test)]
mod tests {
    use super::{project_listing, ListingContext};
    use tam_domain::{
        CanonicalTerm, Decider, EdgeKind, ProjectionBlocked, ProjectionEdge, TermKind,
        VocabularyId, VocabularyPath,
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

    fn ctx<'a>(terms: &'a [CanonicalTerm], edges: &'a [ProjectionEdge]) -> ListingContext<'a> {
        ListingContext {
            org: ORG,
            mapping: MAPPING,
            inventory: InventoryId::TesNz,
            now: NOW,
            terms,
            edges,
            no_counterparts: &[],
        }
    }

    #[test]
    fn a_free_scanned_covered_mapped_product_projects() {
        let catalogue = terms();
        let edges = [nz_edge()];
        let projection = project_listing(
            &product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW }),
            &ctx(&catalogue, &edges),
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

    #[test]
    fn a_grade_reaches_the_target_under_the_targets_own_id_rather_than_the_sources() {
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

        let projection =
            project_listing(&source, &ctx(&catalogue, &edges)).expect("the grade is mapped");
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

    #[test]
    fn an_unmapped_term_blocks_as_a_vocabulary_gap() {
        let catalogue = terms();
        let blocked = project_listing(
            &product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW }),
            &ctx(&catalogue, &[]),
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
        let edges = [nz_edge()];
        let paid = PriceIntent::Paid(Money::new(300, Currency::Gbp).expect("a price"));
        assert!(
            project_listing(
                &product(paid, true, ScanOutcome::Clean { at: NOW }),
                &ctx(&catalogue, &edges),
            )
            .is_ok(),
            "the NZ currency is fixed to GBP, so a priced listing clears the currency gate"
        );
        assert!(
            project_listing(
                &product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW }),
                &ctx(&catalogue, &edges),
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
        let edges = [nz_edge()];
        let long = "A worksheet ".repeat(400);
        let mut source = product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW });
        source.title = Title(long.clone());
        source.body = ListingCopy {
            body: long.clone(),
            format: CopyFormat::Markdown,
        };
        let projection =
            project_listing(&source, &ctx(&catalogue, &edges)).expect("everything is in order");
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
        let edges = [nz_edge()];
        assert!(
            matches!(
                project_listing(
                    &product(PriceIntent::Free, false, ScanOutcome::Clean { at: NOW }),
                    &ctx(&catalogue, &edges),
                ),
                Err(ProjectionBlocked::CoverMissing)
            ),
            "Tes requires the cover"
        );
    }

    #[test]
    fn an_unscanned_payload_blocks_naming_the_file() {
        let catalogue = terms();
        let edges = [nz_edge()];
        let blocked = project_listing(
            &product(PriceIntent::Free, true, ScanOutcome::Pending),
            &ctx(&catalogue, &edges),
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
        let paid = PriceIntent::Paid(Money::new(300, Currency::Gbp).expect("a price"));
        let blocked = project_listing(
            &product(paid, false, ScanOutcome::Pending),
            &ctx(&catalogue, &[]),
        );
        assert!(
            matches!(blocked, Err(ProjectionBlocked::Blocked { .. })),
            "several gates would fire; the enum's first names the block: {blocked:?}"
        );
    }
}
