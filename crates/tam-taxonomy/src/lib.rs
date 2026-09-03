//! The taxonomy hub: the pure projection function over the edge relation,
//! the crosswalk derivation over the captured Tes trees, and the Tes
//! age-range table, the create-form limits TPT's own capture states, and the
//! captured labels a form renders those vocabularies with. No I/O; the
//! durable side lives in tam-storage.

#![forbid(unsafe_code)]

pub mod grades;
pub mod licences;
pub mod listing;
pub mod native_labels;
pub mod project;
pub mod provenance;
pub mod resource_types;
pub mod subjects;
pub mod tes;
pub mod tpt_form;

pub use grades::{derive_grade_crosswalk, GradeCrosswalk, GradeError, Uncovered};
pub use licences::{derive_licence_crosswalk, LicenceCrosswalk, LicenceError};
pub use listing::{
    project_listing, project_listing_with_overrides, projection_vocabularies, routed_vocabularies,
    ListingContext,
};
pub use native_labels::native_label;
pub use project::{
    ingest, ingest_grades, project, project_axis, project_axis_with_overrides, project_terms,
    AxisRequest, BlockedTerm, GradeIngest, TermsOutcome,
};
pub use provenance::{check_native_ids, ForeignNativeId, ForeignNativeIds};
pub use resource_types::{
    derive_resource_type_crosswalk, ResourceTypeCrosswalk, ResourceTypeError,
};
pub use subjects::{
    derive_subject_crosswalk, SubjectCrosswalk, SubjectError, SubjectMismatch,
    SubjectMismatchReason, SubjectResidue,
};
pub use tes::{
    derive_crosswalk, derive_interval, parse_tree, Crosswalk, CrosswalkError, Mismatch,
    MismatchReason, Residue, ResidueNode, TesAgeRange, TesSubject, TesTopic, TesTree,
    TES_MAIN_AGE_RANGES,
};
pub use tpt_form::{FormError, Picker, TptForm};
