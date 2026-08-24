#!/usr/bin/env bash
# Re-probe marketplace egress reachability. Reusable whenever the host moves.
# Usage: probes/reachability.sh <url> [user-agent]
set -euo pipefail
url="${1:?usage: reachability.sh <url> [user-agent]}"
ua="${2:-curl/8.0}"
status="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 25 -A "$ua" "$url" || echo 000)"
now="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
printf '%s\t%s\t%s\t%s\n' "$url" "$status" "$ua" "$now"
