#!/usr/bin/env bash
# Gate G3: one upload run. G3_VARIANT selects offscreen (default) or tab.
# The profile directory is reused across invocations so a second run is a
# genuine browser restart against an already-installed extension.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
label="${G3_LABEL:-run}"
profile="${G3_PROFILE:-/tmp/g3-profile}"
out="${G3_OUT:-/tmp/g3-$label.json}"
mem="${G3_MEM:-/tmp/g3-$label-mem.json}"
mkdir -p "$profile"

G3_OUT="$out" G3_LABEL="$label" nix shell nixpkgs#python3 --command \
  python3 "$here/servers/g3-sink.py" &
sink=$!
sleep 1

stop="$(mktemp -u /tmp/g3-stop.XXXXXX)"
MEM_STOP="$stop" MEM_DURATION="${G3_TIMEOUT:-600}" nix shell nixpkgs#python3 --command \
  python3 "$here/servers/mem-sample.py" "$profile" "$mem" &
sampler=$!

nix shell nixpkgs#chromium --command chromium \
  "${CHROME_MODE:---headless=new}" \
  --no-first-run --no-default-browser-check --disable-gpu \
  --user-data-dir="$profile" \
  --disable-extensions-except="$here/g3-payload" \
  --load-extension="$here/g3-payload" \
  about:blank &
browser=$!

status=0
wait "$sink" || status=$?
: > "$stop"
kill "$browser" 2>/dev/null || true
wait "$sampler" 2>/dev/null || true
wait "$browser" 2>/dev/null || true
echo "result: $out  memory: $mem  status: $status"
exit "$status"
