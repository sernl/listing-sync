# Listing Sync developer commands

# Query macros compile from committed .sqlx metadata; a live DATABASE_URL is
# opted into per recipe so `just check` stays hermetic
export SQLX_OFFLINE := "true"

# Local-dev Postgres on both provisioning paths; dev-only credential
db_url := "postgres://tam_app:tam_dev_password@127.0.0.1:5433/tam"

# The identity role, whose search_path is auth and whose grants stop at that
# schema; dev-only credential, matching db/init/02-auth-role.sql
auth_db_url := "postgres://tam_auth:tam_auth_dev@127.0.0.1:5433/tam"

# The pg-gated crates and the feature spelled per crate, shared by the two
# database-backed lanes so a crate cannot be added to one and missed by the other
pg_tests := "-p tam-storage --features pg-tests -p tam-api --features tam-api/pg-tests -p tam-import --features tam-import/pg-tests -p tam-session-broker --features tam-session-broker/pg-tests -p tam-engine --features tam-engine/pg-tests -p tam-sync-worker --features tam-sync-worker/pg-tests"

default:
    @just --list

# Format Rust and Nix
fmt:
    cargo fmt
    nix fmt

# The gated lane: zero warnings tolerated
check:
    cargo fmt --check
    cargo clippy --all-targets --all-features -- --deny warnings
    just purity
    cargo nextest run

# A convention is one `cargo add` from false: the pure core must never
# acquire a runtime, HTTP or database dependency (M1a plan, task 4)
purity:
    #!/usr/bin/env sh
    set -eu
    tree="$(cargo tree -e normal -p tam-types -p tam-marketplace -p tam-domain -p tam-authoring -p tam-taxonomy -p tam-standards -p tam-vocab-drift --prefix none)"
    if printf '%s\n' "$tree" | grep -E '^(tokio|tokio-util|reqwest|sqlx) v'; then
        echo 'purity violation: a banned dependency reached the pure core' >&2
        exit 1
    fi
    # The interpreter is held to the same bar plus `tam-storage`: it runs on
    # the seller's device, so a database handle, a runtime or a storage row in
    # its dependency graph is the boundary having failed rather than a
    # portability inconvenience.
    driver="$(cargo tree -e normal -p tam-engine-driver --prefix none)"
    if printf '%s\n' "$driver" | grep -E '^(tokio|tokio-util|reqwest|sqlx|tam-storage) v'; then
        echo 'purity violation: a banned dependency reached tam-engine-driver' >&2
        exit 1
    fi
    echo 'purity: the pure-core subgraph is clean'

# The client surfaces the pure core and the adapters must reach, one triple
# each: web and extension, Windows, macOS, iOS, Android. Kept out of `check`
# because it compiles the same crates five more times.
portable_targets := "wasm32-unknown-unknown x86_64-pc-windows-msvc aarch64-apple-darwin aarch64-apple-ios aarch64-linux-android"

# The transport-free set. --no-default-features drops the adapters' `live`
# feature, which is reqwest and nothing else; reqwest, tokio and rustls are
# native-only and stay behind it. The server-bound crates are absent by
# design: they hold sqlx, axum and a runtime, and no client compiles them.
# tam-engine-driver joined on 2026-09-03: the tam-limits usize::BITS assertion
# that failed it on wasm32 is now cfg-scoped to non-wasm targets by founder
# decision 3 of docs/research/rethink/oxichrome-extension-client.md, and its
# other blocker had already gone when blake3's `pure` feature dropped the cc
# and ml64.exe paths.
portable_crates := "-p tam-types -p tam-marketplace -p tam-domain -p tam-authoring -p tam-taxonomy -p tam-marketplace-tpt -p tam-marketplace-tes -p tam-engine-driver"

# The 64-bit-only leg. These two are not blocked any more — the narrowed
# assertion lets tam-limits compile for wasm32, which is how tam-engine-driver
# reaches it above — and listing them on the wasm leg directly is a separate
# call rather than a consequence of that one.
portable_crates_64 := "-p tam-limits -p tam-pipeline"

