//! The four education-standards frameworks TPT's create form offers, ingested
//! into one shape: Common Core, NGSS, the Texas Essential Knowledge and
//! Skills, and Virginia's Standards of Learning.
//!
//! Pure by construction. The model, the on-disk format, the loader, the picker
//! searches and the licence notices live here; fetching them from the mirror
//! is `tam-standards-fetch`, which is the only crate in this pair that touches
//! the network.
//!
//! What this crate does not do is as deliberate as what it does. There is no
//! cross-framework crosswalk: no owner publishes one, TPT declines to
//! translate CCSS into TEKS or VA SOL citing structural dissimilarity, and it
//! is recorded as a non-goal rather than a backlog item. And there is no
//! function that paraphrases, truncates or rewrites a statement, because the
//! Common Core grant names copy, publish, distribute and display, and not
//! modification.
//!
//! The design note is `docs/notes/design/standards-ingestion.md`; the source
//! research it implements is
//! `docs/research/rethink/education-standards-sources.md`.

#![forbid(unsafe_code)]

pub mod bind;
pub mod crawl;
pub mod format;
pub mod grades;
pub mod model;
pub mod notices;
pub mod search;
pub mod tpt;

pub use bind::{
    bind_walk, BindOutcome, CatalogueIndex, CatalogueRow, JurisdictionScope, Residue, Unresolved,
};
pub use crawl::{
    diff_capture, gate, is_utc_timestamp, normalise_statement, postable, statement_hash,
    tpt_jurisdiction, tpt_jurisdiction_notation, CrawlDiff, CrawlWindow, CrawledStandard,
    DriftedStatement, GateVerdict, MovedId, NotPostable,
};
pub use format::{
    content_hash, load_framework, parse_manifest, render_jsonl, FrameworkManifest, LoadError,
    Manifest, StandardsDocument, FORMAT,
};
pub use grades::{GradeBand, GradeInterval, GradeLevel};
pub use model::{
    addressable, dotted_prefixes, DocumentRef, Framework, Jurisdiction, Licence, NodeShape, SetRef,
    StandardNode, FRAMEWORKS,
};
pub use notices::{
    mirror_attribution, required_notices, Notice, Placement, CCSS_COPYRIGHT_NOTICE,
    CCSS_OWNERSHIP_ACKNOWLEDGEMENT, NGSS_CITATION, NGSS_MARK_RULES, NGSS_TRADEMARK_DISCLAIMER,
};
pub use search::{fold_code, Match, StandardsIndex};
pub use tpt::{load_tpt_node_ids, TptBinding, TptNodeId, TptNodeIdTable};
