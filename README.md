# Listing Sync

Server-side bulk upload and cross-listing for teacher-authors who sell digital
teaching resources on marketplaces that offer no bulk create and no write API.

Status: design complete, pre-implementation. The first chargeable product is
Tes GB-to-US inventory duplication. See [`docs/design/`](docs/design/) for the
full specification, the engineering charter, and the milestone plan.

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
