# Teachouse infrastructure cost evidence

Measured 2026-09-20 ≈11:11 UTC (thunderstorm `btime` 1789122095 + uptime 780584 s = epoch 1789902679). All figures read-only over ssh from `/proc`, cgroup v2 and on-disk layout; manifest figures from the dotfiles repo.

**Tooling limits on this pass (stated, not worked around):** this agent has no shell tool — only file read/grep/glob and web search. Therefore: no `k3s kubectl top`, no `psql` row counts, no `df`/`du`. Node and pod utilisation were obtained from `/proc/stat`, `/proc/meminfo` and `kubepods*.slice/{memory.current,cpu.stat}`, which is equivalent to (and finer than) `kubectl top`. **DB row counts, per-org counts, total blob bytes from the `files` table and job-table trial timestamps are NOT in this report** — they need the psql path in the task brief, which requires a shell. Everything below that touches those quantities is marked `[INFERENCE]` and derived from on-disk evidence instead.

## Node specs and role

| Node | LAN / mesh | CPU | Threads | RAM (MemTotal) | Disk layout | Role |
|---|---|---|---|---|---|---|
| elnino | 192.168.50.10 / 10.147.21.5 | AMD Ryzen 3 2200G (4C/4T, 65 W TDP) | 4 | 7,818,496 kB = 7.46 GiB | single ext4 root + swap | K3s API server (`serverAddr https://10.147.21.5:6443`), Garage node, tam-server replica |
| thunderstorm | 192.168.50.13 / 10.147.21.2 | Intel i7-8700 (6C/12T, 65 W TDP) | 12 | 16,210,004 kB = 15.46 GiB | nvme0n1 (root/k3s) + sda (NAS) + zram0; ZFS host | CNPG affinity target (tam-1, tam-3), tam-server, tam-worker, tam-auth, Garage, Filen mirror, media stack |
| sundog | 10.147.21.3 | Intel i5-2450M (2C/4T, 35 W TDP, laptop) | 4 | 8,015,176 kB = 7.64 GiB | single ext4 root + swap | tam-2 (Postgres), tam-worker, Garage, Argo CD app-controller |

## Current utilisation

CPU, averaged over the whole boot window from `/proc/stat` (busy = total − idle − iowait):

| Node | Uptime | Busy | Mean cores used | Notes |
|---|---|---|---|---|
| thunderstorm | 780,584 s (9.03 d) | 2.88 % of 12 threads | 0.35 | load1 0.01 |
| elnino | 523,830 s (6.06 d) | 2.57 % of 4 threads | 0.10 | load1 0.98, **iowait 10.8 %** — disk-bound, not CPU-bound |
| sundog | 5,956,177 s (68.9 d) | 0.77 % of 4 threads | 0.03 | — |

Memory headroom right now: thunderstorm MemAvailable 8.29 GiB of 15.46; elnino 2.76 GiB of 7.46 (SwapCached 348 MiB — it has been swapping); sundog 2.78 GiB of 7.64 (SwapCached 124 MiB).

All Kubernetes pods on a node (`kubepods.slice/memory.current`): thunderstorm 810 MiB, elnino 397 MiB.

Per-pod, from each pod's cgroup (`memory.current`, `cpu.stat usage_usec`):

| Pod | Node | QoS | RSS | CPU-seconds consumed | Mean cores |
|---|---|---|---|---|---|
| tam-1 (postgres) | thunderstorm | BestEffort | 297.2 MiB | 2,891.6 | ≥ 0.0037 (≤9.03 d window) |
| tam-3 (postgres) | thunderstorm | BestEffort | 226.2 MiB | 1,548.8 | ≥ 0.0020 |
| tam-2 (postgres) | sundog | BestEffort | 475.8 MiB | 3,043.2 | ≥ 0.0005 (≤68.9 d window) |
| tam-server | thunderstorm | Burstable | 36.6 MiB | 14.11 | 0.00027 |
| tam-server | elnino | Burstable | 50.4 MiB | 15.85 | 0.00030 |
| tam-worker | thunderstorm | Burstable | 4.6 MiB | 8.64 | 0.00016 |
| tam-worker | sundog | Burstable | 10.0 MiB | 16.48 | 0.00031 |
| tam-auth | thunderstorm | Burstable | 96.2 MiB | 16.82 | 0.00032 |
| **total** | | | **1.17 GiB** | | **≈0.008 cores** |

