//! The job engine as a library: the effect driver, the fleet breaker, the
//! outbox drainer and the canary probe. Binaries hold wiring and nothing
//! else. The governing axiom throughout: a stalled queue is recoverable and
//! a duplicate-upload storm is not, so every ambiguous resolution here
//! biases toward stalling.

#![forbid(unsafe_code)]

/// The wait capability the driver's verification poll takes, re-exported so
/// a binary can bind a real sleep without depending on `tam-marketplace`
/// directly: this crate holds no timer by design, and the poll is the only
/// place in the engine that waits.
pub use tam_marketplace::Pause;

pub mod breaker;
pub mod broker_client;
pub mod canary;
pub mod driver;
pub mod outbox;
pub mod seed;
