//! The job engine as a library: the effect driver, the fleet breaker, the
//! outbox drainer and the canary probe. Binaries hold wiring and nothing
//! else. The governing axiom throughout: a stalled queue is recoverable and
//! a duplicate-upload storm is not, so every ambiguous resolution here
//! biases toward stalling.

#![forbid(unsafe_code)]

pub mod breaker;
pub mod canary;
pub mod driver;
pub mod outbox;
