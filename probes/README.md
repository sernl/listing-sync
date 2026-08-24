# Probes

Experiments that make M-1 decisions cheap. No product code.

- `reachability.sh <url> [ua]` — re-probe marketplace egress reachability; rerun
  before trusting any new host, especially a commercial-datacentre box.
- `session-longevity.sh` — poll an authenticated Tes endpoint over weeks
  (added in Task 9).
- `local/` — gitignored secrets: session cookies, exported HARs, raw captures.

Findings are recorded as working notes under `docs/notes/probes/`, one per probe.
