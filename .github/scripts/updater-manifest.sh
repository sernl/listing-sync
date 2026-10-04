#!/usr/bin/env bash
# The static updater manifest (`latest.json`) for one release, from the
# artefacts and their minisign signatures.
#
# One definition, two callers: the desktop-release workflow's github-release
# job, over every platform's artefacts at once, and a developer checking a
# local build (`just updater-manifest`). CrabNebula serves the live update
# response; this file is the escape hatch that turns a hosting migration into a
# file copy (docs/notes/design/desktop-distribution.md, "The lock-in escape
# hatch").
#
# Usage: updater-manifest.sh VERSION DIR [BASE_URL] > latest.json
#   VERSION   the release version, without a leading v
#   DIR       a flat directory holding the artefacts and their .sig files
#   BASE_URL  where the artefacts will be served; omitted, every url is a bare
#             filename and `notes` says so
#
# Keys follow tauri-plugin-updater 2.x, which looks up `{os}-{arch}-{bundle}`
# first and `{os}-{arch}` second (plugins/updater/src/updater.rs, get_urls):
#
#   windows-x86_64, windows-x86_64-nsis   the NSIS installer
#   windows-x86_64-msi                    the MSI
#   darwin-aarch64, darwin-x86_64,
#   darwin-aarch64-app, darwin-x86_64-app the universal .app.tar.gz, under both
#                                         architectures, as tauri-action does
#   linux-x86_64, linux-x86_64-appimage   the AppImage
#   linux-x86_64-deb                      the .deb, where the build signed one
#
# The bare `{os}-{arch}` key is the NSIS installer rather than the MSI because
# it is the one the installed base updates from today: the CrabNebula
# `windows-x86_64` update platform carries it, and Tauri's passive install mode
# drives it without an elevation prompt.
#
# The Microsoft Store installer (`*_store-setup.exe`) is never an update: the
# Store updates what it installed. A platform with no artefact in DIR is left
# out rather than failed, so a release whose macOS job was skipped still gets a
# manifest for the rest; the keys written are listed on stderr.
set -euo pipefail

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 VERSION DIR [BASE_URL]" >&2
    exit 2
fi
version=$1
dir=$2
base=${3:-}

command -v jq >/dev/null 2>&1 || {
    echo "updater-manifest: jq is required; run this inside \`nix develop\`" >&2
    exit 2
}
[ -d "$dir" ] || { echo "updater-manifest: $dir is not a directory" >&2; exit 2; }

# The one file in DIR matching a pattern, or nothing. Two matches is an error:
# a stale artefact beside the fresh one would put the wrong signature in the
# manifest, and no rule here can tell which is which.
only() {
    local pattern=$1 exclude=${2:-} found=()
    local candidate
    for candidate in "$dir"/$pattern; do
        [ -f "$candidate" ] || continue
        if [ -n "$exclude" ]; then
            # shellcheck disable=SC2254 # the exclusion is a glob by design
            case "$(basename "$candidate")" in $exclude) continue ;; esac
        fi
        found+=("$candidate")
    done
    if [ "${#found[@]}" -gt 1 ]; then
        echo "updater-manifest: more than one file in $dir matches $pattern: ${found[*]}" >&2
        exit 1
    fi
    if [ "${#found[@]}" -eq 1 ]; then
        printf '%s' "${found[0]}"
    fi
}

platforms='{}'
written=()

# Adds one artefact under each of the given keys. An artefact without its
# signature is an error rather than a skip: createUpdaterArtifacts was on, so a
# missing .sig means the build did not sign, and a manifest without that entry
# would hide it.
add() {
    local file=$1
    shift
    [ -n "$file" ] || return 0
    local sig="$file.sig"
    if [ ! -s "$sig" ]; then
        echo "updater-manifest: $(basename "$file") has no signature beside it at $(basename "$sig")" >&2
        exit 1
    fi
    local name url
    name=$(basename "$file")
    if [ -n "$base" ]; then
        url="${base%/}/$name"
    else
        url="$name"
    fi
    local key
    for key in "$@"; do
        platforms=$(jq --arg key "$key" --arg url "$url" --rawfile sig "$sig" \
            '. + {($key): {signature: ($sig | rtrimstr("\n")), url: $url}}' <<< "$platforms")
        written+=("$key")
    done
}

add "$(only '*_x64-setup.exe' '*_store-setup.exe')" windows-x86_64 windows-x86_64-nsis
add "$(only '*_x64_en-US.msi')" windows-x86_64-msi
add "$(only '*_universal.app.tar.gz')" \
    darwin-aarch64 darwin-x86_64 darwin-aarch64-app darwin-x86_64-app
add "$(only '*_amd64.AppImage')" linux-x86_64 linux-x86_64-appimage
# The .deb is an update only where the build signed it; tauri-bundler 2.9 does,
# and an unsigned one is a download rather than an error.
deb=$(only '*_amd64.deb')
if [ -n "$deb" ] && [ -s "$deb.sig" ]; then
    add "$deb" linux-x86_64-deb
fi

if [ "${#written[@]}" -eq 0 ]; then
    echo "updater-manifest: no updater artefact in $dir" >&2
    exit 1
fi

if [ -n "$base" ]; then
    notes="Fallback manifest. The live endpoint is CrabNebula Cloud, which serves updates dynamically."
else
    notes="Fallback manifest with no host: UPDATER_BASE_URL is unset, so url is a bare filename. The live endpoint is CrabNebula Cloud, which serves updates dynamically."
fi

jq -n --arg v "$version" --arg d "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg n "$notes" \
    --argjson platforms "$platforms" \
    '{version: $v, notes: $n, pub_date: $d, platforms: $platforms}'

printf 'updater-manifest: wrote %s\n' "${written[*]}" >&2
