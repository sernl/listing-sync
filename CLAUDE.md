# Listing Sync

Bulk upload and cross-listing for teaching-resource marketplaces.
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

- Automation is two-branch, and the branch is decided by whether the
  marketplace sanctions it. Where a marketplace publishes an official API and
  issues a token for the purpose, automation runs server-side, on
  infrastructure we operate, using that token; Etsy and Shopify are that
  branch. Where no official API exists, every marketplace request originates
  on the seller's own device under the seller's own session;
  TeachersPayTeachers and Tes are that branch. For the second branch the
  server is a control plane: it holds the catalogue, the mapping decisions,
  the ledger, the dashboard, the subscription and the kill switch, and it
  sends declarative intent describing an outcome. It never composes, signs or
  issues a request to a no-API marketplace, and never holds a session for one.
  The registry records a transport class per marketplace, and a test fails the
  build if a no-API marketplace gains a server transport. The schedule stays
  deterministic and cron-shaped in both branches; only the location of the
  timer moves, and for a no-API marketplace it runs on the seller's device.
- Rust is required for the engine, all I/O, batch processing, the automation
  layer, and anything computationally heavy. TypeScript is for the user
  interface, and for one bounded identity service. That service is
  better-auth, running as `tam-auth`, and it owns platform-user identity and
  browser session only: registration, sign-in, social and passkey
  credentials, email verification, password reset, and the keys for the
  tokens it issues. It owns no domain data, performs no marketplace request,
  and holds no marketplace credential. It reaches Postgres only as the
  `tam_auth` role, whose grants are confined to the `auth` schema; it never
  reads or writes a table in `public` and never sets `app.current_org`.
  Authorisation — which organisation a
  request speaks for and what it may do there — is decided in Rust from
  Postgres, and is never asserted by a token claim. The sole exception is the
  client entitlement token, which transports a decision Postgres already made
  so that a seller's device can run scheduled no-API work between check-ins;
  the server re-checks Postgres on every control-plane call, the client gate
  fails closed and is advisory, and it never grants anything the server did
  not (D10 in `docs/notes/design/vendoo-for-teachers-rethink.md`). Any
  extension of this service beyond identity and session is a new founder
  decision, not an application of this one.
- Sync is deterministic and cron-scheduled, never agent-driven. Models appear
  only in listing-copy generation and selector rediscovery.
- Markdown is one sentence per line; comments earn their place per the
  style policy in the charter.

Amended 2026-09-03: the automation non-negotiable above was reversed from
server-side-only to the two-branch rule, by founder decision D1. The full
decision set, the evidence behind it and the re-baselined plan are in
`docs/notes/design/vendoo-for-teachers-rethink.md`.

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
just dev-all       # everything in one terminal: database, migrations, API + auth + web
just dev           # full local environment: postgres (podman), migrations, API server
just dev-session   # mint a development login; paste the printed line into /login
just web-dev       # the client dev server, proxying /v1 to a running tam-server
just web-check     # the web lane: vocabulary freshness, svelte-check, vitest, build
just db-test       # the database-backed lane: two-tenant RLS isolation
just pre-push      # before pushing: check, then the database lane, no fail-fast
nix flake check    # everything
```

## Current state

The active milestone plan is the newest file in `docs/design/plans/`.
Progress against it is recorded in the jj log; the working tree stays green under `just check`.

Four database roles that can log in, deliberately: `tam_app` (the API path,
forced RLS, cannot read `connection_secret`), `tam_engine` (the cross-tenant
lease scan, BYPASSRLS), `tam_auth` (platform identity; confined to the `auth`
schema, holding no privilege on any table in `public` and not BYPASSRLS), and
`tam_backoffice` (the operator admin surface's cross-tenant reads, enumerated
per table in migration 0037 rather than granted as BYPASSRLS). The dev database
is created by `db/init/01-app-role.sql`, `db/init/02-auth-role.sql` and
`db/init/03-backoffice-role.sql`; `just db-setup` prepares rootless podman once
per machine.

Amended 2026-09-04: there were five that could log in. `tam_broker`, the only
role that read the credential vault, is retired with the session broker itself,
because D1 leaves no server-side seller session for a no-API marketplace and
therefore no vault for a role to be the sole reader of. Migration 0051 revokes
everything it held.

The role itself does not go everywhere at once, and the reason is the
frozen-migration rule. Migrations 0010, 0017 and 0032 grant to it by name, they
are applied and frozen, and Postgres errors on a `GRANT` naming a role that
does not exist — so a cluster that replays the migration set needs the name to
exist or it fails at 0010, in every `sqlx::test` database rather than in the
tests that care. In dev and CI it is therefore kept by
`db/init/01-app-role.sql`, which creates it inert on a fresh cluster and
converges an existing one — `NOLOGIN`, no password, no BYPASSRLS — every time
that file runs: every start on the ephemeral devshell path, but only at initdb
on the compose path, so a data directory older than this change keeps its login
and BYPASSRLS until `just db-reset` re-initialises it or the runbook's first
step alters it. Migration 0051 attempts the same convergence and skips it in
dev and CI, because migrations run there as `tam_app`, which may not alter a
role. Production drops the role outright, by the operator step in
`docs/notes/runbooks/retire-tam-broker-role.md`, which is safe there because
production applies migrations incrementally and never replays 0010 against a
fresh database. So dev names five roles and four of them can log in.

`connection_secret` stays standing: its rows are the only copy of the
credentials already sealed, and disposing of them is a later founder call.
