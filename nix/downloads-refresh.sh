#!/usr/bin/env bash
# The download mirror's refresh: read the newest release, fetch each platform's
# files, verify every one against the release's own SHA256SUMS by name, and
# publish them with `downloads.json` beside them.
#
# `nix/module-downloads.nix` runs this as `teachouse-downloads-refresh` on a
# timer, configured through the environment below. The same file runs by hand
# against a directory of local artefacts, which is how a release's manifest is
# checked before anything is tagged:
#
#   DOWNLOADS_DIR=/tmp/mirror DOWNLOADS_LOCAL_RELEASE=./artefacts \
#   DOWNLOADS_LOCAL_TAG=v0.21.0 bash nix/downloads-refresh.sh
#
# Environment:
#   DOWNLOADS_DIR                  required; the served directory
#   DOWNLOADS_REPOSITORY           owner/name whose releases are read
#   DOWNLOADS_TOKEN_FILE           optional; a file holding a GitHub token with
#                                  read access to releases. Without one the
#                                  API is asked anonymously, which answers for
#                                  a public repository only.
#   DOWNLOADS_PRERELEASE           true to accept a release marked pre-release
#   DOWNLOADS_STORE_GOOGLE_PLAY    optional store listing urls, published in
#   DOWNLOADS_STORE_MICROSOFT      the manifest beside the platform they
#   DOWNLOADS_STORE_MAC_APP_STORE  belong to
#   DOWNLOADS_LOCAL_RELEASE        optional; read release assets from this
#                                  directory instead of GitHub
#   DOWNLOADS_LOCAL_TAG            the tag those local assets belong to
#
# The rules, each of which a previous version of this unit learned the hard
# way:
#
# - Nothing is published that did not verify, and nothing that verified once
#   is removed because something else failed later. Each platform updates its
#   own entry only when every one of its files was fetched and its digest
#   checked by name; a platform that did not is left exactly as it was, the
#   manifest keeps naming it, the cleanup keeps it, and the run ends non-zero.
# - A release that carries no file for a platform (its job was skipped for
#   want of a signing secret, say) keeps the previous entry for that platform.
#   Each entry states its own version, so an older macOS build beside a newer
#   Windows one is described truthfully rather than hidden.
# - Staging is `.staging/` inside the served directory: the same filesystem,
#   so the rename out of it is atomic, and a name tam-server refuses, so
#   nothing half-written is ever reachable. `downloads.json` goes last, so no
#   client reads a manifest naming a file that is not there yet.
# - A file already published under the same name and the digest the release
#   records is not fetched again: a refresh every fifteen minutes would
#   otherwise move every installer across the network every fifteen minutes.
set -euo pipefail

target=${DOWNLOADS_DIR:?DOWNLOADS_DIR names the served directory}
repository=${DOWNLOADS_REPOSITORY:-}
token_file=${DOWNLOADS_TOKEN_FILE:-}
prerelease_ok=${DOWNLOADS_PRERELEASE:-false}
local_release=${DOWNLOADS_LOCAL_RELEASE:-}
local_tag=${DOWNLOADS_LOCAL_TAG:-}

[ -d "$target" ] || { echo "$target is not a directory" >&2; exit 1; }

staging="$target/.staging"
rm -rf "$staging"
mkdir -p "$staging"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Two caps rather than one. --max-time bounds the whole operation including
# retries, so a single value big enough for a sixty-megabyte package would also
# let a hung metadata request sit for half an hour. The file cap carries a stall
# check as well: below ten kilobytes a second for a minute the transfer is not
# progressing and the retry is the point.
fetch_json() { curl --fail --silent --show-error --location --max-time 60 --retry 2 "$@"; }
fetch_file() {
    curl --fail --silent --show-error --location --max-time 1800 --retry 2 \
         --speed-limit 10240 --speed-time 60 "$@"
}

