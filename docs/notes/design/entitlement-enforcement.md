# Entitlement enforcement: every allowance, where the server holds it

- date: 2026-09-30
- status: implemented on `feat/tier-limits` (0.16.0)
- reads with: `research/2026-09-29-pricing-structure-review.md` (the ladder and the matrix), `crates/tam-limits/src/lib.rs` (`Plan::capabilities`, `PLAN_FEATURES`), `crates/tam-api/src/entitlement.rs` (refusals, the in-transaction gates, `GET /v1/entitlement`).

The founder's rule for 0.16.0: the free tier starts at **100 resources**, **watermarked previews are counted per month**, and **every allowance is enforced on the server and tracked**, so nobody gets more than their plan. The console only warns early. Any request that gets past it still meets the gate below.

## 1. The two new caps

| | Look | Starter | Sync | Studio |
|---|---|---|---|---|
| Resources (`resources_max`) | **100** (was 500) | **500** (was no cap) | **2,000** (was no cap) | no cap |
| Watermarked previews a month (`previews_per_month`) | **5** | **50** | **200** | no cap |
| Price (unchanged) | Free | $12 | $29 | $59 |

Why these numbers, tested against the pricing review:

- **Look at 100.** This is the founder's floor. It still shows every tool on a real shop. It is a fifth of the old 500, so the §9 trigger ("Look gives too much: … then the 500-resource ceiling") is pulled now rather than after a quarter of data. Storage stays at 256 MB, which 100 cover images fill to about a tenth.
- **Starter at 500, Sync at 2,000, Studio uncapped.** The steps are 5× and 4×, inside the review's 2–3×-or-better value-gap rule (§2.2). They also sit above the move ladder, which climbs 2.5× then 4× (§3). A Starter seller adds about one resource a week (§5.2), so 500 is years of new work plus an imported back catalogue. A Sync seller adds weekly onto both marketplaces, and 2,000 covers a large TPT store. Studio is sold as "big catalogues and whole-shop moves", so it has no ceiling.
- **The category.** Every cross-lister in §2.1 leaves items uncapped on its top tier, and Studio still does. Starter and Sync are capped because the resource count is the axis that grows with the seller (§2.3, "counted per tier: whatever grows with the seller's volume").
- **Previews 5 / 50 / 200 / uncapped.** Five is enough to see what a watermarked preview does to a listing, but not enough to preview a whole shop. That fits the free-tier evidence: limit by usage rather than withhold (§2.2). Fifty covers one preview for every new Starter resource and a refresh of the rest. Two hundred is four times that, in step with Sync's moves. The ladder has the same shape as the AI-fill ladder (50 / 200 / 600), which copies List Perfectly's AI allowances (§2.1).
- **Monthly, on the UTC calendar month.** A preview is made, not held. Deleting one does not give it back, so it is a counter and not a `count(*)`. The allowance renews at 00:00 UTC on the 1st.

`PLAN_FEATURES` now draws "Watermarked previews" as a `per_month` row ("5 a month") instead of a tick. The landing comparison table, the card highlights and the console's Billing bullets all read it from the generated data. Look's tagline no longer promises "every resource": *"For a small shop: bring your resources in, make previews and try five moves."*

`tam-limits` tests pin three things:
- no capability gets worse up the ladder, including fields the table does not show (`every_cap_climbs_the_ladder` names every field with no `..`);
- moves, resources and previews each strictly climb;
- Look starts at 100.

## 2. Usage tracking

