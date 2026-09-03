#!/usr/bin/env bash
# Loads the built shell in headless Chromium and reports what the browser made
# of it: whether the MV3 service worker registered from a manifest carrying
# both background keys, and whether the popup rendered a version out of the
# wasm module under the extension-pages CSP. Profiles are left under /tmp for
# inspection; nothing here deletes.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
dist="$here/dist"
port="${PORT:-9333}"
profile="$(mktemp -d /tmp/tam-ext-verify.XXXXXX)"

if [ ! -f "$dist/manifest.json" ]; then
    echo "build it first: just extension-dev" >&2
    exit 1
fi

# Chrome derives an unpacked extension's id from the SHA-256 of its absolute
# directory path, one hex digit per character mapped onto a-p. Deriving it
# beats scraping the target list, which also carries Chrome's own component
# extensions.
id="$(python3 - "$dist" <<'PY'
import hashlib, os, sys
digest = hashlib.sha256(os.path.abspath(sys.argv[1]).encode()).hexdigest()[:32]
print("".join(chr(ord("a") + int(c, 16)) for c in digest))
PY
)"

# Each invocation gets its own profile: Chromium refuses a user-data-dir that
# a previous process left locked, which a killed browser always does.
chromium_run() {
    nix shell nixpkgs#chromium --command chromium \
        --headless=new --no-first-run --no-default-browser-check --disable-gpu \
        --user-data-dir="$(mktemp -d "$profile/run.XXXXXX")" \
        --disable-extensions-except="$dist" --load-extension="$dist" "$@"
}

# `nix shell --command` puts a wrapper between us and the browser, so killing
# the job leaves chromium holding the debugging port. Every process carries the
# unique profile path in its argv, which is what makes it matchable.
cleanup() { pkill -f "$profile" 2>/dev/null || true; }
trap cleanup EXIT

chromium_run --remote-debugging-port="$port" about:blank >/dev/null 2>&1 &

worker=""
for _ in $(seq 1 40); do
    sleep 0.5
    worker="$(curl -s "http://127.0.0.1:$port/json/list" 2>/dev/null \
        | grep -o "chrome-extension://$id/background.js" | head -1 || true)"
    if [ -n "$worker" ]; then break; fi
done

cleanup
sleep 1

echo "chromium:       $(nix shell nixpkgs#chromium --command chromium --version 2>/dev/null)"
echo "extension id:   $id"
if [ -n "$worker" ]; then
    echo "service worker: registered at $worker"
else
    echo "service worker: ABSENT — the manifest's background keys were not accepted" >&2
fi

dom="$(chromium_run --virtual-time-budget=8000 --dump-dom \
    "chrome-extension://$id/popup.html" 2>/dev/null)"
echo "popup rendered: $(printf '%s' "$dom" | grep -o '<p id="state">[^<]*</p>')"
echo "profile:        $profile"

test -n "$worker"
