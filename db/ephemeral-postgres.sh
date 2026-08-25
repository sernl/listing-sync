#!/usr/bin/env bash
# The same Postgres major as compose.yaml, run straight from the devshell for
# machines without a container runtime. State lives in .dev/postgres
# (gitignored); the listen address and port match compose so DATABASE_URL is
# identical on both paths.
set -euo pipefail

cmd="${1:?usage: ephemeral-postgres.sh up|down|status}"
root="$(cd "$(dirname "$0")/.." && pwd)"
state="$root/.dev/postgres"
data="$state/data"
sock="$state/sock"

case "$cmd" in
up)
    if [ ! -d "$data" ]; then
        mkdir -p "$sock"
        initdb --auth=trust --username=postgres --pgdata="$data" >/dev/null
    fi
    pg_ctl -D "$data" -l "$state/server.log" \
        -o "-p 5433 -c listen_addresses=127.0.0.1 -c unix_socket_directories='$sock'" \
        start
    psql -v ON_ERROR_STOP=1 -h 127.0.0.1 -p 5433 -U postgres -d postgres \
        -f "$root/db/init/01-app-role.sql"
    ;;
down)
    pg_ctl -D "$data" stop
    ;;
status)
    pg_ctl -D "$data" status
    ;;
*)
    echo "unknown command: $cmd" >&2
    exit 1
    ;;
esac