- **Standing counts** (resources, templates, collections, labels, devices, marketplaces, storage) are `count(*)` / `sum(byte_len)` over the rows themselves, so a deleted row gives its room back. `EntitlementRepo::usage` reads all of them in one query. Labels count the seller's own only; the marketplaces' system labels are the import's and cost nothing, matching both label gates.
- **Monthly counts** (previews, AI fills) live in `usage_counter` (migration 0098): `(org_id, kind, month, used)`, one row per organisation, kind and UTC month, with forced row-level security and no backoffice grant. The gate spends with one conditional upsert (`INSERT … ON CONFLICT DO UPDATE SET used = used + n WHERE used + n <= cap`). Two requests racing for the last preview serialise on the row lock, and the loser is refused. Unlimited plans are counted too, so usage is on record whatever the plan.
- **`GET /v1/entitlement`** returns `capabilities` (every limit) beside `usage`. `usage` now includes `storage_bytes`, `previews`, `ai_fills` and `month_resets_at` (the first instant of next UTC month, in epoch ms). The console's Billing page shows a "What you have used" list from it.

## 3. The audit: every capability, and the gate that holds it

"Before" is what 0.15.0 enforced. Every gap in that column is closed in this change.

| Capability | Server gate (after) | Before | Console |
|---|---|---|---|
| **Resources**: create (`POST /v1/products`) | `prepare_create` early count, plus `refuse_past_resource_cap_in` inside the create's guarded transaction after the insert (`catalogue.rs` `apply_create`) | early count only, outside the lock, so two concurrent creates could both take the last place | `limitOf('resources')` disables New resource (Resources page, console home) |
| **Resources**: spreadsheet rows (`POST /v1/imports/{b}/commit` and the scheduler drain) | same `apply_create`; a refused row fails with the plan's sentence and the batch carries on | early count only | – (the row report states the refusal) |
| **Resources**: shop import run (`import_runs::commit_one`) | `refuse_past_resource_cap_in` after `apply_prepared`, in the item's guarded transaction; the item fails with the sentence and the run completes | **none** | row shows `failure_detail` |
| **Resources**: re-import restoring a deleted resource or a merged survivor | same gate, when the restore actually brought one back | **none** | as above |
| **Resources**: migrate leg from a device (`import.rs` `apply_admitted`) | same gate after `apply_import`, under the request lock | **none** | request's resource failure |
| **Resources**: undoing a merge (`duplicates.rs`) | same gate when the loser is restored (otherwise merge, add one, undo would step around it) | **none** | refusal toast |
| **Storage** (`storage_bytes_max`): uploads, device-library copies | `catalogue.rs` `upload`, `library.rs` `copy_file` | yes | upload refusal sentence |
| **Storage**: shop-import cover and listing pictures | `store_cover_within`: a picture that would pass the ceiling is not kept, so the resource arrives without it | **none** | – |
| **Storage**: migrate-leg cover | refused with the storage sentence (a move needs its cover) | **none** | request's resource failure |
| **Storage**: thumbnail redrawn on a file swap | refused with the storage sentence | **none** | refusal |
| **Storage**: thumbnail redrawn on a file removal | deliberately unbounded, because removing a file is how a seller over the limit makes room | – | – |
| **Watermarked previews** (`previews_per_month`) | every preview a resource gains spends one: `POST /v1/products/{p}/files` with role `preview`, `PUT …/files/{f}` on a preview row, and the `previews` of `POST /v1/products` (spent in the create's transaction). `POST /v1/uploads?slot=preview` (the preview maker and the drop zone) is refused before a byte is stored once the month is used up | **none** | `limitOf('previews')` disables Make a preview, Change and the drop zone, with the sentence |
| **AI description fill** (`ai_fills_per_month`) | **not built**: no route exists. The counter kind `ai_fill`, `QuotaKind::AiFills` and its sentence are in place, so the route must call `spend_monthly_in(AiFill)` in its own transaction | n/a | shown as "coming soon" |
| **Templates** (`templates_max`) | `resource_templates.rs` create (the only writer) | yes | templates page, resource tab |
| **Collections** (`collections_max`) | `collections.rs` create (the only writer) | yes | collections page, export page |
| **Labels** (`labels_max`) | `resources.rs` set labels, `collections.rs` add labels, and now the spreadsheet commit (a row that would create a label past the ceiling fails; reusing existing labels never does) | spreadsheet rows created labels unchecked | labels page |
| **Devices** (`devices_max`) | `devices.rs` register and restore | yes | Devices |
| **Marketplaces** (`marketplaces_max`) | `devices.rs` session bind | yes (no cap on any plan) | Marketplaces |
| **Moves**: spending (the balance) | the ledger: `migrations.rs` confirm, `jobs.rs` sync submit, `work.rs` debit on settle; a move with no balance is refused | yes | `movesReason` |
| **Moves**: `moves_per_month`, `moves_accrual_cap` | `billing.rs` credits `moves_per_month` at each period start, never past `moves_accrual_cap` (the ledger holds the ceiling, so the balance can't outgrow it) | yes | Billing |
| **Moves**: `free_moves_lifetime` | `tam-storage` `device.rs` grants it once per storefront through `grant_storefront_allowance_in` (`storefront_allowance` is the record), so a second organisation naming the same shop gets nothing | yes | Billing |
| **Scheduling** | `schedules.rs` `held()` on every route; the scheduler skips a plan without it | yes | sharing page |
| **Auto-publish rules** | `schedules.rs` republish, `sync_settings.rs`, scheduler `finish` | yes | sync and sharing pages |
| **Edit sync interval** (`sync_pull_interval_secs`) | `sync_settings.rs` refuses and clamps on write, and now the scheduler reads a stored interval shorter than the plan's as the plan's, so a downgrade takes effect on the next pass | on write only; a downgraded seller kept the faster pull until they saved again | sync page |
| **Statistics** (`analytics`) | `analytics.rs` summary, and now the device capture order and capture intake (a plan without statistics captures nothing against the shop's rate budget) | summary only | analytics page |
| **Import** (`import_marketplace`, `import_spreadsheet`) | `import_runs.rs` create, `import.rs` page, `import_batch` | yes (true on every plan) | import page |
| **Duplicate review** | `duplicates.rs`, `import_runs.rs` | yes (true on every plan) | – |
| **Export** (`export`) | `export.rs` now reads it (true on every plan, pinned by `export_is_granted_on_every_plan`) | never read | – |
| `publish_marketplaces_max` | no gate: `u32::MAX` on every plan, so no plan can exceed it. A cap below that needs a gate at publish | – | – |
| `uploads_in_flight_max` | not enforced, with a reason: it is a fairness bound between accounts sharing a replica, not an allowance a seller buys, and belongs beside the limiter; still not built (review §10) | – | – |
| `pack_edit_days` | not enforced, with a reason: editing a moved listing after the window is promised in the review (§10) but not built, so there is nothing yet to hold. The dead console line printing it is removed | – | – |
| `support` | not enforced, with a reason: it is a promise of reply time, not a bound a request can exceed; the pricing table renders it | – | pricing table |

Every refusal is the same 422 `quota_exceeded` with `detail.quota` naming the bound (`listings_max`, `storage_bytes_max`, `labels_max`, `previews_per_month`, …) and a one-sentence reason. The console's `limitReason` uses the same sentences: *"Your plan includes 100 resources. Upgrade to add more."* and *"Your plan includes 5 watermarked previews a month. Upgrade to make more, or wait until next month."*

## 4. Grandfathering

An organisation already over a new ceiling keeps everything it has. It can't add more until it upgrades or gets back under. Every standing-count gate compares against the live count after the write, so the next resource, label or template is refused and nothing existing is touched. Previews start counting from zero at deploy, because the counter table is new.

## 5. What re-opens this

- Look converts under 2% within 90 days, even at 100 (review §9): look at previews before the resource ceiling.
- More than 10% of Starter or Sync sellers reach their resource ceiling within a quarter: the ceiling is setting the price, not the seller's volume. Raise the ceiling before adding a tier.
- More than 25% of any plan's sellers use their whole preview allowance in a month: the preview allowance is too tight for what the plan is sold as.
