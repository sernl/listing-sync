//! The taxonomy hub: the pure projection function over the edge relation,
//! the crosswalk derivation over the captured Tes trees, and the Tes
//! age-range table. No I/O; the durable side lives in tam-storage.

#![forbid(unsafe_code)]

pub mod listing;
pub mod project;
pub mod tes;

pub use listing::{project_listing, ListingContext};
pub use project::{ingest, project, project_terms, BlockedTerm, TermsOutcome};
pub use tes::{
    derive_crosswalk, derive_interval, parse_tree, Crosswalk, CrosswalkError, Mismatch,
    MismatchReason, Residue, ResidueNode, TesAgeRange, TesSubject, TesTopic, TesTree,
    TES_MAIN_AGE_RANGES,
};
