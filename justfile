# Listing Sync developer commands

default:
    @just --list

# Format Rust and Nix
fmt:
    cargo fmt
    nix fmt

# The gated lane: zero warnings tolerated
check:
    cargo fmt --check
    cargo clippy --all-targets -- --deny warnings
    cargo nextest run

# Everything CI runs
ready: check
    nix flake check

# Dependency licence and advisory policy (proprietary licence gate)
deny:
    cargo deny check
