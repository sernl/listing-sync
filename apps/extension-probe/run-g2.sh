#!/usr/bin/env bash
# Gate G2: start the local origin, load the unpacked extension, collect the report.
# Profiles are left under /tmp for inspection; nothing here deletes.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:-/tmp/g2-result.json}"
profile="$(mktemp -d /tmp/g2-profile.XXXXXX)"

G2_OUT="$out" nix shell nixpkgs#python3 --command python3 "$here/servers/g2-server.py" &
server=$!
sleep 1

nix shell nixpkgs#chromium --command chromium \
  "${CHROME_MODE:---headless=new}" \
  --no-first-run --no-default-browser-check --disable-gpu \
  --user-data-dir="$profile" \
  --disable-extensions-except="$here/g2-redirect" \
  --load-extension="$here/g2-redirect" \
  about:blank &
browser=$!

status=0
wait "$server" || status=$?
kill "$browser" 2>/dev/null || true
wait "$browser" 2>/dev/null || true
echo "profile: $profile"
exit "$status"
