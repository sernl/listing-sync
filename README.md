# Listing Sync

## Layout

- `docs/design/` — the specification (`2026-08-25-listing-sync-design.md`),
  the decision record (authoritative), the engineering charter, the
  enforcement toolchain, and the milestone plan.
- `docs/research/` — the four research documents the design is grounded in.
- `crates/` — the Rust workspace.

## Develop

A Nix flake provides the toolchain and checks.

```
nix develop        # dev shell with the pinned toolchain, nextest, cargo-deny, just
just check         # fmt check, clippy with zero warnings, tests
nix flake check    # everything CI runs
```

Without Nix, a Rust 1.97.1 toolchain with `rustfmt` and `clippy` suffices;
`rust-toolchain.toml` pins the version.

## Licence

Proprietary. See [`LICENSE`](LICENSE). All rights reserved.