# Prove every client target still compiles. The standard libraries come from
# rust-toolchain.toml's `targets`, so this needs the devshell rather than a
# rustup install.
check-portable:
    #!/usr/bin/env sh
    set -eu
    for target in {{portable_targets}}; do
        crates="{{portable_crates}}"
        case "$target" in
            wasm32-*) ;;
            *) crates="$crates {{portable_crates_64}}" ;;
        esac
        echo "==> $target"
        cargo check --target "$target" --no-default-features $crates
    done

# Everything CI runs
ready: check
    nix flake check

# Dependency licence and advisory policy (proprietary licence gate), plus the
# serde_json depth bound, which moved here from deny.toml on 2026-09-03. The
# bound is a feature rather than a call site: with `unbounded_depth` off, the
# method that disables the 128-deep parse limit does not exist at all. But
# cargo-deny reads one unified feature set out of cargo metadata and cannot
# separate a build dependency's features from a shipped binary's, so its
# [[bans.features]] entry fired on tauri-build's chain and said nothing about
# what we ship. The per-binary cargo tree below asks only about what ships,
# and taking the binaries from cargo metadata covers a new one without an
# edit here. bash rather than sh because pipefail is what stops a failed
# cargo metadata from passing the check vacuously.

# Dependency licence and advisory policy, and the serde_json depth bound
deny:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo deny check
    crates=$(cargo metadata --format-version 1 --no-deps \
        | jq -r '.packages[] | select(any(.targets[]?; any(.kind[]?; . == "bin"))) | .name')
    for crate in $crates; do
        tree=$(cargo tree -e normal,no-proc-macro -p "$crate" -f '{p} {f}')
        found=$(printf '%s\n' "$tree" | grep 'serde_json v' | grep 'unbounded_depth' || true)
        if [ -n "$found" ]; then
            echo "serde_json feature 'unbounded_depth' reaches the shipped binary crate $crate" >&2
            exit 1
        fi
    done

# One-time host preparation for rootless podman: a user-level signature
# policy (accept-anything; image integrity comes from the digest pin in
# compose.yaml). Never overwrites an existing policy.
db-setup:
    #!/usr/bin/env sh
    set -eu
    policy="${XDG_CONFIG_HOME:-$HOME/.config}/containers/policy.json"
    if [ -e "$policy" ] || [ -e /etc/containers/policy.json ]; then
        echo 'a container signature policy already exists; leaving it untouched'
        exit 0
    fi
    mkdir -p "$(dirname "$policy")"
    printf '%s\n' '{"default":[{"type":"insecureAcceptAnything"}]}' > "$policy"
    echo "wrote $policy"

# Start the dev database (podman, image pinned by digest in compose.yaml)
db-up:
    #!/usr/bin/env sh
    set -eu
    if [ ! -e /etc/containers/policy.json ] \
        && [ ! -e "${XDG_CONFIG_HOME:-$HOME/.config}/containers/policy.json" ]; then
        echo 'no container signature policy found; run `just db-setup` once' >&2
        echo '(on NixOS, virtualisation.podman.enable = true also provides one)' >&2
        exit 1
    fi
    podman-compose up -d db

db-down:
    podman-compose down

# Destroy the dev database and its volume
db-reset:
    podman-compose down -v

# The same Postgres major straight from the devshell, for machines without
# a container runtime
db-up-ephemeral:
    bash db/ephemeral-postgres.sh up

db-down-ephemeral:
    bash db/ephemeral-postgres.sh down

# Wait until the dev database accepts connections
db-wait:
    #!/usr/bin/env sh
    set -eu
    for _ in $(seq 1 120); do
        if pg_isready -h 127.0.0.1 -p 5433 -U tam_app >/dev/null 2>&1; then
            exit 0
        fi
        sleep 0.5
    done
    echo 'database did not become ready on 127.0.0.1:5433' >&2
    exit 1

