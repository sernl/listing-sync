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
    just purity
    cargo nextest run

# A convention is one `cargo add` from false: the pure core must never
# acquire a runtime, HTTP or database dependency (M1a plan, task 4)
purity:
    #!/usr/bin/env sh
    set -eu
    tree="$(cargo tree -e normal -p tam-types -p tam-marketplace -p tam-domain --prefix none)"
    if printf '%s\n' "$tree" | grep -E '^(tokio|tokio-util|reqwest|sqlx) v'; then
        echo 'purity violation: a banned dependency reached the pure core' >&2
        exit 1
    fi
    echo 'purity: the pure-core subgraph is clean'

# Everything CI runs
ready: check
    nix flake check

# Dependency licence and advisory policy (proprietary licence gate)
deny:
    cargo deny check
