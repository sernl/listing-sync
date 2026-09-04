//! The effect interpreter and the ledger it speaks to, with no storage,
//! runtime or transport of its own.
//!
//! The crate exists so the boundary is proved by compilation rather than by
//! review: `just purity` bans `sqlx`, `tokio`, `tokio-util` and `tam-storage`
//! here, so a port method that needs a database connection, a timer or a
//! storage row cannot be written at all. What remains is the process that
//! decides which marketplace request to make, which is the process D1 puts on
//! the seller's own device.
//!
//! The ledger vocabulary is this crate's own rather than `tam-storage`'s,
//! because these types become the Phase 2 wire shapes and a storage row that
//! happens to match one today is a coincidence rather than a contract. The
//! conversions live on the server side, where a variant added to either
//! vocabulary fails to compile instead of being silently mapped.
//!
//! Portability is asserted for four client triples rather than five: the
//! remaining `tam-limits` reads, the attempt budget and the wall-clock
//! deadline, keep the crate off the wasm leg because `tam-limits` asserts
//! `usize::BITS >= 64`. Step 8 moves both into the claim envelope alongside
//! the rate ceiling, as the design note's section 4 already requires, and the
//! crate joins the five-target list then.

#![forbid(unsafe_code)]

#[cfg(any(test, feature = "testing"))]
pub mod conformance;
pub mod driver;
pub mod import;
#[cfg(any(test, feature = "testing"))]
pub mod memory;
pub mod ports;
pub mod seed;
pub mod vocabulary;