db-migrate:
    DATABASE_URL={{db_url}} sqlx migrate run --source crates/tam-storage/migrations

# Compare a freshly captured marketplace vocabulary against the committed one.
# The exit status is the alerting surface, as it is for tam-canary: zero when
# nothing structural moved, one when a facet appeared, disappeared, or changed
# its parent or category. A relabelled value with a stable identifier is
# written to the report and does not fail the run, because labels are read out
# of the captures rather than stored beside the terms.
#
# The fresh capture is a file this recipe is given rather than one it fetches:
# the re-capture is a marketplace request, and D1 puts that on the seller's own
# device for a marketplace with no official API.
vocab-drift inventory committed fresh captured_at:
    cargo run -q -p tam-vocab-drift -- \
        {{inventory}} {{committed}} {{fresh}} docs/design/data/drift {{captured_at}}

# Regenerate sqlx offline query metadata (crates/tam-storage/.sqlx, committed)
db-prepare:
    cd crates/tam-storage && SQLX_OFFLINE=false DATABASE_URL={{db_url}} cargo sqlx prepare

# Assert the committed offline query metadata matches the source, so schema
# drift fails here rather than at the first request. Needs both migration
# sets applied (db-migrate and auth-migrate): the metadata covers a query
# on auth.auth_event, the admin surface's signups read
db-verify:
    cd crates/tam-storage && SQLX_OFFLINE=false DATABASE_URL={{db_url}} cargo sqlx prepare --check

# The database-backed test lane: tenancy isolation, codecs, structural fences,
# the API driven in-process over per-test databases, and the engine driven
# end to end against a fake marketplace
db-test: db-wait db-verify
    DATABASE_URL={{db_url}} cargo nextest run {{pg_tests}}

# The same lane reporting every failure rather than stopping at the first,
# which is what a pre-push check needs: one early failure otherwise hides
# the rest and the next run finds them one at a time
db-test-all: db-wait db-verify
    DATABASE_URL={{db_url}} cargo nextest run --no-fail-fast {{pg_tests}}

# Everything that can fail before a push. The gated lane compiles the
# pg-gated tests but never runs them, so a query built from a literal SQL
# string and an assertion whose expected value has moved both reach main
# green; this runs them. web-check sits second because its first step is the
# vocabulary diff, and a Rust enum that moved leaves vocab.ts stale without
# failing anything in `check` -- cheaper to learn that before the database
# lane than after it.
pre-push: check web-check landing-check db-verify db-test-all check-portable

# Regenerate the client's vocabulary from the closed Rust enums
web-typegen:
    cargo run -p tam-api --bin typegen > web/src/lib/generated/vocab.ts

# The browser's copy of the core: the same rules the API answers with, compiled
# to wasm32 and bound for the browser. Generated output is gitignored and built
# here, so nothing generated is committed and no stale copy can ship.
#
# --lib is load-bearing: the fixture generator is a bin in the same crate and
# does not cross-compile.
# The artefact path honours CARGO_TARGET_DIR rather than assuming `target/`,
# because agents working this repository in parallel each set their own: two
# sharing one target directory measured 2.1 times slower, so a private one is
# the standing arrangement and a hardcoded path silently breaks it.
web-wasm:
    cargo build --target wasm32-unknown-unknown --release --lib -p tam-core-wasm
    wasm-bindgen --target web --out-name core \
        --out-dir web/src/lib/core/generated \
        "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/tam_core_wasm.wasm"

# Re-record what the native path decides for each fixture draft. The vitest
# suite and a Rust test both compare against the recorded file, so run this
# only when a rule changed on purpose; the diff is the record of that change.
# The hand-written half of the extension, listed rather than globbed so the
# package's contents are decided here and not by the shell's expansion order.
extension_static := "manifest.json background.js popup.html popup.js"