manifest="$target/downloads.json"
# A manifest that will not parse yields nothing rather than aborting the run:
# errexit fires on a failed command substitution in an assignment, so without
# this one corrupt file would lock the surface at that file for good.
previous_manifest='{}'
if [ -f "$manifest" ] && jq -e 'type == "object"' "$manifest" >/dev/null 2>&1; then
    previous_manifest="$(cat "$manifest")"
fi

# ---------------------------------------------------------------------------
# The release: its tag, its publication time, and a table of its assets.
# `assets.tsv` holds `name<TAB>source`, where the source is an API url or a
# local path, so everything below is the same for both.

release_tag=""
release_date=""
curl_auth=()

github_release() {
    if [ -z "$repository" ]; then
        echo "DOWNLOADS_REPOSITORY is unset; it names the repository whose releases are mirrored" >&2
        return 1
    fi
    if [ -n "$token_file" ]; then
        # Read on its own line, where errexit fires. Inside a command
        # substitution in an argument position a failure is discarded, which
        # turns an unreadable secret into `Authorization: Bearer ` and a
        # misconfiguration that stays invisible for exactly as long as the
        # repository stays public.
        local token
        token="$(cat "$token_file")" || {
            echo "$token_file could not be read; it must exist and be readable by the refresh account" >&2
            return 1
        }
        if [ -z "$token" ]; then
            echo "$token_file is empty; it holds a GitHub token with read access to releases" >&2
            return 1
        fi
        # A subshell, so the unit's own 0077 is restored after it.
        ( umask 077
          printf 'header = "Authorization: Bearer %s"\n' "$token" > "$work/curl.conf" ) || return 1
        curl_auth=(--config "$work/curl.conf")
    fi

    fetch_json "${curl_auth[@]}" \
               --header "Accept: application/vnd.github+json" \
               --header "X-GitHub-Api-Version: 2022-11-28" \
               --output "$work/releases.json" \
               -- "https://api.github.com/repos/$repository/releases?per_page=20" || {
        echo "the releases of $repository could not be listed; a private repository needs DOWNLOADS_TOKEN_FILE" >&2
        return 1
    }

    # By version rather than by publish date, never a draft, and a prerelease
    # only where configured: the release workflow marks every channelled
    # release a prerelease. Twenty rather than five, so a run of prereleases
    # cannot push every conforming tag off the page.
    local release
    release="$(jq -c --argjson prerelease_ok "$prerelease_ok" '
        [ .[]
          | select(.draft | not)
          | select((.prerelease | not) or $prerelease_ok)
          | select(.tag_name | test("^v[0-9]+\\.[0-9]+\\.[0-9]+$")) ]
        | sort_by(.tag_name | ltrimstr("v") | split(".") | map(tonumber))
        | last // empty
    ' "$work/releases.json")" || return 1
    if [ -z "$release" ]; then
        echo "no release of $repository carries a released vN.N.N tag" >&2
        return 1
    fi
    release_tag="$(jq -r '.tag_name' <<< "$release")"
    release_date="$(jq -r '.published_at // empty' <<< "$release")"
    # The asset API url with an octet-stream Accept is the only way to read an
    # asset of a private release; the browser url is not one.
    jq -r '.assets[] | [.name, .url] | @tsv' <<< "$release" > "$work/assets.tsv"
}

local_release_assets() {
    [ -d "$local_release" ] || { echo "$local_release is not a directory" >&2; return 1; }
    if ! printf '%s' "$local_tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
        echo "DOWNLOADS_LOCAL_TAG must name the release as vN.N.N, not '$local_tag'" >&2
        return 1
    fi
    release_tag="$local_tag"
    release_date="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    find "$local_release" -maxdepth 1 -type f -printf '%f\t%p\n' | sort > "$work/assets.tsv"
}

