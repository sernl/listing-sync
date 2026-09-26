# Pricing re-evaluation, 2026-09-26: what self-hosting really costs, whether free accounts can hurt paid ones, and whether to keep a free tier

- date: 2026-09-26
- status: proposal for the founder. It extends `2026-09-20-pricing-model-re-evaluation.md`, which is now largely in code (`crates/tam-limits/src/lib.rs:133-621`). Only what that note got wrong or left open is re-argued here.
- evidence:
  - `2026-09-26-pricing-evidence-capacity.md`: live cluster and database measurements, where each unit of work runs, the tenant-contention table, and Stripe fees.
  - `2026-09-26-pricing-evidence-competitors.md`: 23 vendors re-fetched on 2026-09-26, and the marketplaces' free-listing norms.
  - The 09-20 packs (`-evidence-{product,infra,market}.md`) remain the source for seller economics, VA rates and SaaS benchmarks.
- the founder's question: maximise profit given self-hosting; decide from evidence whether free accounts should exist at all; could free accounts starve paid ones of memory or CPU unless paid accounts are prioritised?

---

## 0. Answer

1. **Self-hosting makes a free account nearly free:** ≈US$0.0003 a month in disk. An active seller costs ≈US$0.001 a month. The marketplace work runs on the seller's own device, so the server stores covers and answers polls. What costs money per seller is Stripe (5.5–6.7%) and the founder's support time.
2. **Free accounts cannot starve paid ones through the job ledger.** Each seller's marketplace work is claimed by that seller's own device and is scoped to their own organisation in SQL. Two shared mechanisms, however, let *any one account, free or paid,* stop *every* seller today:
   - the fleet circuit breaker counts items, not tenants;
   - a single large upload can out-of-memory the API pods.

   Both are plan-blind bugs, so prioritising paid accounts would not fix either. Fix them now. Add per-tenant upload caps at ~100 sellers. No plan-based priority is needed at 1,000 sellers either.
3. **Keep Look free forever, with tighter caps**: import everything up to 500 resources, 256 MiB storage (down from 1 GiB), 5 lifetime moves per seller (not per shop), and one upload in flight. None of the alternatives (trial clock, card-required trial, paid-only with money back) earns more per 1,000 visitors on this product's shape. Every one of them adds support load or breaks the "test 10–20 resources for 3–6 months" behaviour sellers actually follow.
4. **The market does not argue for moving any price.** Sync at $29 a month sits exactly on the category midpoint. Packs cost 5–15× more per listing than resale crosslisters but 5–25× less than a VA, and no competitor serves TPT↔Tes. Keep every price. Pre-commit the triggers that would raise them (§5.3).

---

## 1. The self-hosting cost model, measured

### 1.1 Where each unit of work runs

Verified in code (capacity pack §1):
- Catalogue reading, cover rendering at import, and every create, publish, update and delete on TPT and Tes run **on the seller's device**, under the seller's session (`apps/desktop/src-tauri/Cargo.toml:146-160`; `crates/tam-worker/src/main.rs:12-20`).
- The server receives a description and one cover per resource, never the bundle (`crates/tam-api/src/import.rs:1-22`, D27).
- The server does four things:
  - answers the device's work poll (one tenant-scoped SQL claim every 10 s while the app is open: `apps/desktop/src-tauri/src/scheduler.rs:182`; `crates/tam-storage/src/jobs.rs:1453-1624`);
  - runs the maintenance pass every 5 s (lease reaping, park revival, breaker: `tam-worker/src/main.rs:115-164`);
  - runs the Sync-plan scheduler every 60 s (`tam-server/src/main.rs:412`);
  - serves console reads, cover redraws and manual uploads (`crates/tam-api/src/catalogue.rs:321`, `::redraw_cover`).

### 1.2 Measured today

