# Pricing structure review, 2026-09-29: what is core, what is counted, what starts at a plan

- date: 2026-09-29
- status: decided and implemented on `feat/pricing-model` (`crates/tam-limits/src/lib.rs`: `PLANS`, `Plan::capabilities`, `PLAN_FEATURES`; `apps/landing/src/components/Pricing.astro`, `apps/landing/src/pricing.js`).
- reviews: `2026-09-27-subscription-tiers.md` (the ladder as shipped in 0.14.0) and the founder's brief for 0.15.0 ("declutter the pricing page into good, better, best with a comparison table"). Neither is taken as given: every price, cap and gate below was re-tested against how established tools package their tiers and against what the code actually enforces.
- evidence: vendor pricing pages fetched first-hand on 2026-09-29 (§2), pricing-practice sources (§2.2), and a code inventory of every `Capabilities` field and console feature (§5.1). Earlier evidence packs (`2026-09-26-pricing-evidence-competitors.md`, 23 vendors) still stand and are cited where they are used.

---

## 0. Answer

| | **Look** | **Starter** | **Sync** ★ Recommended | **Studio** |
|---|---|---|---|---|
| Monthly | Free | $12 | $29 | $59 |
| Yearly | Free | $96 ($8/mo) | $240 ($20/mo) | $480 ($40/mo) |
| Yearly saving | – | $48 (33%) | $108 (31%) | $228 (32%) |
| Tagline | Bring your shops in, see every resource in one place and try five moves. | For teachers who add a resource now and then. | For teachers who add resources every week. | For big catalogues and whole-shop moves. |

**No price changes.** Every Stripe price key and amount stays as it is (§3, §4, §11).

**Three cap changes, all upward**, so nobody loses anything and nothing needs grandfathering (§8):

1. **Look gets 1 collection** (was 0), matching its 1 template: a free seller can try grouping resources before paying.
2. **Labels are unlimited on every paid plan** (were 20 / 20 / 50). Look keeps 5.
3. Nothing else moves. Moves, rollover, storage, templates, collections on paid plans, sync cadence, statistics, rules and support are unchanged.

**One structural change:** the comparison table is now data (`PLAN_FEATURES` in `tam-limits`), and its cells are read from the gates at compile time, so the landing and the console can't promise a plan more than the server allows. `PlanRow` gains `recommended` (Sync, exactly one) and `tagline`.

**Recommended plan: Sync**, raised and badged on the page (§6).

---

## 1. What was reviewed, and how

The questions the founder's brief asks, each answered with evidence rather than carried over:

1. Are three paid tiers above a free one the right shape? (§2, §6)
2. Are $12 / $29 / $59 right? (§3)
3. The 2026-09-27 note set the yearly saving at about a third. The brief's norm is 15–20%. Which is right? (§4)
4. Which features belong in every tier, which are counted, and which start at a plan? (§5)
5. Which plan is recommended, and how is it presented? (§6)
6. What should the free tier include and withhold? (§7)
7. When a seller runs out of moves, is that overage or a hard cap? And how would a future price change treat existing subscribers? (§8)

## 2. How established tools package tiers

### 2.1 Vendors (fetched 2026-09-29 unless marked)

