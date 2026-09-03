//! The manifest and the crate state one version, or the build fails.
//!
//! Two places carry the version because two audiences read it: a store reads
//! `manifest.json` and cargo reads `Cargo.toml`. Embedding the manifest at
//! compile time rather than reading it at run time means the test cannot pass
//! against a copy that the build did not use.

const MANIFEST: &str = include_str!("../static/manifest.json");

#[test]
fn the_manifest_states_the_crate_version() {
    let manifest: serde_json::Value =
        serde_json::from_str(MANIFEST).expect("manifest.json should be valid JSON");

    assert_eq!(
        manifest["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION")),
        "manifest.json and Cargo.toml must state one version; \
         bump both or the store ships a build that misreports itself"
    );
}
