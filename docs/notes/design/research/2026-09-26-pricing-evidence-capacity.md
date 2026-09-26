# Pricing evidence: where the work runs, what it costs, and what one tenant can do to another

- date: 2026-09-26
- companion to `2026-09-26-pricing-re-evaluation.md`; extends and corrects `2026-09-20-pricing-evidence-infra.md`
- method: live reads on 2026-09-26 ≈05:10 UTC over ssh to the three nodes (`k3s kubectl top`, `kubectl get pods`, `free -m`, `du`, `df`, `psql` inside the CNPG primary `tam-1`), plus code reads at the cited `file:line` on branch `program/0.12-clarity` (identical to `main` for every file cited). Anything not measured is marked `[INFERENCE]`.

## 1. Where each unit of work runs

| Work | Runs on | Evidence |
|---|---|---|
| Reading a marketplace catalogue (import) | seller's device; the server receives a description and one derived cover per resource, never the bundle | `crates/tam-api/src/import.rs:1-22` (D27: "this route receives a description and never the thing described"); cover stored at `import.rs:571-584` |
| Cover rendering at import | seller's device (`tam-pipeline` linked into the Tauri app) | `apps/desktop/src-tauri/Cargo.toml:160`; brief recon (`render::cover`) |
| Cover redraw on replacement | server, `tam-api` | `crates/tam-api/src/catalogue.rs::redraw_cover` (≈l.2709), which holds the file in memory "bounded by the same `UPLOAD_BODY_BYTES_MAX`" |
| Create / publish / update / delete on TPT and Tes | seller's device, under the seller's session | `apps/desktop/src-tauri/Cargo.toml:146-154` (engine-driver and both adapters); `crates/tam-worker/src/main.rs:12-20` ("every one of those requests now originates on the seller's own device") |
| Claiming that work | server, one SQL statement per poll, **scoped to the caller's own organisation** | `crates/tam-storage/src/jobs.rs:1453-1624`; tenant pin at `:1546`; `transport_class = 'seller_device'` at `:1547` |
| Server-side lease scan (cross-tenant FIFO) | server, but only for `official_api` inventories — Etsy, which has no adapter; today it serves nothing | `jobs.rs:1333-1373` (`transport_class = 'official_api'` at `:1347`); `crates/tam-storage/migrations/0043_transport_class.sql` (tes/tpt = `seller_device`) |
| Lease reaping, park revival, fleet breaker | server, `tam-worker`, every 5 s | `crates/tam-worker/src/main.rs:47,115-164` |
| Scheduler (Sync plan only: `scheduling: true`) | server, `tam-server`, every 60 s | `crates/tam-server/src/main.rs:412,426-457`; `crates/tam-limits/src/lib.rs:151,175` |
| Manual uploads from the web console | server, `tam-api`, whole body in memory | `crates/tam-api/src/catalogue.rs:321-325` (`body: Bytes`), `lib.rs:388` |
| Installer downloads | server, streamed from disk | `crates/tam-server/src/downloads.rs:1-12,109` |
| Storage of covers and small sealed objects | Garage on the three nodes, replication 3 | `modules/nixos/server/garage/default.nix:116-128,167` (dotfiles) |

The desktop app polls for work every 10 s while idle and every 30 s when another of the seller's devices holds the marketplace; it checks in every 5 minutes (`apps/desktop/src-tauri/src/scheduler.rs:182,186,195`). That poll is the only server work that scales with *open apps* rather than with resources.

## 2. Measured, 2026-09-26

### Pods and nodes (`kubectl top`)

| Pod | CPU | Memory |
|---|---|---|
| tam-1 / tam-2 / tam-3 (Postgres, BestEffort) | 7m / 7m / 3m | 222 / 135 / 112 MiB |
| tam-server ×2 | 1m / 1m | 25 / 41 MiB |
| tam-worker ×2 | 1m / 1m | 5 / 4 MiB |
| tam-auth | 1m | 100 MiB |
| **Teachouse total** | **22 millicores** | **≈645 MiB** |