# The extension shell's wasm, built exactly as `web-wasm` builds the core: the
# same pinned wasm-bindgen, a second cdylib, and nothing else structural.
extension-wasm:
    # Absolute paths reach the binary through panic messages and debug info, so
    # without remapping the bytes depend on where the checkout and the registry
    # cache happen to sit, and a reviewer rebuilding from the source package
    # cannot reproduce ours.
    RUSTFLAGS="--remap-path-prefix=$PWD=/build --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}/registry/src=/registry" \
        cargo build --target wasm32-unknown-unknown --release --lib -p tam-extension
    wasm-bindgen --target web --out-name extension \
        --out-dir apps/extension/dist \
        "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/tam_extension.wasm"

# The generated half of the package, named so the .d.ts files wasm-bindgen also
# writes stay out of what ships
extension_built := "extension.js extension_bg.wasm"

# The unpacked extension, loadable from disk with "Load unpacked"
extension-dev: extension-wasm
    #!/usr/bin/env sh
    set -eu
    for file in {{extension_static}}; do
        cp "apps/extension/static/$file" "apps/extension/dist/$file"
    done
    echo "extension: apps/extension/dist is loadable unpacked"

# The package a store receives. Deterministic by construction: sorted entries,
# a fixed timestamp and fixed permissions, so two builds of one tree are
# byte-identical and a reviewer can check the artefact against the source
# package below.
extension-build: extension-dev
    python3 apps/extension/pack.py \
        apps/extension/build/tam-extension.zip \
        apps/extension/dist \
        {{extension_static}} {{extension_built}}

# The reviewable source package Mozilla requires for machine-generated code:
# source, lockfiles, the pinned toolchain and the command list, sufficient to
# rebuild the byte-identical zip from a clean checkout.
extension-source-package:
    python3 apps/extension/source-package.py

web-wasm-fixtures:
    cargo run -q -p tam-core-wasm --bin verdict-fixtures > crates/tam-core-wasm/fixtures/verdicts.json

# The web lane: lockfile install, vocabulary freshness, types, tests, build
web-check: web-wasm
    cargo run -p tam-api --bin typegen | diff -u web/src/lib/generated/vocab.ts - \
        || (echo "vocab.ts is stale; run just web-typegen" && exit 1)
    cd web && npm ci --no-audit --no-fund
    cd web && npx svelte-kit sync && npx svelte-check --fail-on-warnings
    cd web && npx vitest run
    cd web && npm run build

# The client dev server, proxying /v1 to a locally running tam-server
web-dev: web-wasm
    cd web && npm run dev

# The public site at teachouse.io, which is a separate Astro build rather than
# a console route: the console's root layout turns off both server rendering
# and prerendering, which SvelteKit's own documentation calls a large negative
# for performance and search, and the marketing page is the one surface where
# that matters (D28).
#
# The landing lane: lockfile install, then the static build
landing-check:
    cd apps/landing && npm ci --no-audit --no-fund
    cd apps/landing && npm run build

# The landing-page dev server
landing-dev:
    cd apps/landing && npm run dev

# The desktop client (Tauri v2, D2), which hosts this same console and owns the
# seller's marketplace sessions on the seller's own device. The console comes
# from the SvelteKit dev server, so `just web-dev` must already be running.
#
# The desktop client against a running `just web-dev`
desktop-dev:
    cd apps/desktop && cargo tauri dev

# Windows is the shipping surface; this is the bundle that needs no
# cross-compile, and it is how the client is exercised on this machine. The web
# build runs first because the bundle embeds its output.
#
# The Linux desktop bundle
desktop-build:
    cd web && npm run build
    cd apps/desktop && cargo tauri build --bundles deb

# D29's local Windows route. Tauri's own documentation calls the cross-compile
# a last resort and an MSI needs Windows, so the release path stays the GitHub
# Windows runner and this recipe is the local one.
#
# The Windows NSIS installer, cross-compiled through cargo-xwin
desktop-build-windows:
    cd web && npm run build
    cd apps/desktop && env -u CC -u CXX -u NIX_CFLAGS_COMPILE -u NIX_LDFLAGS PATH="$TAURI_WINDOWS_TOOLCHAIN_BIN:$PATH" cargo tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis

