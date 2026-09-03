//! The derivation from the authored product to [`CanonicalProduct`].
//!
//! [`super::TptBaseProduct`] is the source of truth for what a product is, and
//! `CanonicalProduct` is produced from it here rather than being a second
//! definition beside it, so the two cannot drift into disagreeing. The storage
//! side has not folded — `product` keeps its own table and migration 0040 adds
//! a sidecar — and folding it is a scheduled step after the engine driver
//! split rather than a permanent shape.

use tam_types::{
    CanonicalTermId, ImportedTerm, InventoryId, OrgId, PayloadSet, PriceIntent, ProductFile,
    ProductId, TermKind, Title,
};

use crate::product::fields::{CategoryGroup, PriceGroup};
use crate::product::TptBaseProduct;
use crate::{
    AgeInterval, CanonicalProduct, DeclarationSource, GradeDeclaration, RightsDeclaration,
    VocabularyId, VocabularyPath,
};

// ----------------------------------------------------- the derivation

/// What a create form cannot state about itself, supplied by the caller that
/// has already resolved it.
///
/// Three kinds of fact live here and each is outside the form by
/// construction. Row identity and the tenant, which no seller types. The file
/// rows the upload produced, because the form holds content hashes and a
/// [`ProductFile`] carries the row id, the kind, the byte length and the scan
/// verdict the pipeline recorded. And the two values that need a relation
/// this crate cannot reach: the canonical subject terms behind the TPT
/// subject-area slugs, and the age interval `tam_taxonomy::derive_interval`
/// computes from the grade paths.
///
/// It exists so [`TptBaseProduct::into_canonical`] can be total. A derivation
/// that invented any of these would be a second definition of a product
/// wearing the first one's name, which is the thing the derivation exists to
/// prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductIdentity {
    pub id: ProductId,
    pub org: OrgId,
    pub payload: PayloadSet,
    pub cover: Option<ProductFile>,
    pub previews: Vec<ProductFile>,
    /// The canonical terms the subject-area slugs resolve to. Empty where
    /// nothing has resolved them yet, which is honest rather than a claim:
    /// the crosswalk lives in `tam-taxonomy` and the projection raises what
    /// it cannot place.
    pub subjects: Vec<CanonicalTermId>,
    /// The interval derived from the grade paths, where a caller that can
    /// reach the derivation supplied one. `None` is unmeasured, not zero.
    pub derived_grades: Option<AgeInterval>,
    /// The rights grant, where a platform with a licence field carries one.
    /// TPT declares the licence axis measured-absent, so a TPT-base product
    /// states none of its own and this is `Unstated` unless a Tes-bound
    /// caller supplies it.
    pub rights: RightsDeclaration,
}

impl TptBaseProduct {
    /// The product as the rest of the domain reads it.
    ///
    /// Total, and the only definition of the mapping: `CanonicalProduct` is
    /// derived here rather than authored beside this type, so the two cannot
    /// drift into disagreeing about what a product is. The storage side has
    /// not folded yet — `product` keeps its own table and migration 0040 adds
    /// a sidecar — and folding it is a scheduled step after the engine driver
    /// split rather than a permanent shape.
    ///
    /// Three groups have no `CanonicalProduct` field of their own and reach
    /// `native_residue` verbatim: the tag facets, the format facets and the
    /// seller's own custom categories. Each carries `kind: None`, because
    /// residue is for values in axes this model does not type and claiming an
    /// axis for one would be inventing a reading the registry does not hold.
    /// A later upgrade moves them out of residue into a typed axis with no
    /// data migration, which is what residue is for.
    ///
    /// The rest of the TPT base — the thumbnail decision, the additional
    /// licence and bundle discount, the tax code, the standards, the three
    /// details, the attestation and the localisation flag — has no
    /// `CanonicalProduct` field and no residue shape either, because residue
    /// is a list of vocabulary terms and none of those is one. They live on
    /// this type and in the sidecar, which migrations 0040 and 0049 declare.
    #[must_use]
    pub fn into_canonical(self, identity: ProductIdentity) -> CanonicalProduct {
        let raw: Vec<VocabularyPath> = self
            .categories
            .grades
            .iter()
            .map(|slug| tpt_path(TermKind::Phase, slug.as_str()))
            .collect();
        CanonicalProduct {
            id: identity.id,
            org: identity.org,
            title: Title(self.name.as_str().to_owned()),
            body: self.description,
            payload: identity.payload,
            cover: identity.cover,
            previews: identity.previews,
            subjects: identity.subjects,
            grades: GradeDeclaration {
                source: DeclarationSource::Seller,
                derived: identity.derived_grades,
                raw,
            },
            price: match self.price {
                PriceGroup::Free => PriceIntent::Free,
                PriceGroup::Paid(paid) => PriceIntent::Paid(paid.price()),
            },
            rights: identity.rights,
            native_residue: residue(&self.categories),
        }
    }
}