| Node | CPU | RAM used | `free -m` available |
|---|---|---|---|
| elnino (8 GB) | 54m (1%) | 1,718 MiB | 6,105 MiB |
| sundog (8 GB) | 149m (3%) | 1,786 MiB | 5,915 MiB |
| thunderstorm (16 GB) | 137m (1%) | 5,998 MiB | 8,401 MiB |

**Correction to the 09-20 infra pack:** it reported 2.76/2.78 GiB available on the 8 GB nodes. Today both show ~5.9–6.1 GiB available. RAM is still the first thing to run out, but it is not tight today. Postgres pods are still BestEffort (no requests or limits); every Teachouse container requests and is limited to 256Mi (`modules/kubernetes/applications/teachouse.nix:113-122`), and `/tmp` is a memory-backed `emptyDir` that counts against that limit (`:133-143`).

### Database (`psql` on `tam-1`)

- `pg_database_size`: **28 MB**. Tenant tables (77 tables with `org_id`): 13.7 MB. Shared tables: 2.1 MB. The rest is catalog and indexes.
- 4 organisations; products 164 (founder org) + 1. Founder org: 33 import runs, 679 job items, 625 write attempts, 322 mappings.
- **Per-resource DB footprint:** (13.7 MB − ~3 MB fixed overhead for 77 near-empty tables `[INFERENCE]`) ÷ 165 ≈ **65 KB per resource** at the founder's usage (≈4 job items per resource). A Look account, which imports once and makes at most 5 moves, is ≈**25 KB per resource** `[INFERENCE: import_run_item 1.2 KB/row, product+fingerprint+file+terms ≈ 15 KB]`.
- Only `job_event` and metric snapshots are pruned (`crates/tam-storage/src/pruning.rs:54,103`). `import_run_item`, `write_attempt` and `mapping_loss` grow for as long as the account exists.
- PVCs on disk: 597–613 MB each, mostly WAL (`max_wal_size 2GB`, `wal_keep_size 512MB`).
- Connections: 26 backends (`tam_app` 15, `tam_engine` 6, `tam_backoffice` 4, …) against `max_connections 200`.
- Transaction rate with no active sellers: 3,058,883 transactions since 2026-09-15 18:34 UTC ≈ **3.4 tx/s** of baseline (maintenance pass, scheduler, outbox, CNPG probes).
- **Cost of one device claim:** a read-only copy of the claim candidate query (`jobs.rs:1490-1624` without the `FOR UPDATE`), run three times on `tam-1` for the founder org: **execution 0.25 ms, planning 4.4 ms**. sqlx reuses prepared statements per connection, so steady-state cost is closer to the execution figure `[INFERENCE]`.

### Blobs and disk

| | elnino | sundog | thunderstorm |
|---|---|---|---|
| `/var/lib/garage/data` | 97 MB | 97 MB | 97 MB |
| root disk free | 820 GB of 907 | **386 GB of 448** | 606 GB of 907 |

- `blob` table: founder org 121 blobs = 85.4 MB (**median 100 KB**, max 31 MB — a manual upload); a second org 9 blobs = 1.8 MB (median 49 KB); a third 38 blobs = 11.5 MB. Garage holds 97 MB per replica for 98.7 MB logical, so compression gains nothing on PNG/PDF.
- Garage layout capacity is 200 G per node with replication 3 (`garage/default.nix:116-128,167`), so **usable capacity is 200 GB**, not 600 GB. Physical disk behind it is ≥386 GB on every node.

### Edge