# The Android client (D2's second surface). Every recipe below needs the
# Android shell rather than the default one -- `nix develop .#android` -- which
# carries the SDK, the NDK, a JDK and all four Android rust targets. D3 limits
# a phone to work the seller starts, so nothing here schedules anything.
#
# The generated tree is committed rather than regenerated, because the release
# signing config lives in its build.gradle.kts and a build that patched that
# file on the fly would emit an unsigned release rather than an error.
#
# Regenerate gen/android, after a bundle identifier or application name change
android-init:
    cd apps/desktop && cargo tauri android init

# arm64 only: this is the founder's own phone, and every further ABI is a full
# rebuild of the crate graph.
#
# The debug APK
android-build-debug:
    cd web && npm run build
    cd apps/desktop && cargo tauri android build --apk --debug --ci --target aarch64

# Signed when gen/android/keystore.properties is present and unsigned when it
# is not, which is Gradle's own behaviour rather than something this recipe
# decides. arm64 and armv7 rather than all four: the two x86 ABIs are emulator
# and Chromebook targets, and adding one is a word here when a seller needs it.
#
# The release APK
android-build:
    cd web && npm run build
    cd apps/desktop && cargo tauri android build --apk --ci --target aarch64 --target armv7

# Universal rather than per-ABI, because Play splits it itself.
#
# The Play bundle
android-build-aab:
    cd web && npm run build
    cd apps/desktop && cargo tauri android build --aab --ci

# Refuse a release the updater could never serve. Checks that the two version
# fields agree, that an optional tag agrees with them, and that the updater
# public key and endpoint are no longer placeholders. The desktop-release
# workflow runs this same script before it builds anything, so a mismatch
# costs seconds here rather than a Windows runner's minutes there.
#
# The distribution pipeline it gates is docs/notes/design/desktop-distribution.md.
#
# Check the tree, or check a tag you are about to create
release-check tag="":
    sh .github/scripts/release-check.sh {{ tag }}

# Give tam-auth an environment file, creating one from the template on a
# machine that has none. An existing auth/.env is never touched.
auth-env:
    #!/usr/bin/env sh
    set -eu
    # tam-auth's secrets are not defaulted here the way the dev database
    # credentials are: node's --env-file loses to anything already in the
    # environment, so a default exported by this recipe would silently shadow
    # a developer's own auth/.env rather than yield to it.
    if [ -f auth/.env ]; then
        exit 0
    fi
    if [ ! -f auth/.env.example ]; then
        echo 'auth/.env is missing and so is auth/.env.example, which this' >&2
        echo 'recipe copies it from. Restore the template, or write auth/.env' >&2
        echo 'by hand against the variables auth/src/env.ts reads.' >&2
        exit 1
    fi
    cp auth/.env.example auth/.env
    # Not `openssl rand -base64 32`, which the error message this recipe
    # replaced advised: openssl is not in this flake's devshell. Thirty-two
    # bytes of /dev/urandom, base64-encoded, is the same secret.
    secret="$(head -c 32 /dev/urandom | base64)"
    sed -i "s|^BETTER_AUTH_SECRET=.*|BETTER_AUTH_SECRET=$secret|" auth/.env
    # The template is not read here, so the substitution is checked rather
    # than assumed: a missing or commented-out key would otherwise leave the
    # secret unset and fail at tam-auth's startup instead.
    grep -q '^BETTER_AUTH_SECRET=.' auth/.env \
        || printf 'BETTER_AUTH_SECRET=%s\n' "$secret" >> auth/.env
    echo 'created auth/.env from auth/.env.example with a fresh BETTER_AUTH_SECRET'

