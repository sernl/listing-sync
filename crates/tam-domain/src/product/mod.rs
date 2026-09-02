//! The canonical product, on the TPT base, and the validation it runs before
//! anything is written.
//!
//! TPT is the base model by founder decision, so this module is the shape of
//! a product rather than one marketplace's projection of it: the nine groups
//! `#ItemAddForm` renders — Name, Files, Description, Price, Categories,
//! Education Standards, Details, Copyright, Product Status — with the types
//! the field catalogue states, read from `docs/research/rethink/tpt-product-model.md`
//! and `docs/research/rethink/tpt-create-form-dom.md`.
//!
//! [`TptBaseProduct`] is the source of truth for what a product is, and
//! [`CanonicalProduct`](crate::CanonicalProduct) is derived from it by
//! [`TptBaseProduct::into_canonical`] rather than being a second, parallel
//! definition. The storage side has not folded yet — `product` keeps its own
//! table and migration 0040 adds a sidecar — but the domain has one
//! definition either way, and the derivation is where the two are held
//! together. Folding the storage side is a scheduled step after the engine
//! driver split, not a permanent shape.
//!
//! Three further properties of this module are deliberate and each is
//! load-bearing.
//!
//! The selection caps are a parameter rather than a constant. They are facts
//! about a form that was measured on a particular day, they live in
//! `docs/design/data/tpt-vocabulary.json`, and `tam_taxonomy::TptForm` is the
//! reader. This crate cannot call that reader — `tam-taxonomy` depends on
//! `tam-domain`, so the edge only runs one way — so the caller supplies
//! [`SelectionCaps`] and the numbers still come from the capture rather than
//! from a literal typed here. A cap that is `None` is unmeasured, never
//! unlimited, and an unmeasured cap refuses nothing.
//!
//! The three listbox vocabularies are closed enums carrying both their wire
//! id and their menu position, because the two disagree. TPT's Answer Key
//! menu runs N/A, Included, Not Included, Included with Rubric, Rubric Only,
//! Does Not Apply, which is wire ids 0, 1, 2, 4, 5, 3; anything that infers
//! an id from a menu position writes "Included with Rubric" as "Does Not
//! Apply" and no seller can see that it happened. Making the id the value and
//! the position a derived function is what removes that class of bug.
//!
//! Nothing here has a `Default`. A tax code defaulted is a tax determination
//! the seller is contractually answerable for (D7), and a copyright
//! declaration defaulted is an attestation we made rather than they did (D13);
//! TPT's own form pre-selects `copyright_declaration` value `1` on a blank
//! form, and ours must not.

pub mod canonical;
pub mod fields;
pub mod validation;
pub mod vocabularies;

pub use canonical::ProductIdentity;
pub use fields::{
    suggested_additional_licence, CategoryGroup, DetailGroup, FacetSlug, FileGroup, PaidPrice,
    PriceGroup, ProductName, StandardAlignment, UploadRef, ADDITIONAL_LICENCE_PERCENTAGE,
    FREE_RESOURCE_PAGE_GUIDANCE, TITLE_MAX_UTF16_UNITS,
};
pub use validation::{AuthoringError, AuthoringWarning, FormGroup, Picker, Report, SelectionCaps};
pub use vocabularies::{
    AnswerKey, CopyrightDeclaration, ListingStatus, StandardsFramework, TaxCode, TeachingDuration,
    ThumbnailMode, COPYRIGHT_PREAMBLE,
};

use tam_types::ListingCopy;

// ------------------------------------------------------------ the aggregate

/// The whole canonical product, in TPT's own group order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TptBaseProduct {
    pub name: ProductName,
    pub files: FileGroup,
    pub description: ListingCopy,
    pub price: PriceGroup,
    pub categories: CategoryGroup,
    pub standards: Vec<StandardAlignment>,
    pub details: DetailGroup,
    /// Absent is a product that cannot be submitted. Held as an option rather
    /// than being required by the type so a draft in progress is
    /// representable; [`TptBaseProduct::check`] is what refuses it.
    pub copyright: Option<CopyrightDeclaration>,
    pub status: ListingStatus,
}

#[cfg(test)]
pub(crate) mod fixtures;
