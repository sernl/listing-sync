#!/usr/bin/env sh
# Refuse a release the updater could never serve, or that says nothing.
#
# One definition, two callers: `just release-check` and the desktop-release
# workflow's verify job. The workflow runs before anything is built, so a
# mismatch costs seconds rather than a Windows runner's worth of minutes.
#
# Usage: release-check.sh [tag]
#   with a tag   compare it against the tree as well as the tree against itself
#   without one  check the tree only
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
conf="$root/apps/desktop/src-tauri/tauri.conf.json"
manifest="$root/apps/desktop/src-tauri/Cargo.toml"
tag=${1:-}
failed=0

fail() {
    echo "release-check: $1" >&2
    failed=1
}

for f in "$conf" "$manifest"; do
    [ -f "$f" ] || { echo "release-check: missing $f" >&2; exit 2; }
done

command -v jq >/dev/null 2>&1 || {
    echo "release-check: jq is required; run this inside \`nix develop\`" >&2
    exit 2
}

conf_version=$(jq -r '.version // empty' "$conf")
# The first `version` under [package]; awk rather than sed because the file has
# several tables and only the first one may answer.
manifest_version=$(awk '
    /^\[package\]/ { in_pkg = 1; next }
    /^\[/          { in_pkg = 0 }
    in_pkg && /^[[:space:]]*version[[:space:]]*=/ {
        gsub(/^[^"]*"|".*$/, ""); print; exit
    }
' "$manifest")

[ -n "$conf_version" ] || fail "no version found in tauri.conf.json"
[ -n "$manifest_version" ] || fail "no [package] version found in Cargo.toml"

if [ -n "$conf_version" ] && [ "$conf_version" != "$manifest_version" ]; then
    fail "version mismatch: tauri.conf.json says $conf_version, Cargo.toml says $manifest_version"
fi

if [ -n "$tag" ]; then
    tag_version=${tag#v}
    if [ "$tag_version" != "$conf_version" ]; then
        fail "tag $tag does not match tauri.conf.json version $conf_version"
    fi
fi

# A release with no notes is a release nobody can read. The notes are written
# for a seller rather than derived from commits, so they cannot be generated
# here and their absence has to be an error rather than an empty string.
if [ -n "$conf_version" ]; then
    notes="$root/docs/releases/$conf_version.md"
    if [ ! -f "$notes" ]; then
        fail "no release notes at docs/releases/$conf_version.md"
    elif [ -z "$(tr -d '[:space:]' < "$notes")" ]; then
        fail "release notes at docs/releases/$conf_version.md are empty"
    fi
fi

# The updater refuses an unsigned update and cannot be told otherwise, so a
# release built against a placeholder key ships an app whose updates can never
# verify. Same for the endpoint: ORG/APP resolves to nothing.
if grep -q 'PLACEHOLDER_FOUNDER_SUPPLIES_THIS' "$conf"; then
    fail "plugins.updater.pubkey is still the placeholder; see docs/notes/design/desktop-distribution.md"
fi
if grep -q '/update/ORG/APP/' "$conf"; then
    fail "plugins.updater.endpoints still names ORG/APP; see docs/notes/design/desktop-distribution.md"
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi

echo "release-check: version $conf_version, updater key and endpoint set, notes present${tag:+, tag $tag matches}"