fetch_asset() {
    local name=$1 destination=$2 source
    source="$(awk -F '\t' -v want="$name" '$1 == want { print $2; exit }' "$work/assets.tsv")"
    [ -n "$source" ] || { echo "the release carries no asset named $name" >&2; return 1; }
    if [ -n "$local_release" ]; then
        cp -f -- "$source" "$destination"
    else
        fetch_file "${curl_auth[@]}" --header "Accept: application/octet-stream" \
                   --output "$destination" -- "$source"
    fi
}

# The first asset whose name matches a glob and not an exclusion, or nothing.
match_asset() {
    local pattern=$1 exclude=${2:-} name
    while IFS=$'\t' read -r name _; do
        # shellcheck disable=SC2254 # both arguments are globs by design
        case "$name" in $pattern) ;; *) continue ;; esac
        if [ -n "$exclude" ]; then
            # shellcheck disable=SC2254
            case "$name" in $exclude) continue ;; esac
        fi
        printf '%s' "$name"
        return 0
    done < "$work/assets.tsv"
}

# ---------------------------------------------------------------------------
# Verification. Every SHA256SUMS*.txt the release carries is read as one table:
# a release from before 0.21.0 split them per platform, and from 0.21.0 the
# workflow writes one. A name is looked up exactly.
#
# By name rather than through `sha256sum --check --ignore-missing`, which
# asserts only that *something* named in the file was present and matched.
# sha256sum writes `HASH  NAME` in text mode and `HASH *NAME` in binary mode;
# the name starts at column 67 either way, and a name written as `./NAME` is
# read as NAME, which is how every release from v0.1.1 to v0.4.0 wrote it.

collect_sums() {
    : > "$work/sums"
    local name
    while IFS=$'\t' read -r name _; do
        case "$name" in SHA256SUMS*.txt) ;; *) continue ;; esac
        fetch_asset "$name" "$work/$name" || return 1
        cat "$work/$name" >> "$work/sums"
    done < "$work/assets.tsv"
    if [ ! -s "$work/sums" ]; then
        echo "release $release_tag carries no SHA256SUMS file; nothing can be verified, so nothing is published" >&2
        return 1
    fi
}

