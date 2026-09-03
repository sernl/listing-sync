#!/usr/bin/env sh
# The Android toolchain pins, read from the one file that owns them.
#
# `flake.nix` is the source, so a local build and a runner build compile
# against the same NDK. Both Android workflows call this rather than naming a
# version themselves, because a copy in a workflow is a second thing to forget
# when the pin moves.
#
# Usage: android-pins.sh >> "$GITHUB_OUTPUT"
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
flake="$root/flake.nix"

[ -f "$flake" ] || { echo "android-pins: missing $flake" >&2; exit 2; }

ndk=$(sed -n 's/^[[:space:]]*androidNdkVersion[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$flake" | head -1)

if [ -z "$ndk" ]; then
    echo "android-pins: no androidNdkVersion found in $flake" >&2
    exit 1
fi

printf 'ndk=%s\n' "$ndk"
