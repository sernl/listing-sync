//! The taxonomy hub: the pure projection function over the edge relation,
//! the crosswalk derivation over the captured Tes trees, and the Tes
//! age-range table. No I/O; the durable side lives in tam-storage.

#![forbid(unsafe_code)]

pub mod grades;
pub mod licences;
pub mod listing;
pub mod project;
pub mod tes;

pub use grades::{derive_grade_crosswalk, GradeCrosswalk, GradeError, Uncovered};
pub use licences::{derive_licence_crosswalk, LicenceCrosswalk, LicenceError};
pub use listing::{project_listing, ListingContext};
pub use project::{
    ingest, project, project_axis, project_terms, AxisRequest, BlockedTerm, TermsOutcome,
};
pub use tes::{
    derive_crosswalk, derive_interval, parse_tree, Crosswalk, CrosswalkError, Mismatch,
    MismatchReason, Residue, ResidueNode, TesAgeRange, TesSubject, TesTopic, TesTree,
    TES_MAIN_AGE_RANGES,
};
