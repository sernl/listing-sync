//! Puts every crate the workspace ban list names in front of clippy, and arms
//! by call site the two entries clippy cannot report on.

pub use futures as _futures;
pub use reqwest as _reqwest;
pub use serde_json as _serde_json;
pub use tokio as _tokio;

/// Arms the ban on `str::split_at`.
///
/// Clippy emits no reachability diagnostic for a primitive-type path, so a typo
/// in that `disallowed-methods` entry disarms it in silence. Calling the method
/// under an `expect` attribute inverts that: if the entry stops resolving, the
/// expectation goes unfulfilled and the gated lane fails.
#[expect(clippy::disallowed_methods, reason = "the call site is the probe")]
pub const fn split_at_ban_is_armed(s: &str) -> (&str, &str) {
    s.split_at(0)
}

/// Arms the ban on `str::split_at_mut`, by the mechanism above.
#[expect(clippy::disallowed_methods, reason = "the call site is the probe")]
pub const fn split_at_mut_ban_is_armed(s: &mut str) -> (&mut str, &mut str) {
    s.split_at_mut(0)
}