| Tool | Tiers (monthly · per month billed yearly) | Core in every tier | Counted per tier | Starts at a tier | Recommended | Yearly saving | Source |
|---|---|---|---|---|---|---|---|
| **Vendoo** (cross-lister, the closest shape) | Starter $14.99 · $12.49 / Growth $29.99 · $24.99 / Pro $59.99 · $49.99 / Enterprise | unlimited items, all marketplaces, sale detection, templates, import, delist/relist, **analytics**, mobile app | PhotoRoom removals (300 / 1,500), bulk actions (240 at once) | AI listing enhancement (Growth); auto offers, marketplace sharing, listing videos, **premium support** (Pro) | **Growth, the middle** ("Recommended") | "up to 2 months free" (16.7%) | [vendoo.co/pricing](https://www.vendoo.co/pricing) |
| **List Perfectly** (cross-lister) | Simple / Business / Pro / Pro Plus (Simple $29, Pro $69 in `2026-09-26-pricing-evidence-competitors.md`) | catalog, templates (unlimited), import, bulk edit, drafts, unlimited cross-listing, all marketplaces | AI listings 25 / 50 / 200 / 1,000 a month; barcode scans; background removals | which listing fields cross-list (depth climbs per tier); image editing (Business+); sub-accounts | – | – | [listperfectly.com/pricing](https://listperfectly.com/pricing) |
| **Buffer** (creator scheduling) | Free / Essentials $6 per channel / Team $12 per channel | queue, calendar, drafts, templates, AI assistant, integrations, email support on every plan | channels (3 free), scheduled posts (10 per channel free, then unlimited under fair use at 5,000), **tags 3 → 250**, analytics history (30 days → unlimited) | advanced analytics and reports (Essentials); users, approvals, permissions (Team) | – | "approximately 2 months", stated as 20% | [buffer.com/pricing](https://buffer.com/pricing) |
| **Kit** (creator email) | Free / Creator $33 yearly / Pro $66 yearly | unlimited landing pages, forms, broadcasts, tagging, selling | subscribers (the price axis; Free up to 10,000) | automations and sequences (Creator); engagement analytics, insights dashboard, unlimited users, **priority support** (Pro) | Creator badged "Most popular" | 17% ("Save $78 per year") | [kit.com/pricing](https://kit.com/pricing) |
| **Sellfy** (creator store) | Starter $29 · $22 / Business $79 · $59 / Premium $159 · $119 | unlimited products, store, discount codes, email marketing, 24/7 email support | annual sales (soft cap, 2% overage), file size, email credits | upselling, cart abandonment, affiliates, product migration (Business); **priority support** (Premium) | **Business, the middle** (★) | 25% | [sellfy.com/pricing](https://sellfy.com/pricing/) |

### 2.2 Pricing practice

- **Good, better, best.** Three tiers is the modal structure (48% of successful SaaS per [saassoftware.org](https://saassoftware.org/blog/saas-tiered-pricing-models-guide/)). Price Intelligently's figure is about 30% more revenue than a single price ([getmonetizely.com](https://www.getmonetizely.com/articles/good-better-best-does-the-three-tier-pricing-model-still-work-in-saas)). Each step should be worth 2–3 times the one below (Lincoln Murphy, same source).
- **The recommended tier is the middle one, and it must be the best value.** ProfitWell's Patrick Campbell aims for 60–70% of customers on the middle plan ([getmonetizely.com](https://www.getmonetizely.com/articles/the-art-of-decoy-pricing-how-strategic-tier-design-drives-revenue)). "Three tiers only work when the middle tier is genuinely the best value" ([pipelineroad.com](https://pipelineroad.com/agency/blog/saas-pricing-page-best-practices)). A middle tier priced too close to either neighbour fails ([growthhakka.co.uk](https://www.growthhakka.co.uk/2026/07/14/saas-pricing-psychology-killing-the-middle-tier/)). Vendoo, Sellfy and Kit all badge the middle.
- **Yearly discount.** More than half of the companies that discount yearly plans offer 15–20% ([innertrends.com](https://www.innertrends.com/blog/saas-pricing-strategies)). "Two months free" (16.7%) is the common framing ([baremetrics.com](https://baremetrics.com/blog/annual-vs-monthly-pricing-better-retention), [unlocksaas.com](https://unlocksaas.com/benchmarks/annual-vs-monthly-discount)). Companies that *state* a yearly discount average 27% off, median 25% ([suffdigital.com](https://www.suffdigital.com/resources/data-studies/saas-annual-billing-discount)). For indie SaaS, under 10% changes no behaviour, and over 35% "attracts price-shoppers who treat the discount as the value" ([unlocksaas.com](https://unlocksaas.com/benchmarks/annual-vs-monthly-discount)). A concrete dollar saving ("save $720") lifts yearly selection 12–18% over a percentage ([growthspreeofficial.com](https://www.growthspreeofficial.com/blogs/b2b-saas-annual-contract-length-multi-year-discount-benchmarks-2026-impact-on-retention-payback)).
- **Free tier.** Self-serve freemium converts at about 3–5% (good) to 8–12% (great) ([chartmogul.com](https://chartmogul.com/reports/saas-conversion-report/)). Limits should follow usage growth rather than withhold core features, as GitLab caps storage and CI minutes but not core features ([getmonetizely.com](https://www.getmonetizely.com/articles/designing-a-free-tier-that-actually-leads-to-upgrades)). Clear feature differences between tiers go with 37% higher conversion (same source). Edtech-adjacent categories convert about half as well as sharper-pain categories ([fiscallion.io](https://www.fiscallion.io/blog/freemium-conversion-rate-saas)).
- **Overage vs hard cap.** Hard caps suit limits where "hitting the limit pauses the user experience rather than breaks it". They also avoid raising everyone's price to cover a few outliers ([stripe.com](https://stripe.com/resources/more/usage-caps-how-to-protect-performance-and-turn-usage-into-revenue)). Overage has to be explained up front, with alerts at 75 / 90 / 100% ([schematichq.com](https://schematichq.com/blog/overage-pricing-101-base-plans-that-scale-with-usage), [glencoyne.com](https://www.glencoyne.com/guides/fair-use-pricing-fails)).
- **Grandfathering.** Always honour contracted (yearly) prices. Prefer a stated window of 6–12 months over forever, because "forever grandfathering builds a second price list that haunts every future change" ([blakkbear.com](https://blakkbear.com/blog/pricing-reset-b2b-saas/), [rework.com](https://resources.rework.com/libraries/saas-growth/grandfathering-strategy)). At least 45 days' notice reduces churn by about 30% ([getmonetizely.com](https://www.getmonetizely.com/articles/how-can-saas-companies-master-pricing-change-management-without-losing-customers)).

### 2.3 What the category counts as core, counted and premium

Read across the five vendors:

- **Core in every tier**: the product's own verb (Vendoo cross-lists, Kit sends, Buffer schedules) with every destination it supports, plus import, templates, basic organising, export or download, and a support channel. **No cross-lister gates marketplaces or import.**
- **Counted per tier**: whatever grows with the seller's volume. That is items or listings moved, AI or photo credits, scheduled posts, subscribers and storage. The counted axis is almost always the one the price ladder is named after.
- **Starts at a tier**:
  - automation, meaning things that happen without the seller (Vendoo auto offers, Kit automations);
  - deeper analytics (Buffer, Kit);
  - team seats (Buffer Team, Kit Pro, List Perfectly sub-accounts);
  - priority support, and only at the top (Vendoo Pro, Kit Pro, Sellfy Premium).
- **Organising objects** (tags, labels, collections) are capped on free plans and then effectively unlimited (Buffer 3 → 250 on both paid plans; List Perfectly templates unlimited everywhere). **Nobody climbs a ladder on labels.**

## 3. Price points: kept

| Rung | Ours | Category rung | Verdict |
|---|---|---|---|
| Entry | Starter $12 | Vendoo Starter $14.99, Etsy Plus $10, Flyp $9, Listelf $9.99 (`2026-09-26` pack) | inside the band, under the closest competitor |
| Middle | Sync $29 | Vendoo Growth $29.99, Sellfy Starter $29, List Perfectly Simple $29, Crosslist Bronze $29.99 | on the band's centre |
| Top | Studio $59 | Vendoo Pro $59.99, Nifty Pro $59.99, Sellfy Business $59 yearly, List Perfectly Pro $69 | bottom of the band |

The value gaps pass the 2–3× test. Price climbs 2.4× then 2.0×. Moves climb 2.5× (10 → 25) then 4× (25 → 100). Per move, the price is $1.20 → $1.16 → $0.59, so upgrading is never worse per move (pinned by `every_paid_rung_costs_more_and_no_more_per_move`). Nothing in the new evidence argues for moving a price. Moving one would need new Stripe prices plus a grandfathering path, and no signal justifies either.

## 4. Yearly saving: kept at about a third, deliberately above the 15–20% norm

Ours is 33% / 31% / 32%. The category is at 17% (Vendoo, Kit), 20% (Buffer) and 25% (Sellfy). The practice median among companies that state a discount is 25%, with 35% as the line past which the discount attracts price-shoppers.

We keep it, for three reasons:

1. **It is inside the ceiling.** 31–33% is under the 35% price-shopper line (§2.2), and the stated-discount average is 27%.
2. **Teacher-sellers are seasonal.** `[INFERENCE]` Resources are made in the summer and around term starts, so a monthly subscriber has an obvious month to cancel. A year paid up front covers the lull. A deeper yearly saving is the lever for the one churn pattern this market has, and the 2026-09-27 note's rollover (three months of moves) is the same idea from the other side.
3. **Changing it is expensive and unforced.** Moving to "two months free" would make the yearly prices $120 / $290 / $590. That means three new Stripe prices, a migration for every existing yearly subscriber (whose contract price must be honoured, §2.2), and page copy that reads worse ("$24.17 a month").

The page acts on the framing evidence instead. In yearly mode each card says **"Save $48 a year"** in dollars rather than as a percentage, and the default is yearly.

**Re-open** if yearly plans are more than 80% of new subscriptions *and* yearly renewal is under 60% after the first cohort renews. That combination is the price-shopper signature.

## 5. The feature matrix

### 5.1 Inventory: what exists and what is enforced

Every `Capabilities` field, traced to its gate (server first, console second), plus every console feature that has no capability:

| Feature | Where it lives | Gate | Enforced? |
|---|---|---|---|
| Moves (copy or move between marketplaces) | `routes/automations/migration`, `tam-api/src/migrations.rs:243`, `jobs.rs:556` | move balance (`moves_per_month`, `moves_accrual_cap`, `free_moves_lifetime`, packs) | yes |
| Edits sent to every marketplace | `routes/sync`, `sync_settings.rs:96`, `scheduler.rs:144` | `sync_pull_interval_secs` | yes |
| Scheduling | `routes/automations/sharing`, `schedules.rs:282` | `scheduling` | yes |
| Automatic publishing rules | `schedules.rs:389`, `sync_settings.rs:154` | `auto_publish_rules` | yes |
| Term mapping, price rules | `routes/automations/{mappings,pricing}` | none | core |
| Resources | `catalogue.rs:1198` | `resources_max` | yes |
| Storage | `catalogue.rs:391` | `storage_bytes_max` | yes |
| Import (shops, spreadsheet) | `import.rs:88`, `import_batch/mod.rs:424` | `import_*` (true everywhere) | yes |
| Duplicate review | `duplicates.rs:140` | `duplicate_review` (true everywhere) | yes |
| Rich-text descriptions | `panels/DescriptionPanel.svelte` | none | core |
| Watermarked previews | `PreviewMaker.svelte` (drawn in the browser) | none | core |
| Templates | `resource_templates.rs:603` | `templates_max` | yes |
| Collections | `collections.rs:353` | `collections_max` | yes |
| Labels | `resources.rs:1752`, `collections.rs:539` | `labels_max` | yes |
| Export | `export.rs:117` | `export` (true everywhere, test-pinned) | core |
| Statistics | `analytics.rs:78` | `analytics` | yes |
| Devices (desktop app) | `devices.rs:539` | `devices_max` (5 everywhere) | yes |
| AI description fill | `ResourceForm.svelte:1095` "coming soon" chip | `ai_fills_per_month` | **not built** |
| Support | – | `support` | a human promise, no code |
| Team seats | – | – | **does not exist** |
| Bulk edit | `bulk-verbs.ts:43` | – | **not built** |

### 5.2 Classification and every number

**Core: on every plan, including Look.** The product itself costs us nothing per seller, and the category never gates it (§2.3):

- copy or move between marketplaces (metered by moves, never gated)
- import from shops or a spreadsheet
- find and merge duplicates
- rich-text descriptions
- watermarked previews
- term mapping and price rules
- export
- the desktop app

A trial that withholds any of these can't show the product.

**Counted per plan: the ladder.**

| Row | Look | Starter | Sync | Studio | Why this number |
|---|---|---|---|---|---|
| Moves a month | – (5 once) | 10 | 25 | 100 | The ladder's axis since 2026-09-20. 10 is about one new resource a week onto one marketplace (4.3) with room for a second. 25 is a weekly maker onto both. 100 is a whole-shop migrator. Steps of 2.5× and 4× (§3). |
| Unused moves carry over, up to | – | 30 | 75 | 300 | Three months of allowance on every plan: long enough to cover the summer lull (§4), short enough not to become a balance to bank and cancel against. |
| Edits sent to every marketplace | – | daily | every 6 hours | hourly | The one automation that costs us scheduler time, so it scales with price. Every paid plan gets it: keeping listings in step is the promise of the product's name. |
| Resources | 500 | no cap | no cap | no cap | Look's only catalogue ceiling. 500 covers covers-only storage in about 150 MiB (`2026-09-26` capacity note). Every cross-lister leaves items uncapped on paid plans. |
| Storage | 256 MB | 5 GB | 20 GB | 200 GB | Covers and previews only (files stay on the seller's device), about 100 KB per resource. The capacity note's figures are unchanged. |
| Templates | 1 | 5 | 20 | no cap | One on Look to try the feature. The ladder follows moves: a seller moving more needs more reusable setups. |
| Collections | **1** (was 0) | 5 | 20 | no cap | **Changed.** Look had none, so a trial seller could not try a core organising feature. The free-tier evidence says limit by usage, not by withholding (§2.2), and templates already worked this way. |
| Labels | 5 | **no cap** (was 20) | **no cap** (was 20) | **no cap** (was 50) | **Changed.** Labels cost nothing to store and are not a reason anyone upgrades. The old ladder was flat between Starter and Sync (20 / 20), so it was not a ladder. The category caps tags on free and then lifts them (Buffer 3 → 250). A per-resource bound of 20 labels is a separate constant and stays. |
| AI description fill (coming soon) | – | 50 a month | 200 a month | 600 a month | Unchanged, and drawn with a hollow mark, never a tick, until it ships. This matches List Perfectly's AI allowance ladder (25 / 50 / 200 / 1,000). |

**Starts at a plan.**

| Row | Starts at | Why |
|---|---|---|
| Scheduling | Starter | It needs a monthly allowance to be worth anything. Look's five moves are a one-off. |
| Email support | Starter | Look has the guides. A person answering is what a price buys first. |
| Statistics on every shop | **Sync** | It is Sync's pull as the recommended plan (§6), and matters only to a seller publishing often. Vendoo gives analytics to every tier, and that is the risk: the trigger in §9 watches it. |
| Automatic publishing rules | **Sync** | Automation is the category's standard middle-or-top feature (Vendoo auto offers, Kit automations). |
| Priority support, answered within a day | Studio | Priority support sits on the top tier in every vendor that sells it (Vendoo Pro, Kit Pro, Sellfy Premium). |

**Not sold, and why:**

- **Team seats.** None exist, so Studio's copy no longer says "a team". A seat is the category's usual top-tier feature (§2.3). When one is built, it belongs on Studio.
- **Marketplaces and devices.** Both are the same on every plan, and a cross-lister that sells one marketplace isn't one. The table says "Desktop app for your own devices" without a number: the cap of 5 exists for fairness, not for sale.
- **A 90-day free re-edit of a moved listing** (`pack_edit_days`). The old page and FAQ promised it, but no code implements a window or a free re-edit (§10). The new page no longer claims it.

### 5.3 As data

`crates/tam-limits/src/lib.rs`:

- `PLAN_FEATURES: [PlanFeature; 22]`, where each row is `{ key, label, group, unit, soon, included: { free, starter, subscriber, studio } }`.
- A cell is `false` (not on the plan), `true` (included, and on a counted row, uncapped) or a number in the row's `unit` (`count`, `per_month`, `megabytes`, `every_hours`).
- Cells come from `FeatureKey::included(&Plan::capabilities(None))` at compile time. The only hand-written parts are the labels and the order.
- `PLAN_FEATURE_GROUPS` gives the five section headings.
- Tests pin that:
  - no row gets worse up the ladder (an interval counts down);
  - there is exactly one row per key, grouped in order;
  - yes-or-no rows carry no numbers;
  - only the AI fill is `soon`;
  - the core rows are on every plan;
  - exactly one paid plan is recommended;
  - every tagline is one sentence.

## 6. Recommended plan and page layout

**Sync is the recommended plan.** It is the middle of three paid rungs, where Vendoo, Sellfy and Kit put the badge (§2.1), and it is the plan whose features (statistics, rules, 6-hourly edits) match the seller the product is built for: a teacher who adds resources every week.

It must also be the best value. It is the best value on features (statistics and rules start there) and on moves (cheaper per move than Starter). Studio is cheaper per move again, but only for a seller who uses 100 moves a month every month.

The landing page (`Pricing.astro`):

- A monthly / yearly switch that defaults to yearly. It is a pair of native radio inputs, so it works with keyboard and screen readers and without script.
- Each price says "a month, billed yearly $N" or "a month, billed monthly". Yearly mode says "Save $N a year".
- Four cards. Sync is raised, with a "Recommended" badge.
- Each card carries only its tagline, price, call to action and at most five highlights.
- The full comparison is a table under the cards: rows are features grouped by section, columns are plans, and cells are ticks, dashes or figures. On a phone it scrolls sideways with the feature column pinned.
- The Move Packs table and the sale banner with struck prices (`sale.js`) are kept.
- The FAQ is trimmed from eleven questions to six.

## 7. Free tier: Look

- **Shape: freemium with a usage limit, not a trial clock.** No trial days on any row (unchanged). The ChartMogul and ProductLed figures favour freemium for self-serve, and a clock beside a free plan was two offers (the 2026-09-27 decision).
- **Gives:** the whole core (§5.2), 500 resources, 256 MB, five moves once (granted per storefront, so it can't be farmed by a second organisation), 1 template, 1 collection and 5 labels.
- **Withholds:** monthly moves, edit syncing, scheduling, automation, statistics and email support. These are all things that grow with a seller's volume, which is where the evidence says a free limit should bite.

## 8. Overage vs hard cap, and grandfathering

- **Moves are a hard cap with a prepaid top-up, never an overage bill.** When the allowance runs out, the next move waits for the next month or for a Move Pack. Listings already live keep syncing, so hitting the cap pauses one action rather than breaking the shop, which is Stripe's condition for a hard cap. Packs never produce a surprise bill.
  - Open item: the console should warn at 75% and 90% of the balance, the practice alerts in §2.2. That is not in this slice.
- **Every other count** (resources, storage, templates, collections, labels) is a hard cap refused at the gate with an upgrade line. None is metered.
- **This review needs no grandfathering.** No price changes, and every cap change is upward.
- **Policy for a future price change:**
  - A yearly subscriber keeps their price to the end of the paid year.
  - A monthly subscriber keeps it for 12 months from the announcement.
  - The founder emails at least 60 days before the change, saying what changes and why.
  - New prices go to new customers first.
  - No "forever" legacy prices.

## 9. Triggers that re-open this

- Sync takes less than 40% of new paid subscriptions after two quarters. The middle is not the best value, so reconsider statistics on Starter (Vendoo's shape) before touching a price.
- Starter takes more than 60% (the 2026-09-27 trigger, kept).
- Studio stays under 5% of subscribers after two quarters: try $49 / $408 (kept).
- The yearly signature in §4.
- Look converts under 2% within 90 days of signup. Look gives too much: first consider scheduling on paid plans only (it already is), then the 500-resource ceiling.
- A label or collection count from a single organisation above 5,000: add a fair-use bound beside the gate, as Buffer does at 5,000 posts per channel.

## 10. Findings outside the price list

- **`pack_edit_days` (90) is promised and not built.** The console's Account page (`web/src/lib/entitlement.ts:355`) still prints "Moved listings can be edited for 90 days". Either build the free re-edit window or remove the line and the field. This slice removed it from the landing page only.
- **`uploads_in_flight_max` is never read.** The per-organisation upload bound the 2026-09-26 capacity note relies on does not exist. It is not a plan feature and is not in the matrix.
- **`publish_marketplaces_max` is text only**, and it is `u32::MAX` on every plan.
- **The console's `capabilityLines` (`web/src/lib/entitlement.ts:411-419`) prints `u32::MAX` as a number** ("4294967295 collections" on Studio, and now labels on every paid plan). The Billing page should render from `PLAN_FEATURES` instead (BillingAdminUI's slice).

## 11. Stripe

No change. All eleven price keys and amounts stay the same:

- `starter_*`: $12 / $96
- `pro_*` (then `sync_*`): $29 / $240
- `studio_*`: $59 / $480
- `pack_*`: $47 / $77 / $127 / $247 / $397

## 12. Addendum, 2026-09-30: the founder's tier changes (0.17.0)

The founder reviewed the live Billing page and changed these figures. Prices, price keys, the recommended plan and the grandfather rule (§8) do not change.

| | Look | Starter | Pro (was Sync) | Studio |
|---|---|---|---|---|
| Name | Look | Starter | **Pro** | Studio |
| Resources | 100 | **250** (was 500) | **500** (was 2,000) | no cap |
| Watermarked previews | **5 for the account's life** (was 5 a month) | **20 a month** (was 50) | **50 a month** (was 200) | **100 a month** (was no cap) |
| Templates | 1 | 5 | **10** (was 20) | no cap |
| Collections | 1 | 5 | **10** (was 20) | no cap |
| AI description fill | – | **none** (was 50 a month, "coming soon") | 200 a month | 600 a month |

- **Sync is sold as Pro.** The plan id stays `subscriber`, because stored grants carry it. The price keys were `sync_monthly` / `sync_yearly` and became `pro_monthly` / `pro_yearly` in 0.18.0, so the key, the Stripe product, its prices' nicknames and lookup keys all say Pro; subscriptions store Stripe's price identifier, which did not change.
- **Look's previews are a lifetime allowance**, like its five trial moves: `Capabilities::previews_lifetime` is 5 and `previews_per_month` is 0 on Look, and `Capabilities::previews()` says which window a gate reads. The lifetime figure is every month's `usage_counter` row summed, so no second counter can drift from the monthly one, and a plan change needs no conversion.
- **No plan has unlimited previews.** Every preview is a render and a stored image; 100 a month covers a Studio shop refreshed over a term.
- **Starter carries no AI fill.** The offer starts at Pro, so the Starter card no longer says "AI description fill, coming soon".
- The §2.2 value-gap rule still holds on resources (2.5×, 2×, then no cap) and previews (4×, 2.5×, 2×); `every_cap_climbs_the_ladder` and `every_plan_grants_strictly_more_of_each_counted_axis_than_the_one_below` pin it.
- §9's Sync trigger now reads as Pro.
