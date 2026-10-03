# shellcheck shell=bash
set -euo pipefail

# Fetch every hashed asset of the console and the landing page once through
# the public hosts, so Cloudflare's edge holds them before a teacher asks.
#
# A cold edge is what made sign-in slow in 0.19.0: the origin answers every
# chunk in well under a second, but a first load through an edge that held
# none of them stalled on a few for 20-30 s. The assets are content-hashed and
# immutable, so one fetch per deploy is enough to keep the edge warm, and a
# repeat every few minutes keeps the copies from being evicted for idleness.
# Cloudflare caches per data centre: this warms the one the warming machine
# reaches, which for a box in New Zealand is the one New Zealand teachers do.
#
# GET only, sixteen at a time, each to /dev/null. Exits non-zero, after the
# whole list has run, when any asset answers other than 200: an asset the
# edge cannot fetch is one a teacher's browser could not either.
#
# Written into `teachouse-edge-warm` by `nix/edge-warm.nix`, which bakes the
# built console and landing directories in as the defaults; `just edge-warm`
# runs this file directly against the local builds.

usage() {
  cat >&2 <<'EOF'
usage: teachouse-edge-warm [--limit N] [--ui-dir DIR] [--landing-dir DIR] CONSOLE LANDING

  CONSOLE, LANDING  the hosts (or origins) to fetch through, e.g.
                    dash.teachouse.io teachouse.io; a bare host means https://
  --limit N         fetch at most N assets (default: all of them)
  --ui-dir DIR      the built console (default: the one this package was built with)
  --landing-dir DIR the built landing page (default: likewise; none skips it)
EOF
  exit 2
}

ui_dir="${TEACHOUSE_EDGE_WARM_UI_DIR:-}"
landing_dir="${TEACHOUSE_EDGE_WARM_LANDING_DIR:-}"
limit=""
hosts=()
while (($# > 0)); do
  case "$1" in
    --limit)
      [[ $# -ge 2 && "$2" =~ ^[1-9][0-9]*$ ]] || usage
      limit="$2"
      shift 2
      ;;
    --ui-dir)
      [[ $# -ge 2 ]] || usage
      ui_dir="$2"
      shift 2
      ;;
    --landing-dir)
      [[ $# -ge 2 ]] || usage
      landing_dir="$2"
      shift 2
      ;;
    -h | --help) usage ;;
    -*) usage ;;
    *)
      hosts+=("$1")
      shift
      ;;
  esac
done
((${#hosts[@]} == 2)) || usage

origin() {
  case "$1" in
    http://* | https://*) printf '%s' "${1%/}" ;;
    *) printf 'https://%s' "${1%/}" ;;
  esac
}
console="$(origin "${hosts[0]}")"
landing="$(origin "${hosts[1]}")"

if [[ -z "$ui_dir" || ! -d "$ui_dir/_app/immutable" ]]; then
  echo "teachouse-edge-warm: no built console at '${ui_dir}' (want <dir>/_app/immutable)" >&2
  exit 2
fi

# Every file under the two hashed trees, as a URL on the host that serves it.
# Sorted, so `--limit` always takes the same ones.
urls="$(
  {
    (cd "$ui_dir" && find _app/immutable -type f | LC_ALL=C sort | sed "s|^|$console/|")
    if [[ -n "$landing_dir" && -d "$landing_dir/_astro" ]]; then
      (cd "$landing_dir" && find _astro -type f | LC_ALL=C sort | sed "s|^|$landing/|")
    fi
  } | sed -n "1,${limit:-\$}p"
)"
if [[ -z "$urls" ]]; then
  echo "teachouse-edge-warm: nothing to fetch" >&2
  exit 2
fi
total="$(printf '%s\n' "$urls" | wc -l)"

# One curl, its URLs from a config on stdin so the list is never argv. A 5xx
# while the origin is still starting is retried; anything else is reported.
# `--compressed` asks the way a browser does, so the copy the edge keeps is
# the one a browser is sent.
results="$(
  printf '%s\n' "$urls" | sed 's|.*|url = "&"\noutput = "/dev/null"|' |
    curl --config - --parallel --parallel-max 16 --silent --compressed \
      --user-agent teachouse-edge-warm --max-time 60 --retry 3 --retry-delay 2 \
      --write-out '%{http_code} %header{cf-cache-status} %{url}\n' || true
)"

failed="$(printf '%s\n' "$results" | awk '$1 != "200"' || true)"
fetched="$(printf '%s\n' "$results" | grep -c . || true)"
ok="$(printf '%s\n' "$results" | awk '$1 == "200"' | grep -c . || true)"
cache="$(printf '%s\n' "$results" | awk 'NF == 3 { n[$2]++ } END { for (k in n) printf " %s=%d", k, n[k] }')"
echo "teachouse-edge-warm: ${ok} of ${total} assets answered 200 through ${console} and ${landing}${cache:+ (cf-cache-status:${cache})}"
if [[ -n "$failed" || "$fetched" -ne "$total" ]]; then
  echo "teachouse-edge-warm: not every asset answered 200:" >&2
  printf '%s\n' "$failed" | sed '/^$/d' >&2
  exit 1
fi
