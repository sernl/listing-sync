#!/usr/bin/env bash
# Verifies the G1 probe end to end against the local mimic. Never contacts a
# marketplace: the origin handed to the probe is 127.0.0.1.
#
# The popup is opened through the DevTools HTTP endpoint rather than as a
# startup URL, because a chrome-extension:// startup URL is resolved before the
# unpacked extension finishes loading and is silently dropped.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
ext="$here/g1-tpt-envelope"
out="${1:-/tmp/g1-selfcheck.json}"
port="${G1_DEBUG_PORT:-9334}"
profile="$(mktemp -d /tmp/g1-profile.XXXXXX)"

# Chrome derives an unpacked extension's id from the absolute path of its
# directory, confirmed against the browser's own /json/list.
id="$(python3 -c "
import hashlib,sys
h=hashlib.sha256(sys.argv[1].encode()).hexdigest()[:32]
print(''.join(chr(ord('a')+int(c,16)) for c in h))" "$ext")"
echo "extension id: $id"

G1_OUT="$out" nix shell nixpkgs#python3 --command python3 "$here/servers/g1-mimic.py" &
mimic=$!
sleep 1

nix shell nixpkgs#chromium --command chromium \
  "${CHROME_MODE:---headless=new}" \
  --no-first-run --no-default-browser-check --disable-gpu \
  --remote-debugging-port="$port" \
  --user-data-dir="$profile" \
  --disable-extensions-except="$ext" --load-extension="$ext" \
  about:blank > "$profile/chromium.log" 2>&1 &
browser=$!

for _ in $(seq 1 40); do
  if curl -s -o /dev/null "http://127.0.0.1:$port/json/version"; then break; fi
  sleep 0.5
done

query="open=1&origin=http%3A%2F%2F127.0.0.1%3A8733"
query="$query&autorun=route-a-login,route-a-create,route-b-login,route-b-create,route-c-create,route-d-create"
query="$query&report=http%3A%2F%2F127.0.0.1%3A8733%2Fselfcheck"
popup="chrome-extension://$id/popup.html?$query"
encoded="$(python3 -c "import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=''))" "$popup")"
curl -sf -X PUT "http://127.0.0.1:$port/json/new?$encoded" \
  | python3 -c "import json,sys; print('opened:', json.load(sys.stdin)['url'][:80])"

status=0
wait "$mimic" || status=$?
kill "$browser" 2>/dev/null || true
wait "$browser" 2>/dev/null || true
echo "selfcheck: $out  profile: $profile  status: $status"
exit "$status"