Stateless pod ages are exact: all restarted 2026-09-20T08:31:15+12:00, i.e. 52,804 s (14.67 h) before measurement, so their mean-core figures are hard numbers. Postgres pods predate that; their mean-core figures are lower bounds (they can only be higher).

Declared, not measured: every Teachouse container requests `cpu 100m / memory 256Mi` and is limited to `500m / 256Mi`; five stateless pods ⇒ 500m CPU and 1,280 MiB memory *requested* against 20 threads and 30.5 GiB. The three Postgres pods declare **no** requests or limits (BestEffort) — they are the first eviction victims under node memory pressure.

## Storage

- **Postgres**: CNPG `Cluster tam`, 3 instances, `storageClass local-path`, `size 40Gi` each ⇒ 120 GiB provisioned, node-local, one copy per node. On disk: `/var/lib/rancher/k3s/storage/pvc-…_teachouse_tam-1/pgdata` and `…_tam-3` on thunderstorm; tam-2's volume on sundog. Actual bytes used could not be read without `du`/`df`.
- **Blobs (Garage)**: `/var/lib/garage/data` on thunderstorm holds 136 occupied first-level fan-out directories of 256. Sampled leaves: `02/` → 1 subdir → 1 block file (`…​.zst`), `fe/` → 1 subdir, `a2/` → 2 subdirs. That is ≈180 block files; Garage's default block size is 1 MiB and blocks are zstd-compressed, so **the live blob set is on the order of 0.15–0.2 GiB per replica, ×3 replicas** `[INFERENCE — extrapolated from a 3-directory sample, not a du]`. Garage layout capacity is 200 G per node.
- **Consequence worth pricing on:** the trial catalogue's payload bundles are ~15.4–15.8 MB each (`teachouse-first-thirty-catalogue.json` `observed_byte_len` 15,424,174 and 15,779,029). 154 of those would be ≈2.4 GB, an order of magnitude more than the entire Garage store. So **bundle payloads are not sitting on Teachouse's servers** — they are held natively on the seller's device (`teachouse-first-thirty-native-holdings.json` records hash/bytes/locator per product) and only small sealed objects (covers, canonical payloads ~98 kB) land in Garage. Server-side storage cost per resource is therefore ~1 MiB, not ~15 MiB.
- Disk I/O since boot on thunderstorm's NVMe: 1.18 TB written (2,306,096,024 sectors × 512 B) over 9 days — dominated by the media/ZFS workload, not Teachouse.

## DB row counts

Not collected — requires the psql exec path, which requires a shell this agent does not have. The corroborating artefacts show the trial catalogue size: `"discovered": 154` per enumeration (`teachouse-phone-rules-proof.json:91,119`), and 30-resource evidence files carrying per-file `byte_len`/`hash`/`role` rows (`teachouse-first-thirty-catalogue.json`). Treat 154 products / 1 seller org as the measured working set and re-run the row counts with a shell before pricing on them.

## Per-resource cost

- **Stored bytes:** ≈1.2 MiB per resource server-side ×3 replicas ≈ 3.6 MiB `[INFERENCE, from the Garage block estimate ÷ 154 resources]`. At 40Gi × 3 provisioned Postgres and 200 G × 3 Garage capacity, the current catalogue uses well under 1 % of provisioned storage.
- **Worker CPU:** both tam-worker replicas together consumed 25.1 CPU-seconds in 14.67 h of wall clock. If the 154-create and 154-publish runs fall inside that window, that is **≤0.08 CPU-seconds per marketplace operation** `[INFERENCE — I cannot confirm the runs are in-window without the job tables]`. Either way the worker's steady-state draw is 0.5 millicores; it spends its life waiting on the marketplaces, not computing. The worker's own log line confirms the duty cycle: `maintenance pass every 5000ms`.

## Measured throughput of the trial runs

Using the run figures given in the brief (154 creates ≈62 min, 154 publishes ≈26 min); the job-table timestamps that would let me verify them need psql.