/// One TPT facet slug as a vocabulary path. The slug is both the segment and
/// the native id, because `data[TaxonomyTags][]` posts the slug itself.
fn tpt_path(kind: TermKind, slug: &str) -> VocabularyPath {
    VocabularyPath {
        vocabulary: VocabularyId(InventoryId::Tpt, kind),
        segments: vec![slug.to_owned()],
        native_id: Some(slug.to_owned()),
    }
}

/// The category groups no canonical field holds, kept verbatim so a round trip
/// back to TPT loses nothing and a projection elsewhere can name what it
/// dropped.
fn residue(categories: &CategoryGroup) -> Vec<ImportedTerm> {
    let facets = categories
        .tags
        .iter()
        .chain(categories.formats.iter())
        .map(|slug| slug.as_str().to_owned());
    let shelves = categories.custom_categories.iter().cloned();
    facets
        .chain(shelves)
        .map(|value| ImportedTerm {
            inventory: InventoryId::Tpt,
            kind: None,
            segments: vec![value.clone()],
            native_id: Some(value),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::product::fields::{PaidPrice, PriceGroup};
    use crate::product::fixtures::{identity, product, slugs};
    use crate::product::vocabularies::TaxCode;
    use crate::{CanonicalProduct, DeclarationSource, RightsDeclaration};
    use tam_types::{Currency, Money, PriceIntent, Title};

    /// The condition that keeps the domain to one definition of a product.
    ///
    /// The destructuring is the assertion: a field added to
    /// `CanonicalProduct` stops this test compiling until the derivation
    /// carries it, so the two cannot drift into disagreeing about what a
    /// product is. Every field is then checked against the thing on the
    /// TPT-base model or the identity that produced it.
    #[test]
    fn every_canonical_product_field_is_reachable_from_the_tpt_base_model() {
        let mut base = product();
        base.categories.tags = slugs(&["centers", "autumn"]);
        base.categories.formats = slugs(&["easel"]);
        base.categories.custom_categories = vec!["Autumn unit".to_owned()];
        let name = base.name.as_str().to_owned();
        let body = base.description.clone();
        let given = identity();

        let CanonicalProduct {
            id,
            org,
            title,
            body: derived_body,
            payload,
            cover,
            previews,
            subjects,
            grades,
            price,
            rights,
            native_residue,
        } = base.into_canonical(given.clone());

        assert_eq!((id, org), (given.id, given.org), "identity is the caller's");
        assert_eq!(title, Title(name), "the title is the form's own name");
        assert_eq!(
            derived_body, body,
            "the body crosses with its declared format"
        );
        assert_eq!(payload, given.payload, "the payload rows are the upload's");
        assert_eq!((cover, previews), (given.cover, given.previews));
        assert_eq!(
            subjects, given.subjects,
            "resolved by the crosswalk, not here"
        );
        assert_eq!(
            rights,
            RightsDeclaration::Unstated,
            "TPT holds no licence field"
        );
        assert_eq!(
            price,
            PriceIntent::Free,
            "the free branch of the price group is the free price intent"
        );
        assert_eq!(
            (grades.source, grades.derived),
            (DeclarationSource::Seller, None),
            "a grade set the seller ticked, whose interval only tam-taxonomy can derive"
        );
        assert_eq!(
            grades
                .raw
                .iter()
                .map(|path| path.native_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("3rd-grade")],
            "each chosen slug becomes one verbatim TPT path"
        );
        assert_eq!(
            native_residue
                .iter()
                .map(|term| (term.native_id.as_deref(), term.kind))
                .collect::<Vec<_>>(),
            vec![
                (Some("centers"), None),
                (Some("autumn"), None),
                (Some("easel"), None),
                (Some("Autumn unit"), None),
            ],
            "tags, formats and the seller's own shelves have no canonical field, so they \
             survive verbatim in residue with no axis claimed for them"
        );
    }

    /// The priced branch, so both arms of the price mapping are exercised.
    #[test]
    fn a_priced_tpt_base_product_derives_the_paid_price_intent() {
        let mut paid = product();
        let money = Money::new(450, Currency::Usd).expect("a positive amount");
        paid.price = PriceGroup::Paid(
            PaidPrice::new(money, money, None, TaxCode::DigitalBooks).expect("one currency"),
        );
        assert_eq!(
            paid.into_canonical(identity()).price,
            PriceIntent::Paid(money),
            "the additional-licence price and the tax code have no canonical field and stay \
             on the TPT-base model"
        );
    }
}
