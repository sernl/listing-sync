# Listing Sync developer commands

# Query macros compile from committed .sqlx metadata; a live DATABASE_URL is
# opted into per recipe so `just check` stays hermetic
export SQLX_OFFLINE := "true"

# Local-dev Postgres on both provisioning paths; dev-only credential
db_url := "postgres://tam_app:tam_dev_password@127.0.0.1:5433/tam"

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

# Regenerate sqlx offline query metadata (crates/tam-storage/.sqlx, committed)
db-prepare:
    cd crates/tam-storage && SQLX_OFFLINE=false DATABASE_URL={{db_url}} cargo sqlx prepare

# Assert the committed offline query metadata matches the source, so schema
# drift fails here rather than at the first request
db-verify:
    cd crates/tam-storage && SQLX_OFFLINE=false DATABASE_URL={{db_url}} cargo sqlx prepare --check

# The database-backed test lane: tenancy isolation, codecs, structural fences,
# and the API driven in-process over per-test databases
db-test: db-wait db-verify
    DATABASE_URL={{db_url}} cargo nextest run -p tam-storage --features pg-tests -p tam-api --features tam-api/pg-tests -p tam-import --features tam-import/pg-tests

# Regenerate the client's vocabulary from the closed Rust enums
web-typegen:
    cargo run -p tam-api --bin typegen > web/src/lib/generated/vocab.ts

# The web lane: lockfile install, vocabulary freshness, types, tests, build
web-check:
    cargo run -p tam-api --bin typegen | diff -u web/src/lib/generated/vocab.ts - \
        || (echo "vocab.ts is stale; run just web-typegen" && exit 1)
    cd web && npm ci --no-audit --no-fund
    cd web && npx svelte-kit sync && npx svelte-check --fail-on-warnings
    cd web && npx vitest run
    cd web && npm run build

# The client dev server, proxying /v1 to a locally running tam-server
web-dev:
    cd web && npm run dev

# Mint a development login: ensures the dev organisation and founder user
# exist, then prints the tam_session=... line the login page asks for
dev-session: db-wait
    cargo run -p tam-mint-session -- {{db_url}} aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
        founder@example.test --ensure-org founder-dev

# Full local environment: database, migrations, API server
dev: db-up db-wait db-migrate
    @echo "need a login? in another terminal:  just dev-session"
    cargo run -p tam-server -- {{db_url}}