expected_digest() {
    gawk -v want="$1" '
        length($0) >= 67 && !found {
            name = substr($0, 67)
            sub(/^\*/, "", name)
            sub(/^\.\//, "", name)
            if (name == want) { print substr($0, 1, 64); found = 1 }
        }
        END { exit(found ? 0 : 1) }
    ' "$work/sums"
}

# ---------------------------------------------------------------------------
# One platform. Its spec is a list of `kind|glob|exclusion`, primary first: the
# primary is the file the download button offers, and a release without one
# keeps the previous entry. Prints the new entry as JSON on success; a failure
# prints nothing and returns non-zero, and the caller keeps the old entry.

# Platforms run in command substitutions, which are subshells, so what each
# staged is recorded in a file rather than an array, and only once the whole
# platform verified: half a platform is never renamed into place.
: > "$work/staged"

refresh_platform() {
    local platform=$1
    shift
    local primary_glob primary_exclude
    IFS='|' read -r _ primary_glob primary_exclude <<< "$1"
    if [ -z "$(match_asset "$primary_glob" "$primary_exclude")" ]; then
        echo "$platform: release $release_tag carries no $primary_glob; the previous entry stays" >&2
        return 2
    fi

    local files='[]' fetched=() spec kind glob exclude name digest size
    for spec in "$@"; do
        IFS='|' read -r kind glob exclude <<< "$spec"
        name="$(match_asset "$glob" "$exclude")"
        [ -n "$name" ] || continue
        if ! digest="$(expected_digest "$name")"; then
            echo "$platform: the release's sums carry no digest line for $name" >&2
            return 1
        fi
        if [ -f "$target/$name" ] &&
           printf '%s  %s\n' "$digest" "$target/$name" | sha256sum --check --strict --status -; then
            # Already published under this name and still intact: nothing to move.
            size="$(stat -c %s "$target/$name")"
        else
            fetch_asset "$name" "$staging/$name" || return 1
            if ! ( cd "$staging" && printf '%s  %s\n' "$digest" "$name" | sha256sum --check --strict --status - ); then
                echo "$platform: $name does not match the digest the release records" >&2
                return 1
            fi
            size="$(stat -c %s "$staging/$name")"
            fetched+=("$name")
        fi
        files="$(jq -c --arg k "$kind" --arg n "$name" --arg d "$digest" --argjson s "$size" \
            '. + [{kind: $k, file: $n, sha256: $d, size: $s}]' <<< "$files")"
    done

    if [ "${#fetched[@]}" -gt 0 ]; then printf '%s\n' "${fetched[@]}" >> "$work/staged"; fi
    jq -cn --arg v "${release_tag#v}" --arg u "$release_date" --argjson files "$files" \
        '{version: $v, updated: $u, files: $files}'
}

# The Microsoft Store's installer is not a download for sellers: Partner Center
# fetches it from the mirror by URL at certification and again on every Store
# install, so the URL of the live submission must keep answering while the next
# one is certified. The current one and the one before it are kept.
refresh_msstore() {
    local name digest
    name="$(match_asset '*_store-setup.exe')"
    [ -n "$name" ] || return 2
    digest="$(expected_digest "$name")" || {
        echo "msstore: the release's sums carry no digest line for $name" >&2
        return 1
    }
    if [ ! -f "$target/$name" ] ||
       ! printf '%s  %s\n' "$digest" "$target/$name" | sha256sum --check --strict --status -; then
        fetch_asset "$name" "$staging/$name" || return 1
        ( cd "$staging" && printf '%s  %s\n' "$digest" "$name" | sha256sum --check --strict --status - ) || {
            echo "msstore: $name does not match the digest the release records" >&2
            return 1
        }
        printf '%s\n' "$name" >> "$work/staged"
    fi
    jq -c --arg n "$name" '[$n] + ((.msstore_installers // []) | map(select(. != $n)) | .[:1])' \
        <<< "$previous_manifest"
}

# ---------------------------------------------------------------------------

degraded=0
declare -A entry
for platform in windows macos linux android; do
    # A manifest from before schema 2 named one file per platform at the top
    # of its entry; it is read as that file alone, so a platform this run
    # cannot refresh keeps serving it.
    entry[$platform]="$(jq -c --arg p "$platform" '
        (.version // null) as $top
        | .[$p] // null
        | if . == null then null
          elif has("files") then del(.store)
          else {version: (.version // $top), updated: null,
                files: [{kind: (.file | ascii_downcase | capture("\\.(?<x>[a-z]+)$").x),
                         file, sha256, size: null}]}
          end
    ' <<< "$previous_manifest")"
done
msstore_installers="$(jq -c '.msstore_installers // []' <<< "$previous_manifest")"

if [ -n "$local_release" ]; then
    source_ok=local_release_assets
else
    source_ok=github_release
fi

if "$source_ok" && collect_sums; then
    echo "reading release $release_tag"
    for platform in windows macos linux android; do
        case "$platform" in
            windows) specs=("exe|*_x64-setup.exe|*_store-setup.exe" "msi|*_x64_en-US.msi|") ;;
            macos)   specs=("dmg|*_universal.dmg|") ;;
            linux)   specs=("appimage|*.AppImage|" "deb|*.deb|") ;;
            android) specs=("apk|*.apk|") ;;
        esac
        status=0
        fresh="$(refresh_platform "$platform" "${specs[@]}")" || status=$?
        if [ "$status" -eq 0 ]; then
            entry[$platform]="$fresh"
            echo "$platform: $(jq -r '[.files[].file] | join(", ")' <<< "$fresh")"
        elif [ "$status" -ne 2 ]; then
            echo "$platform: did not refresh; the previous entry keeps serving" >&2
            degraded=1
        fi
    done
    status=0
    fresh="$(refresh_msstore)" || status=$?
    if [ "$status" -eq 0 ]; then
        msstore_installers="$fresh"
    elif [ "$status" -ne 2 ]; then
        echo "msstore: did not refresh; the previous installers keep serving" >&2
        degraded=1
    fi
