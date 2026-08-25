//! The file pipeline: the attacker-supplied-content boundary. Every
//! decompression is bounded by a caller-supplied budget, kinds come from
//! magic bytes rather than trust, and the heavy native closures live here so
//! they never enter the API's closure.

#![forbid(unsafe_code)]

pub mod archive;
pub mod hash;
pub mod pipeline;
pub mod probe;
pub mod render;
pub mod scan;
pub mod store;
