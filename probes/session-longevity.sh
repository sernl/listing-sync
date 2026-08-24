#!/usr/bin/env bash
# Poll an authenticated Tes endpoint over time and classify session state.
# Refresh-aware: uses a cookie jar with -b/-c so server Set-Cookie refreshes persist,
# which measures whether a captured server-side session can be kept alive.
# Requires probes/local/tes-cookies.jar (Netscape format, gitignored).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
jar="$here/local/tes-cookies.jar"
log="$here/local/session-longevity.log"
probe_url="${TES_PROBE_URL:-https://www.tes.com/api/tier/gmv/me}"
refresh_url="https://www.tes.com/api/authn/refresh-cookies"
[ -f "$jar" ] || { echo "missing $jar" >&2; exit 1; }
curl -sS -o /dev/null --max-time 25 -b "$jar" -c "$jar" -A "Mozilla/5.0" "$refresh_url" || true
code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 25 -b "$jar" -c "$jar" -A "Mozilla/5.0" "$probe_url" || echo 000)"
eff="$(curl -sS -o /dev/null -w '%{url_effective}' -L --max-time 25 -b "$jar" -c "$jar" -A "Mozilla/5.0" "$probe_url" || echo "")"
state="authed"
case "$code" in 401|403) state="expired";; 000) state="neterror";; esac
case "$eff" in *login*|*sign-in*|*otp*) state="challenged";; esac
printf '%s\t%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$code" "$state" | tee -a "$log"
