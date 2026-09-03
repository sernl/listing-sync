//! The extension shell, wasm half.
//!
//! A shell and nothing more. It exposes what a background loader needs to
//! prove the module is alive, and the one pattern the extension's transport
//! will be built out of. It holds no marketplace logic, contacts no
//! marketplace and names no marketplace host; those arrive only once gates G1
//! to G3 of `docs/research/rethink/oxichrome-extension-client.md` have
//! answered, and the extension is a candidate until they do.
//!
//! The boundary is JSON strings in both directions, on `tam-core-wasm`'s
//! discipline: one wire format rather than two, and an error shape rather than
//! a trap, because a trap poisons the module instance and takes every later
//! call with it. That matters more in a service worker the browser expects to
//! evict and revive than it does on a page.

pub mod bridge;

#[cfg(target_family = "wasm")]
pub mod alarms;
#[cfg(target_family = "wasm")]
pub mod transport;

use wasm_bindgen::prelude::wasm_bindgen;

/// The shell's version, which is the crate's version.
///
/// `static/manifest.json` carries the same string and `tests/version_sync.rs`
/// fails when the two drift apart, so the store's idea of the version and the
/// binary's cannot disagree.
#[wasm_bindgen]
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Liveness, as a JSON string.
///
/// `{"status":"ok","surface":"extension-shell","version":"..."}`. The loader
/// reads it to confirm instantiation, and the popup renders the version from
/// it, so both read one answer rather than two.
#[wasm_bindgen]
#[must_use]
pub fn health() -> String {
    serde_json::json!({
        "status": "ok",
        "surface": "extension-shell",
        "version": env!("CARGO_PKG_VERSION"),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::{health, version};

    #[test]
    fn health_reports_the_crate_version_it_was_built_from() {
        let reported: serde_json::Value =
            serde_json::from_str(&health()).expect("health should be JSON");

        assert_eq!(
            reported["version"].as_str(),
            Some(version().as_str()),
            "health and version should agree on one string"
        );
        assert_eq!(
            reported["status"].as_str(),
            Some("ok"),
            "a shell that answers at all is healthy"
        );
    }
}
