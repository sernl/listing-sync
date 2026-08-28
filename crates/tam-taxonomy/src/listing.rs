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

use tam_domain::registry::{registry, truncate, FieldSpec};
use tam_domain::{
    CanonicalProduct, CanonicalTerm, ListingProjection, ProjectionBlocked, ProjectionEdge,
    ReconciliationItem, ReconciliationState, TermKind, VocabularyId, VocabularyPath,
};
use tam_types::{
    CanonicalTermId, CurrencyRule, InventoryId, MappingId, OrgId, PriceIntent, ScanOutcome,
    Timestamp,
};

use crate::project::project_terms;

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

    // Gate one: taxonomy. Terms are projected per kind because a vocabulary
    // is per (inventory, kind); blocked terms become the queue items the
    // caller raises, deduplicated here on (term, kind) so one gap is one item.
    let mut included = Vec::new();
    let mut loss = Vec::new();
    let mut items: Vec<ReconciliationItem> = Vec::new();
    for kind in [TermKind::Subject, TermKind::Topic] {
        let of_kind: Vec<CanonicalTermId> = product
            .subjects
            .iter()
            .copied()
            .filter(|term| kinds.get(term) == Some(&kind))
            .collect();
        if of_kind.is_empty() {
            continue;
        }
        let vocabulary = VocabularyId(ctx.inventory, kind);
        let outcome = project_terms(&of_kind, vocabulary, ctx.edges, ctx.no_counterparts);
        included.extend(outcome.included);
        loss.extend(outcome.loss);
        for blocked in outcome.blocked {
            items.push(ReconciliationItem {
                org: ctx.org,
                term: blocked.term,
                target: vocabulary,
                raised_by: ctx.mapping,
                raised_at: ctx.now,
                state: ReconciliationState::Open,
            });
        }
    }
    // A term the catalogue does not classify cannot be projected and cannot
    // be silently dropped: it blocks as its own queue item under the subject
    // vocabulary, which is the fail-closed reading of an impossible input.
    for term in &product.subjects {
        if !kinds.contains_key(term) {
            items.push(ReconciliationItem {
                org: ctx.org,
                term: *term,
                target: VocabularyId(ctx.inventory, TermKind::Subject),
                raised_by: ctx.mapping,
                raised_at: ctx.now,
                state: ReconciliationState::Open,
            });
        }
    }
    if !items.is_empty() {
        return Err(ProjectionBlocked::Taxonomy { items });
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

    let grades: Vec<VocabularyPath> = product
        .grades
        .raw
        .iter()
        .map(|path| {
            let VocabularyId(_, kind) = path.vocabulary;
            VocabularyPath {
                vocabulary: VocabularyId(ctx.inventory, kind),
                segments: path.segments.clone(),
                native_id: path.native_id.clone(),
            }
        })
        .collect();

    let declared = &registry(ctx.inventory).canonical;

    Ok(ListingProjection {
        inventory: ctx.inventory,
        title: capped(&product.title.0, &declared.title),
        body: capped(&product.body.body, &declared.description),
        price: product.price,
        taxonomy: included,
        grades,
        files: product.payload.iter().map(|file| file.id).collect(),
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
        CanonicalTermId, ContentHash, Currency, FileId, FileKind, FileRole, InventoryId,
        ListingCopy, MappingId, Money, OrgId, PayloadSet, PriceIntent, ProductFile, ProductId,
        ScanOutcome, Timestamp, Title, Uuid,
    };

    const ORG: OrgId = OrgId(Uuid([0xAA; 16]));
    const MAPPING: MappingId = MappingId(Uuid([0x31; 16]));
    const TERM: CanonicalTermId = CanonicalTermId(Uuid([0x77; 16]));
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
        assert_eq!(
            projection.grades[0].native_id.as_deref(),
            Some("2"),
            "grade native ids pass through: the age vocabulary is account-scoped"
        );
        assert_eq!(
            projection.grades[0].vocabulary,
            VocabularyId(InventoryId::TesNz, TermKind::Phase),
            "the grade path is re-labelled to the target inventory"
        );
    }

    #[test]
    fn an_unmapped_term_blocks_as_a_taxonomy_item() {
        let catalogue = terms();
        let blocked = project_listing(
            &product(PriceIntent::Free, true, ScanOutcome::Clean { at: NOW }),
            &ctx(&catalogue, &[]),
        );
        let Err(ProjectionBlocked::Taxonomy { items }) = blocked else {
            panic!("no edge means the taxonomy gate blocks, got {blocked:?}");
        };
        assert_eq!(
            (items.len(), items[0].term, items[0].raised_by),
            (1, TERM, MAPPING),
            "one gap raises one item naming the term and the mapping"
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
        source.body = ListingCopy { body: long.clone() };
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
            matches!(blocked, Err(ProjectionBlocked::Taxonomy { .. })),
            "several gates would fire; the enum's first names the block: {blocked:?}"
        );
    }
}