# The auth lane: lockfile install, types
auth-check:
    cd auth && npm ci --no-audit --no-fund
    cd auth && npx tsc --noEmit

# Emit the DDL auth/src/auth.ts implies, to diff against db/auth/
auth-ddl: db-wait auth-env
    cd auth && npm run --silent ddl

# Apply the reviewed identity DDL as tam_auth, which owns the auth schema.
# Which files have run is tracked in a ledger table created here rather than in
# db/auth/, whose files are verbatim generator output that `just auth-ddl`
# diffs byte for byte against auth/src/auth.ts.
auth-migrate: db-wait
    #!/usr/bin/env sh
    set -eu
    auth_psql() { psql '{{auth_db_url}}' -X -q -v ON_ERROR_STOP=1 "$@"; }
    auth_psql -c 'SET client_min_messages = warning; CREATE TABLE IF NOT EXISTS auth.applied_migration (name text PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())'
    # A database migrated before the ledger existed holds the DDL with no row
    # to show for it. One named marker per file rather than a general
    # heuristic: a wrong guess here would skip a migration in silence.
    auth_psql -c "INSERT INTO auth.applied_migration (name) SELECT '0001_identity.sql' WHERE to_regclass('auth.\"user\"') IS NOT NULL ON CONFLICT DO NOTHING"
    auth_psql -c "INSERT INTO auth.applied_migration (name) SELECT '0002_audit_event.sql' WHERE to_regclass('auth.auth_event') IS NOT NULL ON CONFLICT DO NOTHING"
    for file in db/auth/*.sql; do
        name="$(basename "$file")"
        if [ -n "$(auth_psql -At -c "SELECT 1 FROM auth.applied_migration WHERE name = '$name'")" ]; then
            echo "already applied $name"
            continue
        fi
        echo "applying $name"
        # One transaction over the file and its ledger row, so neither can
        # land without the other and a failed run leaves nothing half-applied.
        auth_psql --single-transaction -f "$file" \
            -c "INSERT INTO auth.applied_migration (name) VALUES ('$name')"
    done

# The identity service: better-auth over /api/auth/*, nothing else
auth-dev: auth-env
    cd auth && npm run dev

# Mint a development login: ensures the dev organisation and founder user
# exist, then prints the tam_session=... line the login page asks for
dev-session: db-wait
    cargo run -p tam-mint-session -- {{db_url}} aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
        founder@example.test --ensure-org founder-dev

# Full local environment: database, migrations, API server
dev: db-up db-wait db-migrate
    @echo "need a login? in another terminal:  just dev-session"
    cargo run -p tam-server -- {{db_url}}

# The whole environment in one terminal: database, both migration sets, an
# environment file if the machine has none, then the API server, the identity
# service and the client dev server together.
dev-all: db-up db-wait db-migrate auth-migrate auth-env
    #!/usr/bin/env sh
    set -eu
    # Read back through the same URL-origin normalisation auth/src/env.ts
    # applies, so the issuer tam-server is told to expect is byte-identical to
    # the `iss` tam-auth signs into its assertions.
    issuer="$(node --env-file=auth/.env \
        -e 'process.stdout.write(new URL(process.env.TAM_AUTH_BASE_URL).origin)')"
    echo "listing-sync development environment"
    echo "  web    http://localhost:5173"
    echo "  auth   $issuer/api/auth"
    echo "  API    http://127.0.0.1:8080"
    echo "need a login? in another terminal:  just dev-session"
    # kill 0 signals this recipe's whole process group, which is what reaps
    # cargo's and npm's own children rather than orphaning them; the trap is
    # cleared first so the signal it sends cannot re-enter it.
    trap 'trap - INT TERM; kill 0' INT TERM
    cargo run -p tam-server -- '{{db_url}}' \
        --auth-issuer "$issuer" --auth-jwks-url "$issuer/api/auth/jwks" &
    (cd auth && npm run dev) &
    (cd web && npm run dev) &
    wait