| Run | Items | Wall | Per item | Rate |
|---|---|---|---|---|
| Create | 154 | ~3,720 s | 24.2 s | 2.48/min |
| Publish | 154 | ~1,560 s | 10.1 s | 5.92/min |
| Both | 308 | ~5,280 s | 17.1 s | 3.50/min |

Compare with worker CPU: ≤25 CPU-seconds for ~5,280 s of wall clock across 308 operations ⇒ **CPU utilisation during the trials was under 0.5 %.** Wall time is entirely marketplace round-trip latency and politeness pacing. Scaling throughput is a concurrency/rate-limit question against TPT and Tes, not a hardware question — which is the single most important fact for pricing: the unit cost of a create or a publish is essentially zero in compute, and whatever it costs is the marketplace's rate limit and the seller's session.

## Monthly running cost

UK electricity, Ofgem cap 1 Oct–31 Dec 2026: **26.32 p/kWh** electricity unit rate, 54.83 p/day standing charge, direct debit, England/Scotland/Wales average; no VAT on electricity 1 Oct 2026 – 31 Mar 2027. The standing charge is excluded below — the household pays it whether or not the cluster runs. *Caveat:* all three nodes report `+12:00` local time and the Garage zones are named `auckland-*`, so the hardware may not be on a UK tariff; the Ofgem rate was requested and is used, but re-check the tariff if the machines are in NZ `[INFERENCE]`.

Power draw `[INFERENCE — no PDU/IPMI reading available; from CPU TDP class, node role and measured utilisation]`:

| Node | Idle | At current load | Basis |
|---|---|---|---|
| elnino | ~28 W | ~30 W | 65 W-TDP desktop APU at 2.6 % busy, one SSD |
| thunderstorm | ~48 W | ~55 W | 65 W-TDP desktop + NVMe + spinning NAS disk, 2.9 % busy |
| sundog | ~13 W | ~15 W | 35 W-TDP mobile CPU, laptop chassis, 0.8 % busy |
| **cluster** | ~89 W | **~100 W** | |

