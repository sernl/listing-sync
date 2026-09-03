//! The payout-field guard.
//!
//! Its own file because it is the one source in the crate that must name the
//! forbidden words, and a scan that included itself would trip on them. Every
//! other module is scanned, and the module list is derived from `lib.rs`'s own
//! declarations rather than maintained by hand: a hardcoded list is a list
//! that stops covering the crate the moment somebody adds a file, which is
//! exactly what happened to `upload.rs`.

/// The seller's payout configuration is reachable from the same session this
/// connector holds, and nothing this connector does needs it. The scan is over
/// this crate's own sources rather than over a list of constants, so a query
/// text, a url fragment or a field name added anywhere in the crate trips it.
const FORBIDDEN: [&str; 4] = ["payout", "hyperwallet", "bank", "SellerPayoutPreferences"];

/// Every module `lib.rs` declares, plus `lib.rs` itself. Held as
/// `include_str!` pairs because the contents must be readable without a
/// filesystem: the gated lane and the nix sandbox run the same test, and
/// `std::fs::read_to_string` is banned crate-wide besides.
const SOURCES: [(&str, &str); 13] = [
    ("lib.rs", include_str!("lib.rs")),
    ("classify.rs", include_str!("classify.rs")),
    ("endpoints.rs", include_str!("endpoints.rs")),
    ("flows.rs", include_str!("flows.rs")),
    ("form.rs", include_str!("form.rs")),
    ("identity.rs", include_str!("identity.rs")),
    ("live.rs", include_str!("live.rs")),
    ("read_model.rs", include_str!("read_model.rs")),
    ("s3.rs", include_str!("s3.rs")),
    ("session.rs", include_str!("session.rs")),
    ("standards.rs", include_str!("standards.rs")),
    ("upload.rs", include_str!("upload.rs")),
    ("write_model.rs", include_str!("write_model.rs")),
];

/// The file each `pub mod` line in `lib.rs` declares.
fn declared_modules() -> Vec<String> {
    include_str!("lib.rs")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub mod "))
        .filter_map(|rest| rest.strip_suffix(';'))
        .map(|module| format!("{module}.rs"))
        .collect()
}

#[test]
fn no_endpoint_in_this_crate_addresses_the_sellers_money() {
    for (name, source) in SOURCES {
        let lowered = source.to_lowercase();
        for needle in FORBIDDEN {
            assert!(
                !lowered.contains(&needle.to_lowercase()),
                "{name} names {needle:?}: this connector reads and writes products, and the \
                 seller's money is not its business"
            );
        }
    }
}

/// The guard above is only as wide as its list, and a list nobody checks stops
/// covering the crate silently. This is what makes a new module fail the lane
/// until it is scanned.
#[test]
fn the_guard_scans_every_module_this_crate_declares() {
    let declared = declared_modules();
    assert!(
        !declared.is_empty(),
        "the module list is derived from lib.rs's own `pub mod` lines, and finding none means \
         the derivation broke rather than that the crate has no modules"
    );
    for module in &declared {
        assert!(
            SOURCES.iter().any(|(name, _)| name == module),
            "{module} is declared by lib.rs but escapes the payout guard; add it to SOURCES"
        );
    }
    assert_eq!(
        SOURCES.len(),
        declared.len().saturating_add(1),
        "the scan covers every declared module and lib.rs itself, and nothing else: declared \
         {declared:?}"
    );
}