else
    echo "no release could be read; every previous entry keeps serving" >&2
    degraded=1
fi

stores="$(jq -n \
    --arg windows "${DOWNLOADS_STORE_MICROSOFT:-}" \
    --arg macos "${DOWNLOADS_STORE_MAC_APP_STORE:-}" \
    --arg android "${DOWNLOADS_STORE_GOOGLE_PLAY:-}" \
    '{windows: $windows, macos: $macos, linux: "", android: $android}')"

next="$(jq -n \
    --argjson windows "${entry[windows]}" \
    --argjson macos "${entry[macos]}" \
    --argjson linux "${entry[linux]}" \
    --argjson android "${entry[android]}" \
    --argjson stores "$stores" \
    --argjson msstore "$msstore_installers" '
    def with_store($p; $e):
        ($stores[$p]) as $s
        | if $e == null and $s == "" then null
          elif $e == null then {version: null, updated: null, files: [], store: $s}
          else $e + {store: (if $s == "" then null else $s end)} end;
    {
      schema: 2,
      version: ([$windows, $macos, $linux, $android]
                | map(select(. != null) | .version)
                | sort_by(split(".") | map(tonumber)) | last // null),
      windows: with_store("windows"; $windows),
      macos: with_store("macos"; $macos),
      linux: with_store("linux"; $linux),
      android: with_store("android"; $android),
      msstore_installers: $msstore
    }')"

if [ "$(jq -c '.version' <<< "$next")" = "null" ]; then
    echo "nothing has ever been published here and this run published nothing" >&2
    rm -rf "$staging"
    exit 1
fi

# `refreshed_at` is when the published set last changed, so a run that changed
# nothing leaves the manifest alone rather than restamping a week-old set as
# fresh — the one field a reader would use to decide the surface is stale.
if [ "$(jq -c 'del(.refreshed_at)' <<< "$previous_manifest")" = "$(jq -c . <<< "$next")" ] &&
   [ ! -s "$work/staged" ]; then
    echo "nothing changed; the published set and its manifest are left as they are"
    rm -rf "$staging"
    exit "$degraded"
fi

jq --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" '. + {refreshed_at: $at}' <<< "$next" \
    > "$staging/downloads.json"

# Renamed inside the target directory, so no client sees a partial file, and
# the manifest last. The mode is stated rather than inherited: the unit's umask
# is 0077 and tam-server reads these as a different account.
publish() {
    chmod 0644 "$staging/$1"
    mv -f "$staging/$1" "$target/$1"
}
while IFS= read -r file; do publish "$file"; done < "$work/staged"
publish downloads.json

# Only now, and only names nothing published still refers to. The predicate is
# `! -type d`, so a symlink planted here is removed rather than skipped over.
mapfile -t keep < <(jq -r '
    "downloads.json",
    ([.windows, .macos, .linux, .android][] | select(. != null) | .files[].file),
    .msstore_installers[]
' "$target/downloads.json")
find "$target" -maxdepth 1 ! -type d -printf '%f\0' |
    while IFS= read -r -d "" existing; do
        keeping=0
        for name in "${keep[@]}"; do
            if [ "$existing" = "$name" ]; then keeping=1; break; fi
        done
        if [ "$keeping" -eq 0 ]; then rm -f -- "$target/$existing"; fi
    done

rm -rf "$staging"
echo "published: ${keep[*]}"

if [ "$degraded" -ne 0 ]; then
    echo "one or more platforms did not refresh; their previous files keep serving" >&2
    exit 1
fi
