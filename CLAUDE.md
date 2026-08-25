# Listing Sync

Server-side bulk upload and cross-listing for teaching-resource marketplaces.
This file orients an agent working in this repository; the authoritative
sources are under `docs/design/`.

## Read first

- `docs/design/decisions.md` — the decision record. Authoritative, and it
  overrides the research where they conflict. Several decisions were taken
  deliberately against research advice; do not reopen them.
- `docs/design/2026-08-25-listing-sync-design.md` — the specification.
- `docs/design/engineering-charter.md`, `operational-charter.md`,
  `enforcement-toolchain.md` — how the code is built and enforced.
- `docs/design/milestones.md` — the plan and its kill gates.

## Non-negotiables

- Automation runs server-side, on infrastructure we operate. The client is
  thin and performs no automation; it renders progress.
- Rust is required for the engine, all I/O, batch processing, the automation
  layer, and anything computationally heavy. TypeScript is for the UI only.
- Sync is deterministic and cron-scheduled, never agent-driven. Models appear
  only in listing-copy generation and selector rediscovery.
- Markdown is one sentence per line; comments earn their place per the
  style policy in the charter.

## Enforcement is founder-gated

The workspace lints table (in `Cargo.toml`), and the `clippy.toml`, `deny.toml`
and `crates/tam-limits` files once present, are shared gates. Changing a limit,
relaxing a lint, or adding a dependency is a founder decision, not a way to make
a build pass. A crate-local `clippy.toml` replaces the root file rather than
merging with it, so there is exactly one, at the root.

The full enforcement config is verified in `docs/design/enforcement-toolchain.md`
and is activated incrementally: commit one carries the workspace-lints table and
`tam-limits`; the `disallowed-methods` layer and its `ban-probe` land in M0 with
the dependencies they govern.

## Build

```
just check         # the gated lane: fmt, clippy --deny warnings, purity, tests
just db-setup      # once per machine: rootless-podman signature policy
just dev           # full local environment: postgres (podman), migrations, API server
just dev-session   # mint a development login; paste the printed line into /login
just web-dev       # the client dev server, proxying /v1 to a running tam-server
just web-check     # the web lane: vocabulary freshness, svelte-check, vitest, build
just db-test       # the database-backed lane: two-tenant RLS isolation
nix flake check    # everything
```

## Current state

The active milestone plan is the newest file in `docs/design/plans/`.
Progress against it is recorded in the jj log; the working tree stays green under `just check`.

Three database roles, deliberately: `tam_app` (the API path, forced RLS,
cannot read `connection_secret`), `tam_engine` (the cross-tenant lease scan,
BYPASSRLS), `tam_broker` (the only role that reads the credential vault). The
dev database is created by `db/init/01-app-role.sql`; `just db-setup` prepares
rootless podman once per machine.
