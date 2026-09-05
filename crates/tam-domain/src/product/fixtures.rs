//! The products these tests are written against.
//!
//! One fixture shared by the three test modules rather than three copies, so
//! a test that changes one control is visibly changing one control.

use super::fields::{
    CategoryGroup, DetailGroup, FacetSlug, FileGroup, PriceGroup, ProductName, UploadRef,
};
use super::validation::SelectionCaps;
use super::vocabularies::{CopyrightDeclaration, ListingStatus, ThumbnailMode};
use super::TptBaseProduct;
use crate::product::canonical::ProductIdentity;
use crate::RightsDeclaration;
use tam_types::{
    ContentHash, CopyFormat, FileBytes, FileId, FileKind, FileRole, ListingCopy, OrgId, PayloadSet,
    ProductFile, ProductId, ScanOutcome, Timestamp, Uuid,
};

pub(crate) const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// The numbers the committed capture states, which the API supplies from
/// `tam_taxonomy::TptForm`. `subject_areas` is `None` because a capture has
/// already exceeded the form's claim of three.
pub(crate) const CAPS: SelectionCaps = SelectionCaps {
    grades: Some(4),
    subject_areas: None,
    tags: Some(6),
    formats: Some(3),
    thumbnails: Some(4),
};

pub(crate) fn slugs(names: &[&str]) -> Vec<FacetSlug> {
    names
        .iter()
        .map(|name| FacetSlug::new(name).expect("a non-empty slug"))
        .collect()
}

pub(crate) fn product() -> TptBaseProduct {
    TptBaseProduct {
        name: ProductName::new("Fractions on a number line").expect("a short title"),
        files: FileGroup {
            payload: UploadRef::new(HASH).expect("a hex digest"),
            preview: None,
            video_preview: None,
            thumbnail_mode: ThumbnailMode::AutoGenerate,
            thumbnails: vec![],
        },
        description: ListingCopy {
            body: "Twelve task cards.".to_owned(),
            format: CopyFormat::Markdown,
        },
        price: PriceGroup::Free,
        categories: CategoryGroup {
            grades: slugs(&["3rd-grade"]),
            subject_areas: slugs(&["math"]),
            tags: slugs(&["centers"]),
            formats: vec![],
            custom_categories: vec![],
            appropriate_for_country: None,
        },
        standards: vec![],
        details: DetailGroup::default(),
        copyright: Some(CopyrightDeclaration::OriginalWork),
        status: ListingStatus::Draft,
    }
}

pub(crate) fn file(role: FileRole) -> ProductFile {
    ProductFile {
        id: FileId(Uuid([0x11; 16])),
        role,
        kind: FileKind::Pdf,
        bytes: FileBytes::Held {
            hash: ContentHash([0x22; 32]),
            byte_len: 2048,
            scan: ScanOutcome::Clean {
                at: Timestamp(1_700_000_000_000),
            },
        },
    }
}

pub(crate) fn identity() -> ProductIdentity {
    ProductIdentity {
        id: ProductId(Uuid([0x0f; 16])),
        org: OrgId(Uuid([0xaa; 16])),
        payload: Some(PayloadSet::new(file(FileRole::Payload), vec![])),
        cover: Some(file(FileRole::Cover)),
        previews: vec![file(FileRole::Preview)],
        subjects: vec![],
        derived_grades: None,
        rights: RightsDeclaration::Unstated,
    }
}