| Quantity | Value | Source |
|---|---|---|
| Teachouse pods | 22 millicores, ≈645 MiB total; tam-server 25/41 MiB, workers 4/5 MiB | `kubectl top`, capacity pack §2 |
| Node CPU | 1–3% | same |
| RAM available on the 8 GB nodes | 5.9–6.1 GiB (**the 09-20 pack's 2.8 GiB was wrong**) | `free -m` |
| Database | 28 MB; tenant tables 13.7 MB for 165 resources ⇒ **≈65 KB/resource** at heavy use, ≈25 KB for a Look account `[INFERENCE]` | `psql` |
| Blobs | 97 MB per Garage replica; **median cover 100 KB** | `du`, `blob` table |
| One device claim | 0.25 ms execution + 4.4 ms planning (plan reused on a warm connection) | `EXPLAIN ANALYZE` on `tam-1` |
| Idle baseline | 3.4 tx/s with no sellers active | `pg_stat_database` |
| Power | homelab ~100 W ≈ NZ$31/month at NZ$0.42/kWh; Teachouse's share ≈ NZ$0.60 | capacity pack §3 (**the 09-20 pack priced it on the UK Ofgem cap; the nodes are in Auckland**) |

### 1.3 Per-account and per-active-seller marginal cost

| | Garage (×3 replicas) | Postgres (×3 instances) | Requests | Cost / month |
|---|---|---|---|---|
| Look account, 100 resources, idle | 30 MB (≤150 MB once covers reach marketplace size) | 7.5 MB | none | **≈US$0.0003** |
| Active seller, 200 resources, app open 3 h/day | 60–300 MB | 39 MB | ~21,600 claims → 5–100 DB CPU-seconds | **≈US$0.001** |

The only per-seller costs worth a line in a P&L are:
- **Stripe:** 5.5% + ~US$0.17 on a USD pack from a US card, 6.2% + US$0.17 on a subscription, +0.5% where Teachouse is registered for tax (capacity pack §5). This replaces the Paddle figures in the 09-20 note.
- **Support hours.**

### 1.4 The real ceilings and the seller count at which each binds

| Ceiling | Capacity | Binds at |
|---|---|---|
| tam-server pod memory (256Mi limit, memory-backed `/tmp`) vs whole-body uploads held ~3× in memory | one upload above ~70 MB per replica `[INFERENCE from code]` | **any seller count: one upload** |
| Fleet breaker: 5 adverse settlements in 30 min, counted per item | one account with a broken session | **any seller count: one account** |
| App DB pool, 2 × 8 connections, held across Garage writes during uploads | 16 concurrent uploads | ~100 active sellers importing at once `[INFERENCE]` |
| RAM on elnino and sundog (8 GB), Postgres BestEffort | ~6 GiB headroom each | not seller-bound below ~1,000; binds when a co-tenant workload spikes |
| Garage usable capacity (200 G layout × RF3 = **200 GB usable**) | 30–150 MB per Look account; quota 1 GiB | 1,300–6,600 typical accounts; **200 Look accounts filled to quota** |
| Postgres PVC 40 Gi | ~13 MB per heavy seller | ~3,000 sellers |
| Cloudflare Free | 100 MB request body; 1 IP-keyed rate-limit rule | the body cap makes `UPLOAD_BODY_BYTES_MAX = 256 MiB` unreachable today |
| CPU | 20 threads at 1–3% | not before ~10,000 open apps |

**Conclusion.** Self-hosting is a cost advantage only because the device does the work. At 1,000 sellers, electricity and disk together remain under US$25 a month. The ceilings are concurrency faults, not capacity faults, and each is fixed by configuration or a small code change, not by spending money.

---

## 2. Can free accounts starve paid ones?

### 2.1 Is there per-tenant priority or fairness in the job ledger?

**Priority: none.** Nothing anywhere reads the plan when handing out work. **Fairness: yes, by construction, and that is why priority is unnecessary:**

- **Seller-device work (all of TPT and Tes).** `claim_for_device` filters to `ji.org_id = current_setting('app.current_org')` (`jobs.rs:1546`), so a device can only ever claim its own seller's items. One free seller's queue of 500 items never stands in front of a paid seller's queue: the queues are different rows read by different devices.
- **Per-tenant mutex.** One live lease per organisation and connection (`jobs.rs:1367-1370`, unique index `job_item_one_live_lease_per_connection`). One seller cannot run two writes against the same shop, and nobody can run writes against another seller's shop.
- **Rate budget.** Per organisation and connection (`jobs.rs:3902-3906`; `ledger.rs:404-425`).
- **Cross-tenant FIFO.** `acquire` orders by `created_at` across tenants (`jobs.rs:1371`), but only for `transport_class = 'official_api'` (`:1347`). That is Etsy, which has no adapter, so this path serves nothing today. **This is the one place a plan-priority term would ever belong**, and only once server-side automation for an official API ships.

### 2.2 What server work does a free account generate?

| Work | Look account | Server cost |
|---|---|---|
| Import run | device reads the shop and posts pages of descriptions + one cover each | one DB transaction and one Garage write per cover |
| Catalogue reads (console) | ordinary page loads | milliseconds |
| Cover redraws | only when the seller replaces a file | one file held in memory |
| Moves | at most 5 in total | 5 job items, ~0.08 CPU-s each |
| Device polling | one claim every 10 s while the app is open, even with an empty queue | 0.25–4.7 ms each |
| Scheduler, sync pulls | none (`scheduling: false`, `sync_pull_interval_secs: None`, `lib.rs:151-152`) | 0 |
| Manual upload | allowed up to the storage quota (1 GiB today) | **the whole file ×~3 in a 256Mi pod** |

### 2.3 Where contention can actually happen

The full table is in capacity pack §4. Of the eight shared things, two let one account stop everyone:

1. **The fleet breaker counts items, not tenants.** `recent_outcomes` groups by inventory only (`jobs.rs:389-401`). `run_breaker` halts the marketplace fleet-wide at ≥5 settlements with ≥50% adverse in 30 minutes (`crates/tam-engine/src/breaker.rs:18-20,38-53`). The halt is durable, and no code path lifts it (`inventory_halt` is insert-only, `jobs.rs:2912-2930`).
   - A seller whose TPT session expired, with five queued items settling `blocked`, halts TPT for every seller until the founder deletes a row by hand.
   - The design forbids exactly this (`docs/design/2026-08-25-listing-sync-design.md:649`).
   - It bites hardest when there are few sellers, because one tenant is then the whole window.
   - Free accounts make it likelier only because they are more numerous and less practised `[INFERENCE]`.
2. **Uploads can out-of-memory the API.**
   - `catalogue::upload` buffers the body (`catalogue.rs:325`); `BlobRepo::put` seals a second copy and encodes a third (`crates/tam-storage/src/blobs.rs:89,124`).
   - All of that happens inside a 256Mi container whose `/tmp` is RAM (`modules/kubernetes/applications/teachouse.nix:113-143`, dotfiles).
   - The code cap is 256 MiB (`lib.rs:633`); Cloudflare's 100 MB is the real ceiling. Roughly one 70 MB+ upload per replica is enough.
   - `put` also holds a pool connection across the Garage write (`blobs.rs:98-128`), so 16 concurrent uploads exhaust the whole app pool (`tam-server/src/main.rs:471-472`).

Neither is caused by being free. A paid account triggers both just as easily, so plan priority fixes neither.

### 2.4 Cheapest controls, and when each is needed

| When | Control | Where | Cost |
|---|---|---|---|
| **Now** | Breaker trips only when adverse outcomes span ≥3 organisations; one organisation's run of failures stays an org-level halt (which the machine already raises on an ambiguous create) | `crates/tam-engine/src/breaker.rs` (new `BREAKER_MIN_TENANTS = 3`), `jobs.rs:389-401` (`count(DISTINCT ji.org_id) FILTER (…)`) | ~20 lines + test |
| **Now** | Lower `UPLOAD_BODY_BYTES_MAX` to 64 MiB until the upload streams to Garage. The desktop import path never sends bundles, so only web-console manual uploads of very large bundles are refused, and those are already refused at Cloudflare above 100 MB | `crates/tam-limits/src/lib.rs:633` | one constant |
| **Now** | tam-server memory limit 256Mi → 512Mi; Postgres `requests 512Mi / limits 1.5Gi` so it stops being BestEffort | `teachouse.nix:113-122`, `elnino-pilot.nix` (dotfiles) | config |
| **Now** | Look storage quota 1 GiB → 256 MiB. A Look account holds covers only; 500 × ~300 KB = 150 MB. This moves the adversarial Garage fill from 200 accounts to 800 | `lib.rs:142` | one constant |
| **~100 sellers** | Uploads in flight per organisation: Look 1, Sync/Studio 3, held as a per-org semaphore in `tam-api` | new `Capabilities::uploads_in_flight_max` | small |
| **~100 sellers** | One Cloudflare rate-limit rule (the Free plan's one rule) on `/v1/*/upload` and `/v1/import/*` by IP | Cloudflare dashboard | none |
| **~1,000 sellers** | Nothing in the ledger. Optionally, a Look app with an empty queue polls every 60 s instead of 10 s, which cuts the only load that grows with free accounts by 6×. The saving is ~0.1 core at 1,000 open apps, so do it only if Postgres CPU shows it | `scheduler.rs:182` | small |
| **When Etsy's official-API branch ships** | Order `acquire`'s cross-tenant scan by plan strength, then `created_at` | `jobs.rs:1371` | small |

**Verdict:**
- **Now:** no prioritisation. Fix two plan-blind faults.
- **At 100 sellers:** per-tenant concurrency caps, which are fairness rather than priority.
- **At 1,000 sellers:** still no plan priority. Priority becomes relevant only when the server itself starts doing marketplace work.

---

## 3. Should there be a free tier?

### 3.1 The facts that decide it

- **Cost:** a Look account costs ≈US$0.0003 a month (§1.3). Ten thousand of them cost about $3 a month, roughly one pack's Stripe fee.
- **Seller behaviour:** the documented behaviour is a 10–20 resource test catalogue, watched for 3–6 months before the rest follows (09-20 market pack §7). A 14- or 30-day clock expires long before that decision is made.
- **Product shape:** revenue is packs first (one-off moves). A trial of a one-off purchase is meaningless. A card-required trial can only be a trial of the Sync *subscription*, which pushes pack buyers into a plan the 09-20 note showed churns at month 2–3.
- **Marketplace norms:** listing is free on both marketplaces Teachouse bridges (TPT $0 per listing after the seller membership; Tes $0 and free uploads encouraged). Etsy's $0.20 per listing is the only per-listing charge a teacher is likely to know (competitor pack §5.5). A teacher's own business runs on free resources that lead to paid ones. "Import free, move for a fee" is that same pattern.
- **Category norms:**
  - Permanent volume-capped free tiers: Zipsale (10 items/mo), Sellbrite (30 orders/mo).
  - No-card trials: Flyp 100 days, Zipsale 1 month, PrimeLister 7 days capped at 100 tasks, Listelf 7 days.
  - Card-first: Vendoo (14 days, card required) and Crosslist (pay first, 3-day / 20-listing money-back).
  - **Where a trial is capped, it is capped by listings, not only by days** (competitor pack §5.2).
- **Abuse:** free moves are keyed on the storefront's platform-account digest and are once-ever (`crates/tam-storage/src/device.rs:761-807`). Farming them means opening a new TPT shop ($29 membership, which costs more than the 5 moves are worth at $11.75) or a new Tes author account whose shop is not the farmer's real one. Multi-accounting is self-defeating.

### 3.2 The four options per 1,000 organic visitors

Base rates:
- ChartMogul, 09-20 market pack §6: no-card 4.5% visitor→signup and 8.9% signup→paid; card-required 3.5% and 31.4%.
- Paid-first rates are `[INFERENCE]`; no public benchmark exists for a $47 desktop tool.
- "Buyer value" is first-year gross per paying seller: mean first pack $126 (mix 30/25/25/12/8% across the five rungs), 25% repeat at ~$100, plus Sync and service attach. That gives ≈**$150**.

| Option | Signups | Buyers in 90 days | Late buyers (3–6 months) | Gross, year 1 | Extra support | Failure mode |
|---|---|---|---|---|---|---|
| **(a) Look, free forever, capped** | 45 | 4.0 | +0.9 (2% of a list that stays reachable) | **≈$735** | onboarding questions: 45 × 30% × 15 min ≈ 3.4 h | dormant accounts (cost ≈ nothing) |
| (b) 14/30-day trial, then read-only | 45 | 4.0 | +0.2 (moves expired, list colder) | ≈$630 | + expiry tickets ≈ 1.1 h | the clock ends before the seller's Tes test does |
| (c) Card-required 14-day Sync trial | 35 | 11 first charges at $29 | — | ≈$680–800 (mean 2.5 months; ~15% refunds `[INFERENCE]`) | refunds, disputes, "I forgot to cancel" | sells the subscription to pack buyers; every refund is a founder hour; conflicts with packs-first |
| (d) Paid-only, 14-day money-back | — | 3–5 (0.3–0.5% of visitors `[INFERENCE]`) | 0 (no list) | ≈$400–675 after ~10% refunds | refund requests on already-delivered moves | loses the Look list, the only marketing asset that compounds with no ad budget |

On revenue, the options sit within about ±20% of each other, and the spread is smaller than the uncertainty in the inputs. What separates them is what they cost the founder and what they leave behind:
- (a) is the only option that keeps a reachable list of sellers who have already imported their shop. That list is where a 3–6-month decision gets made.
- (a) is also the only option whose failure mode (idle accounts) costs nothing.
- (c) wins on paper only if its conversions stay subscribed, and the 09-20 evidence says they do not.
- (d) is worth revisiting only if Look→pack conversion within 90 days is below 3% after the first 100 accounts. At that point the free tier is not converting, and paid-first with a money-back window becomes the cheaper experiment.

### 3.3 Verdict and exact caps

**Keep option (a): Look, free forever, labelled "(Trial)" on the page as the founder's feedback asks.**

| Cap | Today | Proposed | Why |
|---|---|---|---|
| Resources imported | 500 | **500** | Covers the 40–200-resource seller whole. The ~764-listing dual-lister sees most of their shop and has a reason to buy. |
| Storage | 1 GiB | **256 MiB** | Look stores covers only (≤150 MB at 500 resources with marketplace-size covers). Bounds the adversarial Garage fill. |
| Free moves | 5 **per storefront** (10 for a TPT+Tes seller; `device.rs:792-803`) | **5 per seller**, still once-ever per storefront digest | Matches the founder's copy ("5 moves onto a marketplace of your choice"). Ten moves is half of Pack 20. |
| Uploads in flight | unbounded | **1** | Tenant-contention control 2.4 |
| Scheduling, sync pulls, rules | off | off | unchanged |
| Devices | 5, unadvertised | 5 | Server impact is negligible; not worth re-opening. |
| Dormancy purge | none | **none** | An idle Look account costs ≈US$0.0003 a month. Purging would save cents and lose the list. |

---

## 4. The competitive market, real data (fetched 2026-09-26)

Full tables with URLs are in `2026-09-26-pricing-evidence-competitors.md`. The figures that bear on Teachouse:

| Segment | Vendors, entry price | Unit | $ per listing at entry | Free / trial | Annual discount |
|---|---|---|---|---|---|
| Resale crosslisters, flat | Vendoo $14.99 · List Perfectly $29 · PrimeLister $49.99 · Flyp $9 · Listelf $9.99 | unlimited listings; meter AI/photo credits | n/a | Vendoo 14 d card; LP "100 free listings" guarantee; Flyp 100 d no card | −16.7% (Vendoo), −33% (PrimeLister), none (LP, Flyp) |
| Resale crosslisters, metered | Crosslist $29.99/200 · Zipsale £15/100 (credits from £0.18, valid 12 months) | new listings per month | **$0.13–0.18** | Crosslist pay-first, 3-day/20-listing money-back; Zipsale Free 10/mo | −16.7% / −15% |
| Resale, inventory-metered | Nifty $39.99 / 1,500 active items | active items + credits | $0.027 per item-month | 7 days | −10% |
| Ecommerce multichannel | Sellbrite $29 (100 orders) · Listing Mirror $69 (500 SKUs) · LitCommerce $29 (3 ch × 1,000) | orders; SKUs × channels; listings × channels | $0.03–0.29 | Sellbrite Free 30 orders/mo; LM 30 d no card | LitCommerce −20% |
| Creator storefronts | Gumroad 10% + $0.50 · Payhip 5/2/0% (Free/$29/$99) · Stan $29 · Teachable $39 + 7.5% | % of sale or flat | n/a | free tiers are take-rate | Stan −13.8%, Teachable −22–26% |
| Teacher tooling | TPT Basic $29 once / Premium $59.95/yr · Easel free · Canva for Education free · Boom 15%/10% · Teach Simple 50% pool (non-exclusive) | membership or % | $0 per listing | — | — |
| Manual substitute | Fiverr TPT VA $35/gig; uploads $5–20; covers from $10 (09-20 market pack §3) | per listing | **$10–30** | — | — |

**Where moves-as-currency sits:**
- A Teachouse move costs **$0.79–2.35** (`lib.rs:543-574`). That is 5–15× a resale crosslister's new listing and 4–12× an Etsy listing fee, but 5–25× cheaper than the VA doing the same work.
- The resale comparison flatters the resale tools. A reseller lists hundreds of one-off items a month at a few dollars' margin each and wants the listing to cost nothing. A teacher moves a static catalogue once, and each move carries work no resale tool does: the cover rebuilt at the target's size, taxonomy and standards mapped, US→UK year groups, a converted and fee-aware price (the £3 trap), and a licence.
- Measured automation time is 24 s per create against ~20 minutes by hand `[INFERENCE, 09-20]`. At a teacher's $20–30 an hour, a move saves **$7–10** of time and is priced at 8–34% of that.
- The monthly price sits exactly on the category: **Sync $29** vs Crosslist Bronze $29.99, Vendoo Growth $29.99 and List Perfectly Simple $29.
- The annual price, **$240 ($20/mo, −31%)**, is deeper than the category median of −16.7% and matched only by PrimeLister's −33%. That is deliberate: it makes monthly the penalty and takes a year's cash before the month-2–3 churn. Under Stripe the fee argument for annual has shrunk: the fixed fee is NZ$0.30, not 50¢, so annual now saves ~0.8 points on fees rather than ~2–4. The churn and cash arguments stand unchanged.
- **No competitor cross-lists TPT↔Tes.** The only live alternatives are doing it by hand or paying a VA.

**Is anything undersold?** The two candidates:
- **The 250 and 500 rungs ($0.98 and $0.79 a move).** A top-decile seller earning $300–1,000 a month would pay more for the time saved. Their ceiling is set by what Tes earns them, not by what the work is worth: reported Tes returns are "a few pounds a month" to "£40 over six months" (09-20 market pack §7). $397 already takes 4–8 months of plausible incremental Tes income to earn back. Raising it before a single sale proves the ceiling is higher would be a guess.
- **The annual discount.** Moving to the category's 2-months-free ($290) would collect ~$50 more per annual subscriber (≈$1,500 a year at 30 subscribers). It would also break the "$20 a month" page anchor and the Founding offer's stated 25%/20% arithmetic (`lib.rs:602-612`). It is not worth it before the first cohort renews.

---

## 5. Recommended model

### 5.1 Keep (unchanged in `tam-limits`)

| Line | Price | Included | Reason per number |
|---|---|---|---|
| **Look (Trial)** | $0 forever | import up to 500 resources; 5 moves per seller; 256 MiB | §3.3 |
| **Sync (Subscription)** | **$29/mo · $240/yr** | 25 moves/mo, rolling to 75; scheduling, 6-hourly pulls, rules, statistics, email support | $29 = category midpoint. $240 = "$20 a month, billed yearly", deliberately 31% off so annual is the obvious choice. One month at $29 buys 25 moves at $1.16 each, dearer than every rung above 50, so a seller cannot undercut the packs by subscribing for one month and cancelling. |
| **Move Packs (One-Off)** | $47/20 · $77/50 · $127/100 · $247/250 · $397/500 · "Talk to us" above | moves valid 12 months; 90-day edit window | Readable, approved and live. $2.35→$0.79 a move sits between the resale per-listing band and the VA. The 12-month validity matches Zipsale's credit precedent. "Talk to us" above 500 is where the founder learns most per hour. |
| **Studio** | $44/mo · $440/yr, unsold | 100 moves/mo, rolling to 300 | Trigger unchanged: 10 paying Sync accounts, or one subscriber exhausting the rolling cap twice in a quarter. |
| **Founding 100** | $180 year 1, $192 years 2–3, then list | +20 moves; annual only; closes at 100 places or 2026-12-31 | Cash before churn can start: 100 × $180 = $18,000 collected. The three-year cap stands. |
| **Move with me** | $99 | 45-minute screen share of the first import and pack | Highest margin on the page, and a discovery interview. Stripe allows it; Paddle forbade it (`decisions.md:913`). Keep the 5-a-week cap. |
| AI | coming soon, 200 fills bundled on Sync | — | unchanged; costs cents |

### 5.2 Change

These are the only changes to `tam-limits` and adjacent constants (also listed in the output JSON):

| Constant | Current | Proposed | Reason |
|---|---|---|---|
| `Plan::Free` `storage_bytes_max` (`lib.rs:142`) | `1 << 30` (1 GiB) | `256 << 20` (256 MiB) | Look holds covers only; bounds the adversarial Garage fill (200 → 800 accounts) |
| Free-move grant keying (`tam-storage/src/device.rs:792-803`; value `free_moves_lifetime: 5`, `lib.rs:149`) | 5 per bound storefront | 5 per organisation, still once-ever per storefront digest | Founder's copy says 5 onto the marketplace of the seller's choice; 10 is half of Pack 20 |
| `http::UPLOAD_BODY_BYTES_MAX` (`lib.rs:633`) | 256 MiB | 64 MiB until uploads stream | Unreachable through Cloudflare (100 MB) and unsafe in a 256Mi pod holding ~3 copies |
| `job::CONCURRENT_JOBS_GLOBAL_MAX` (`lib.rs:669`) | 8 | delete | Read nowhere; its comment describes server-side automation that D1 removed |
| new `Capabilities::uploads_in_flight_max` | — | Free 1 · Subscriber 3 · Studio 3 | The per-tenant fairness control for the one shared memory-heavy path, needed by ~100 sellers |
| new `tam-engine::breaker::BREAKER_MIN_TENANTS` | — (counts items) | 3 organisations | One account must not be able to halt a marketplace for everyone (design line 649) |

Not constants, but part of adopting this: tam-server memory limit 512Mi; Postgres requests and limits; one Cloudflare rate-limit rule on the upload paths; enable USD settlement in Stripe if the NZ account supports it, which saves the 2% conversion fee on nearly every sale.

### 5.3 Pre-committed triggers (instead of guessing now)

- **Raise Pack 250 and 500 by 20%** (to $297 / $477) if ≥30% of the first 30 pack buyers choose 250 or 500.
- **Raise Pack 20 to $57** if Look→first-pack conversion within 90 days exceeds 15% over the first 100 Look accounts.
- **Switch to paid-first with a 14-day money-back window** (option d) if that conversion is below 3% over the first 100 Look accounts.
- **Sell Studio** on its existing trigger.

### 5.4 Expected revenue

Per Look account per year, gross. Founding pricing is used for yearly Sync until 100 places or 2026-12-31.

| Assumption | Low | Base | High |
|---|---|---|---|
| Look → first pack within 90 days (mean $126) | 5% | 10% | 15% |
| Of those, buy a second pack (mean $100) | 25% | 25% | 35% |
| Look → Sync yearly | 1% | 3% | 6% |
| Look → Sync monthly (mean life 3 months) | 1% | 2% | 3% |
| Look → Move with me | 1% | 3% | 5% |
| **Gross per Look account per year** | **≈$11–12** | **≈$25–27** | **≈$42–46** |

| Look accounts in year 1 | Low | Base | High | Server cost / year | Founder support `[INFERENCE: 15 min × 30% of Look + 1 h per payer]` |
|---|---|---|---|---|---|
| **50** | $560 | $1,260 | $2,130 | < US$1 marginal | ~13 h |
| **200** | $2,240 | $5,040 | $8,500 | < US$2 | ~50 h |
| **1,000** | $11,500 | $26,100 | $42,500 | < US$10 marginal (+ ~US$120 once for a disk, if covers grow) | ~255 h (≈5 h/week) |

Net of Stripe is ≈93–94% of gross. The whole homelab's electricity is ≈US$220 a year whatever the count, and Teachouse's share of it is ≈US$4.

These numbers are lower than the 09-20 note's ≈$47,000 at 900 accounts. That note assumed all 100 founding places plus 40 non-founding annual subscribers (≈15% of Look). Here the base is 3%, which is what the category's opt-in conversion supports.

The consequence for "no underselling": with a pool this small (~4,000 dual-listers, top decile ~400, 09-20 market pack §2), **revenue is bounded by reach, not by price**. The founder's hours are worth more on outreach and "Move with me" sessions than on another price change. Break-even against the $5,000 a month opportunity cost in `commercial-model.md` needs ~2,300 Look accounts at the base rates, or the high rates at ~1,400.

---

## 6. The recommendation, one sentence per number

1. Keep **Look free forever**; do not replace it with a trial clock, a card-required trial or paid-only.
2. Keep Look's import cap at **500 resources**.
3. Cut Look's storage to **256 MiB** from 1 GiB.
4. Give **5 free moves per seller**, not 5 per shop.
5. Allow Look **1 upload in flight**, and Sync/Studio **3**.
6. Keep **Sync at $29 a month and $240 a year**.
7. Keep Sync's **25 moves a month, rolling to 75**.
8. Keep the packs at **$47/20, $77/50, $127/100, $247/250, $397/500**, and "Talk to us" above.
9. Keep pack moves **valid for 12 months** with a **90-day edit window**.
10. Keep **Founding 100 at $180, then $192 for years two and three**, annual only, **+20 moves**, closing at **100 places or 2026-12-31**.
11. Keep **Move with me at $99**, capped at five a week.
12. Keep **Studio at $44 / $440, unsold** until its trigger.
13. Lower `UPLOAD_BODY_BYTES_MAX` to **64 MiB** until uploads stream.
14. Make the fleet breaker require **3 organisations** before halting a marketplace for everyone.
15. Delete the unused `CONCURRENT_JOBS_GLOBAL_MAX = 8`.
16. Pre-commit the price triggers in §5.3: **+20% on the top two rungs**, **Pack 20 to $57**, or **switch to paid-first**.