- `teachouse.io` and `/v1/*` answer `server: cloudflare`, `cf-cache-status: DYNAMIC` (curl, 2026-09-26).
- Cloudflare's maximum request body is **100 MB on Free and Pro**, 200 MB Business, 500 MB Enterprise; larger requests get a 413 at the edge ([developers.cloudflare.com/workers/platform/limits](https://developers.cloudflare.com/workers/platform/limits/), fetched 2026-09-26).
- Cloudflare Free includes **1 rate-limiting rule**, counted by IP only, matching on path and host ([blog.cloudflare.com/unmetered-ratelimiting](https://blog.cloudflare.com/unmetered-ratelimiting/); [developers.cloudflare.com/waf/rate-limiting-rules/parameters](https://developers.cloudflare.com/waf/rate-limiting-rules/parameters/), fetched 2026-09-26).
- No in-process rate limiter or concurrency limit exists in `tam-server`/`tam-api` (no `ConcurrencyLimitLayer`, `governor` or equivalent anywhere under `crates/`).

## 3. Per-account and per-active-seller cost

Assumptions: a 1 TB consumer NVMe at ~US$60 over 36 months gives ≈US$0.0017 per GB-month `[INFERENCE]`; electricity at NZ$0.42/kWh (MBIE national average, May 2026, via [powerbill.co.nz/blog/electricity-price-per-kwh-nz](https://powerbill.co.nz/blog/electricity-price-per-kwh-nz/) `[SECONDARY]`, fetched 2026-09-26). The nodes are in Auckland (`+12:00`, Garage zones `auckland-*`), so the 09-20 pack's Ofgem tariff was the wrong one.

| | Look account, 100 resources, idle | Active seller, 200 resources, app open 3 h/day |
|---|---|---|
| Garage | 100 × 100 KB × 3 = 30 MB (≤150 MB if covers grow to ~500 KB after the Thumbnails work `[INFERENCE]`) | 200 × 100–500 KB × 3 = 60–300 MB |
| Postgres | 100 × 25 KB × 3 instances = 7.5 MB | 200 × 65 KB × 3 = 39 MB |
| Requests | none while the app is closed | 1 claim / 10 s → ~21,600 claims a month |
| DB CPU | 0 | 21,600 × 0.25–4.7 ms = 5–100 CPU-seconds a month |
| **Server cost / month** | **≈US$0.0003** | **≈US$0.001** (disk) + <1 Wh of power |

Whole homelab: ~100 W × 730 h = 73 kWh ≈ **NZ$31 (≈US$18) a month**. Teachouse's share at today's 22 millicores and 645 MiB is ~2 W ≈ **NZ$0.60 a month** `[INFERENCE: no PDU reading]`.

**Consequence:** the marginal server cost of a free account is about three hundredths of a cent a month. For an active seller it is about a tenth of a cent. What costs money per seller is the payment processor (§5) and the founder's support time.

## 4. What one tenant can do to another

No plan-based priority exists anywhere. No per-tenant fairness is needed for the marketplace work, because that work is partitioned by construction. What is shared, and what one account can exhaust:

| # | Shared thing | Mechanism | Plan-aware? | Binds at |
|---|---|---|---|---|
| 1 | **Fleet breaker** | `run_breaker` trips a durable, fleet-wide `inventory_halt` when ≥5 settlements in 30 min are ≥50% `failed`/`ambiguous`/`blocked` (`crates/tam-engine/src/breaker.rs:18-20,34-53`). The window counts items, not organisations (`jobs.rs:389-401`, `GROUP BY j.inventory` only). Nothing in `crates/` ever deletes from `inventory_halt` (`raise_fleet_inventory`, `jobs.rs:2912-2930`, is insert-only), and both claim paths refuse work while it stands (`jobs.rs:1354,1570`). | no | **Now.** One account with an expired session and five items settling `blocked` inside half an hour halts that marketplace for every seller until the founder deletes the row by hand. The design required the opposite: "the predicate must distinguish tenant-local failure from marketplace-wide failure, because letting one expired credential take the whole product offline is a worse outage than the failure it prevents" (`docs/design/2026-08-25-listing-sync-design.md:649`). Worst at low seller counts, where one tenant is the whole window. |
| 2 | **tam-server memory under uploads** | `catalogue::upload` takes the whole body as `Bytes` (`catalogue.rs:325`), then `BlobRepo::put` seals it into a second buffer (`crates/tam-storage/src/blobs.rs:89`) and encodes a third for the object store (`:124`). That is ~3× the upload in one pod limited to 256Mi, with a memory-backed `/tmp` on top. The ceiling is `UPLOAD_BODY_BYTES_MAX = 256 MiB` (`crates/tam-limits/src/lib.rs:633`); Cloudflare's 100 MB cap is what actually binds. The only plan-dependent check is the storage quota (`catalogue.rs:371`), and Look has 1 GiB (`lib.rs:142`). | quota only | **Now.** One upload of more than ~70 MB (≈(256 − 45 MiB baseline) ÷ 3) should OOM-kill a replica `[INFERENCE: mechanism read from code, not load-tested]`. Two such uploads, one per replica, take the API down for everyone. |
| 3 | **App DB pool** | 8 connections per `tam-server` replica, 16 in total (`crates/tam-server/src/main.rs:471-472`). `BlobRepo::put` holds its transaction, and so its connection, across the Garage write (`blobs.rs:98-128`). Device claims hold one for a few ms each. | no | Uploads: at **16 concurrent uploads or cover pages** fleet-wide, other tenants wait up to sqlx's default 30 s acquire timeout. Claims: 100 claims/s (1,000 open apps) × ~3 ms ≈ 0.3 connections, so claims alone do not bind below ~30,000 open apps `[INFERENCE]`. |
| 4 | **Postgres eviction** | tam-1/2/3 are BestEffort, so they are the first pods evicted under node memory pressure. `max_connections 200 × work_mem 8MB` is a 1.6 GB worst case (`modules/kubernetes/clusters/elnino-pilot.nix:120-124`, dotfiles). | no | When a co-tenant workload on the node spikes. Not tied to seller count below ~1,000; today there is ~6 GB headroom. |
| 5 | **Garage capacity** | 200 GB usable. Look quota 1 GiB, Sync 20 GiB, Studio 200 GiB (`lib.rs:142,166,190`). | via quota | Typical use (30–150 MB per account): 1,300–6,600 accounts. **Adversarial:** 200 Look accounts filled to quota, or 10 Sync accounts. |
| 6 | **Device claim poll** | Every open app makes one tenant-scoped claim every 10 s whatever the plan, including a Look account with nothing queued. | no | At 1,000 open apps, 100 claims/s ≈ 0.03–0.47 cores of Postgres on a 12-thread node. Not binding, but it is the only load that grows with free accounts. |
| 7 | **Rate budget** | `rate_budget` is keyed per org and connection (`jobs.rs:3902-3906`), so one tenant cannot spend another's. | n/a | never |
| 8 | **Marketplace pacing** | `OUTBOUND_REQUESTS_PER_MINUTE_MAX = 30` is a per-organisation, per-connection window (`lib.rs:750`, drawn in `crates/tam-engine/src/ledger.rs:404-425` against the `rate_budget` row of row 7). The requests leave from each seller's own device and IP. | n/a | never cross-tenant |

`job::CONCURRENT_JOBS_GLOBAL_MAX = 8` (`lib.rs:669`) is read nowhere outside `tam-limits`. Its doc comment ("Automation runs here, on our own infrastructure") has been false since D1 moved automation to devices.

## 5. Payment processor, corrected

Stripe replaced Paddle on 2026-09-22 (`docs/design/decisions.md:905`). The 09-20 net-revenue tables used Paddle's 5% + 50¢. Stripe NZ standard pricing, fetched 2026-09-26 from [stripe.com/nz/pricing](https://stripe.com/nz/pricing):

- domestic cards **2.65% + NZ$0.30**; international cards **3.5% + NZ$0.30**; **+2%** where currency conversion is needed;
- Billing (subscriptions) **0.7%** of Billing volume, pay-as-you-go;
- Tax Basic, no-code **0.5%** per transaction where Teachouse is registered to collect.

A US teacher paying in USD to a NZ account that settles in NZD costs **5.5% + ~US$0.17** on a pack and **6.2% + ~US$0.17** on a subscription. UK and EU buyers add the 0.5% Tax fee. Settling USD into a USD bank account would remove the 2% conversion fee, the single largest avoidable fee. Stripe documents multi-currency settlement ([docs.stripe.com/payouts/multi-currency-settlement](https://docs.stripe.com/payouts/multi-currency-settlement), fetched 2026-09-26), but availability for NZ accounts could not be confirmed from public pages; check Dashboard → Balances `[NOT VERIFIED]`. NZ$0.30 ≈ US$0.17 at an assumed 0.58 USD/NZD.