- 100 W × 730 h = **73 kWh/month ⇒ £19.2/month** for the whole homelab.
- **Teachouse's marginal share at current load:** Teachouse pods account for ≈0.008 of 20 threads (0.04 %). A loaded core on these parts costs roughly +10 W, so the compute delta is ~0.1 W; adding memory residency and PVC/Garage I/O gives ~2 W ⇒ **≈£0.4/month**, call it **under £1/month**. Fully-allocated (if the cluster existed only for Teachouse) it is the full **£19/month**.
- **10× load** (≈1,540 resources, ~10 sellers): CPU ≈0.08 cores, blobs ≈2 GiB/replica, Postgres well inside 40Gi. No hardware change; power +~2 W ⇒ **≈£19.6/month fully-allocated, ≈£1/month marginal**.
- **100× load** (≈15,400 resources, ~100 sellers): Postgres ≈0.4–0.8 cores sustained, blobs ≈18 GiB/replica (still under the 40Gi PVC and far under Garage's 200 G), worker concurrency up to whatever the marketplaces tolerate. CPU is still under 5 % of the cluster; **RAM becomes the binding constraint before CPU or disk**. Assume +30 W for the extra memory and sustained activity ⇒ **≈£25/month fully-allocated**. Assumptions: linear bytes-per-resource; no change in per-op marketplace pacing; no analytics/AI inference moved on-cluster (AI metadata fills are a third-party API cost, not measured here).

Interpretation for pricing: infra is not the cost driver at any load this business is likely to see in 12 months. £19–£25/month of electricity covers a four-figure seller count. The real marginal costs are per-seller marketplace rate limits, AI API calls, and support.

## Biggest capacity ceiling

**RAM on the two 8 GB nodes.** elnino has 2.76 GiB and sundog 2.78 GiB available of ~7.5 GiB each, and both are already swapping. The three Postgres replicas are BestEffort — no requests, no limits — so they are evicted first when a node runs short, and the CNPG cluster's Postgres tuning is explicitly annotated `Sized for a 8 GiB node, which is the smallest of the three` with `shared_buffers 512MB` and `max_connections 200`. 200 connections × ~8 MB `work_mem` is 1.6 GiB of potential per-query memory on a node with 2.8 GiB free: the first real concurrency spike, not the first storage growth, is what breaks this cluster. CPU is at 0.8–2.9 %, disk is at a few percent of 120 GiB provisioned + 600 G Garage capacity — neither is close.

Second ceiling, worth naming: the household uplink. A single catalogue import moves ~2.4 GB of bundles (154 × ~15.5 MB), and while today those stay on the seller's device, any feature that routes bundles through the server turns a residential upstream into the throughput limit `[INFERENCE — uplink speed not measured]`.

## Sources

- `modules/kubernetes/applications/teachouse.nix:126-140` (container requests 100m/256Mi, limits 500m/256Mi), `:46-52` (Garage endpoints, port 3900, bucket `teachouse`), `:212-215` (tam-server replicas = 2), `:366-372` (tam-worker replicas = 2), `:404-408` (tam-auth replicas = 1).
- `modules/kubernetes/clusters/elnino-pilot.nix:88-95` (instances 3, local-path, 40Gi), `:117-128` (shared_buffers 512MB, max_connections 200, work_mem 8MB, `Sized for a 8 GiB node`), `:98-114` (affinity toward thunderstorm).
- `modules/nixos/server/garage/default.nix:116-128` (200G capacity, zones), `:167-170` (replication_factor 3, lmdb, /var/lib/garage/{meta,data}).
- `modules/hosts/elnino/_nixos/hardware.nix:24-38`, `modules/hosts/sundog/_nixos/hardware.nix:26-40`, `modules/hosts/thunderstorm/_nixos/disko.nix` (single ext4 root + swap per node).
- `modules/hosts/{elnino,sundog,thunderstorm}/_nixos/default.nix` (mesh IPs 10.147.21.5 / .3 / .2, k3s roles, `garage.enable`).
- ssh root@192.168.50.13: `/proc/cpuinfo` (i7-8700, 6C/12T), `/proc/meminfo` (MemTotal 16210004 kB), `/proc/stat` (cpu totals, btime 1789122095), `/proc/uptime` (780584.26), `/proc/loadavg` (0.01), `/proc/diskstats` (nvme0n1 2306096024 sectors written), `/var/log/pods/` (pod inventory + UIDs), `/sys/fs/cgroup/kubepods.slice/**/{memory.current,cpu.stat}`, `/var/lib/rancher/k3s/storage/` (two PVC dirs), `/var/lib/garage/{meta,data}` (layout, 136 occupied fan-out dirs, sampled leaves).
- ssh root@192.168.50.10: `/proc/cpuinfo` (Ryzen 3 2200G, 4C/4T), `/proc/meminfo` (7818496 kB), `/proc/stat`, `/proc/uptime` (523829.85), `/proc/loadavg` (0.98), `/var/log/pods/`, tam-server pod cgroup.
- ssh root@10.147.21.3: `/proc/cpuinfo` (i5-2450M), `/proc/meminfo` (8015176 kB), `/proc/stat`, `/proc/uptime` (5956177.40), `/var/log/pods/`, tam-2 and tam-worker pod cgroups.
- Container start timestamps: `/var/log/pods/teachouse_tam-worker-…/tam-worker/0.log:1` and `…tam-server…/tam-server/0.log:1` — both `2026-09-20T08:31:1x+12:00`.
- `local://teachouse-phone-rules-proof.json:91,119` (`"discovered": 154`); `local://teachouse-first-thirty-catalogue.json:968,1495` (`observed_byte_len` 15,779,029 / 15,424,174); `local://teachouse-first-thirty-native-holdings.json:87-91` (bundle bytes held natively on device); `local://teachouse-live-canonical-payloads.json:28` (`byte_len 97633`).
- Ofgem, "Changes to energy price cap between 1 October and 31 December 2026" — 26.32 p/kWh electricity unit rate, 54.83 p/day standing charge, England/Scotland/Wales average, direct debit. https://www.ofgem.gov.uk/news/changes-energy-price-cap-between-1-october-and-31-december-2026 (accessed 2026-09-20). VAT-free electricity 1 Oct 2026 – 31 Mar 2027 per https://www.ofgem.gov.uk/information-consumers/energy-advice-households/energy-price-cap-unit-rates-and-standing-charges (accessed 2026-09-20).
