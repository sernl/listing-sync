# Pricing evidence: product surface, plans, and cost lines

Read-only pass over `~/projects/edtech-workspace/listing-sync` on 2026-09-20. Every repo claim carries `file:line`. `[INFERENCE]` marks anything not stated in a source.

## 1. The plan table as built

Single source: `crates/tam-limits/src/lib.rs`. The landing and console render from `plans.generated.js`, emitted from this crate by `tam-typegen` and diffed by `just web-check` (`apps/landing/src/pricing.js:1-14`).

### Price rows — `PLANS` (`crates/tam-limits/src/lib.rs:402-436`)

| id | name | monthly_cents | yearly_cents | trial_days | sold |
|---|---|---|---|---|---|
| `free` | Free | none | none | 0 | true |
| `subscriber` | Teachouse Subscription | 2_400 ($24) | 24_000 ($240) | 14 | true |
| `migration_only` | Catalogue Import | none (rung-priced) | none | 0 | true |
| `studio` | Studio | 4_400 ($44) | 44_000 ($440) | 0 | **false** |

`sold: false` means no checkout may route to Studio and no surface may render it (`lib.rs:347-352`, `apps/landing/src/pricing.js:28-33`).

### Catalogue Import ladder — `IMPORT_LADDER` (`lib.rs:438-468`)

| up_to | price_cents | per resource |
|---|---|---|
| 20 | 4_700 ($47) | $2.35 |
| 50 | 7_700 ($77) | $1.54 |
| 100 | 12_700 ($127) | $1.27 |
| 250 | 24_700 ($247) | $0.99 |
| 500 | 39_700 ($397) | $0.79 |

Above 500: `LADDER_ABOVE = "Talk to us"` (`lib.rs:470-473`), appended as a display-only open rung (`apps/landing/src/pricing.js:44`). The rung counts "resources added to your catalogue after duplicates are merged" (`apps/landing/src/pricing.js:61-62`; decided `docs/design/decisions.md:816`). A test pins rung == `resources_max` == `migrations_per_month` (`lib.rs:739-747`).

### Capabilities per plan — `Plan::capabilities` (`lib.rs:126-241`)

| field | Free | Subscriber | MigrationOnly (rung `n`) | Studio |
|---|---|---|---|---|
| `resources_max` | 20 | 400 | `n` (0 if no rung) | `u32::MAX` |
| `marketplaces_max` | 1 | MAX | MAX | MAX |
| `storage_bytes_max` | 1 GiB | 20 GiB | 5 GiB | 200 GiB |
| `import_spreadsheet` | true | true | true | true |
| `import_marketplace` | false | true | true | true |
| `duplicate_review` | false | true | true | true |
| `publish_marketplaces_max` | 1 | MAX | MAX | MAX |
| `edit_days_after_purchase` | none | none | **30** | none |
| `migrations_per_month` | 0 | 20 | `n` | 100 (operator rung may raise) |
| `scheduling` | false | true | false | true |
| `sync_pull_interval_secs` | none | 21_600 (6 h) | none | 3_600 (1 h) |
| `auto_publish_rules` | false | true | false | true |
| `templates_max` | 1 | 20 | 1 | MAX |
| `collections_max` | 0 | 20 | 0 | MAX |
| `labels_max` | 5 | 20 | 0 | 50 |
| `analytics` | false | true | false | true |
| `export` | **true** | true | **true** | true |
| `devices_max` | 1 | 2 | 1 | 3 |
| `ai_fills_per_month` | 0 | 200 | 0 | 600 |
| `support` | `guides` | `email_2_days` | `email_30_days_after_purchase` | `email_1_day` |

Notes carried in code: `u32::MAX` is the no-ceiling sentinel (`lib.rs:325-335`); `export` is never false on any plan and a test pins it (`lib.rs:326-329`); plan strength Free 0 < MigrationOnly 1 < Subscriber 2 < Studio 3 decides which of several grants wins (`lib.rs:113-122`); a `migration_only` grant with no rung grants **nothing** rather than the largest rung (`lib.rs:106-110`, `lib.rs:181-185`). Studio trigger: ships when a fifth of subscribers exceed 300 resources or hit the migration cap twice in a quarter (`lib.rs:41-45`, `decisions.md:818`).

### AI packaging — `AI` (`lib.rs:490-495`)

`status: ComingSoon`, `included_fills: 200`, `add_on_fills: 100`, `add_on_cents: 500` ($5). `AiStatus` is a closed one-variant enum so shipping it is a compile error at every "coming soon" surface (`lib.rs:381-395`). No credit currency (`decisions.md:827`).

## 2. The founding offer as built

`FOUNDING` (`lib.rs:477-483`):

- `discount_year_one_pct: 25`
- `discount_ongoing_pct: 20`
- `ongoing_years: 3` — capped by the founder on 2026-09-12 (`decisions.md:817`); originally perpetual (`decisions.md:771`)
- `free_imports: 20`
- `places: 100`

Rendered verbatim in the FAQ from these constants (`apps/landing/src/pricing.js:152-155`). Effective price computed in research: $18.00/mo or $180/yr year one, $19.20/mo or $192/yr ongoing; both above Paddle's under-$10 custom-pricing line, Paddle's 50c fixed is 2.8% of the discounted monthly (`docs/notes/design/research/2026-09-12-pricing-and-tiers.md:189-191`). For a subscriber the "20 free imports" clause is already true (import is included); for a `migration_only` buyer it zeroes the $47 rung (same file, :191). Risk recorded: 100 permanent-20% places ≈ $5,760/yr foregone at list and anchors $19.20 as the real price (`:...§7 risk 3`).

## 3. Feature inventory, by group, with the gating field

Gates are enforced server-side in `tam-api` against `context.entitlement.caps` and mirrored in the console by `web/src/lib/entitlement.ts`.

### Import
| feature | gate field | enforcement |
|---|---|---|
| Spreadsheet import (xlsx upload, ≤8 MiB, ≤500 rows across tabs) | `import_spreadsheet` | `tam-api/src/import_batch/mod.rs:424-427`; bounds `tam-limits/src/lib.rs:547-563` |
| Marketplace catalogue import (read your shop) | `import_marketplace` | `tam-api/src/import.rs:88-91`, `import_runs.rs:601-604`, `:1050-1053` |
| Duplicate review / merge before commit | `duplicate_review` | `tam-api/src/duplicates.rs:140-143`, `:251-254`; commit path `import_batch/commit.rs:350` |
| Import binds each resource to its source listing (so re-reads skip) | ungated | `decisions.md:846-847` |
| Storage for imported bytes | `storage_bytes_max` | `tam-api/src/catalogue.rs:371-377` |
| Batch expiry sweep at 14 days | ungated bound | `tam-limits/src/lib.rs:565-575` |

### Publish / sync
| feature | gate field | enforcement |
|---|---|---|
| Resource count ceiling | `resources_max` | `tam-api/src/catalogue.rs:1170-1177` |
| Marketplaces connected | `marketplaces_max` | `tam-api/src/devices.rs:440-448` |
| Publish fan-out per resource | `publish_marketplaces_max` | plan table `lib.rs:135,150,...` |
| Migrations (copy **and** move), metered in resources not batches | `migrations_per_month` | `tam-api/src/migrations.rs:167-171,230-237,420-425`; `jobs.rs:559-571`; both kinds consume it (`decisions.md:850`) |
| Scheduling (timetabled publishes) | `scheduling` | `tam-api/src/schedules.rs:283-288`, `scheduler.rs:126` |
| Sync pulls (cadence floored at plan interval) | `sync_pull_interval_secs` | `tam-api/src/sync_settings.rs:98,144-147`, `scheduler.rs:144-146`; floor rule `decisions.md:866` |
| Auto-publish / republish-on-update rules | `auto_publish_rules` | `schedules.rs:403-406`, `sync_settings.rs:159-161,173-177`, `scheduler.rs:151` |
| Edit window after a one-off purchase | `edit_days_after_purchase` (30, MigrationOnly only) | plan table `lib.rs:190` |
| Seller pricing rules (USD→GBP ×0.75 preset, ECB rate option, rounding) | ungated | `tam-domain/src/seller_rules.rs:1726-1748`; decision `decisions.md:881-884` |
| Outbound pacing 30 req/min per marketplace | ungated bound | `tam-limits/src/lib.rs:611-621` |

### Catalogue management
| feature | gate field | enforcement |
|---|---|---|
| Templates (any subset of the new-resource form, generic or marketplace-scoped) | `templates_max` | `tam-api/src/resource_templates.rs:602-610`; shape `decisions.md:870-872` |
| Collections (ordered reference lists; publish, apply-template-and-labels, export) | `collections_max` | `tam-api/src/collections.rs:352-359`; shape `decisions.md:838-841` |
| Labels | `labels_max` | `collections.rs:539-547`, `resources.rs:1599-1607` |
| Analytics | `analytics` | `tam-api/src/analytics.rs:78-82` |
| Export (CSV of catalogue with per-marketplace status, price, link) | `export` — **never false, never gated** | `lib.rs:326-329`; `decisions.md:819`; page scope `docs/notes/design/console-redesign-plan.md:25` |
| Device-local TPT preview builder (`pdfjs-dist`, `pdf-lib`) | ungated; TPT preview slot not captured | `decisions.md:790-796` |

### AI
All of it is unbuilt and marked "coming soon" on both the pricing page and the form (`decisions.md:828`; `apps/landing/src/pricing.js:97-102,147-150`).

| feature | gate | source |
|---|---|---|
| Auto-fill from the seller's own file (fact sheet extracted on-device; file never leaves it) | `ai_fills_per_month` (200 Subscriber / 600 Studio / 0 Free & MigrationOnly) | `lib.rs:159,215`; `decisions.md:825-827` |
| Add-on: 100 extra fills for $5 | `AI.add_on_fills/add_on_cents` | `lib.rs:490-495` |
| Roadmap items 4–10 (tag suggestions, GB↔US adaptation, preview-page ranking, analytics narratives, pricing suggestions, standards evidence, bundle candidates) | all "Subscriber" | `docs/notes/design/research/2026-09-12-ai-roadmap.md:332-338` |
| Item 11, help over guides | **Free** | same file, :339 |
| Charter limit: models only in listing-copy generation and selector rediscovery; classification/standards alignment/image gen/help chat need a recorded amendment | n/a | `ai-roadmap.md:15-18`; `decisions.md:825` |

### Devices / sessions
| feature | gate field | enforcement |
|---|---|---|
| Enrolled devices (desktop/Android holding marketplace sessions) | `devices_max` | `tam-api/src/devices.rs:485-492`, `:681-688` |
| Marketplace sign-in in the seller's own window; no password stored | ungated | `apps/landing/src/pricing.js:111-114`; `decisions.md:408-411` |
| Entitlement token minted to the device by heartbeat (Ed25519) | mirrors `Capabilities` | `tam-api/src/lib.rs:110-112`; `decisions.md:819` |
| Desktop modules: connect, webview_session, console_session, import, work, ledger, scheduler, library/library_sync, transfer, heartbeat, entitlement, notify, updater | — | `apps/desktop/src-tauri/src/*.rs` |

### Support / ops
| feature | gate field | source |
|---|---|---|
| Support level (guides / email 2 days / email 1 day / email 30 days post-purchase) | `support` | `lib.rs:249-277` |
| Guides (global table, server-rendered, HTML escaped) | free to all | `decisions.md:879` |
| Per-marketplace status page (support-cost containment) | ungated | `docs/design/milestones.md:115` |
| Backoffice: `/admin/users`, `/admin/orgs`, `/admin/failures`, `/admin/health`, `/admin/import-drain`, `/admin/impersonations`, `/admin/guides` | operator only | `web/src/routes/admin/**`; `decisions.md:878` |
| Operator grant/revoke of any plan or rung with reason, expiry, audit row | operator only | `decisions.md:820`; `tam-api/src/admin.rs:462-465` |
| No product-analytics collector (backoffice reads Postgres); revisit at 100 sellers | — | `decisions.md:833-834` |

### Compliance
| item | state | source |
|---|---|---|
| Merchant of record = Paddle; webhook signature is the authentication | built | `tam-api/src/billing.rs:1-60`, `paddle.rs:1-35` |
| Paddle prohibits products that infringe third-party terms and bans CAPTCHA solving; two MoR candidates prohibit this product by name | contradiction on record | `docs/design/compliance-floor.md:89-94` |
| Stripe written pre-approval under SSA 1.2(a)(ix) is a Stage A gate before any billing code | pending | `compliance-floor.md:30,96-97`; `docs/design/runbook-m1j-first-charge.md:170-174` |
| Second processor onboarded dormant + one month operating cost off-processor | required | `compliance-floor.md:59,103` |
| Refunds Stripe MP may issue within 60 days outside founder control | modelled | `compliance-floor.md:102`; `runbook-m1j-first-charge.md:202` |
| Operating entity and jurisdiction undecided | open | `decisions.md:78` |
| Marketplace names/logos shown under a non-affiliation disclaimer | decided | `decisions.md:449-450` |
| Uncalibrated-constant budget must be zero before the first charge; currently 0 | satisfied | `tam-limits/src/lib.rs:663-682` |
| No marketplace permission enquiries sent; TPT connector proceeds without written permission | decided | `decisions.md:61-62,200-201` |

## 4. Per-marketplace capability matrix

| verb | TPT (`tam-marketplace-tpt`) | Tes (`tam-marketplace-tes`) |
|---|---|---|
| Import / list own catalogue | **supported** — `MyProductListings` GraphQL, `list_own_resources` `flows.rs:205`, observed variant `:234` | **supported** — `getAllResources` + `getAllDrafts`, `flows.rs:1000`,`:1033`; `endpoints.rs:795,802` |
| Fetch one listing for import | **supported** — `flows.rs:1044`; no licence field exists on TPT | **supported** — `flows.rs:1354` over `GET /resources/{id}/draft` |
| Create | **supported** — 13-hop create: form tokens, S3 reserve, signed multipart, two async jobs, 43-field multipart POST answering 302 with the id (`lib.rs:11-22`); exactly **one file** into the product slot (`flows.rs:675-686`) | **supported** — `create_listing` `flows.rs:191`; `POST /api/v2/resources` then `/draft` metadata (`endpoints.rs:371`) |
| File upload | **supported** — S3 multipart with per-call clock/signature (`s3.rs`, `upload.rs`) | **supported** — `upload_file` `flows.rs:268`, presigned direct-to-S3 |
| Publish | **supported** — the 48-field edit form with its status selector moved (`flows.rs:798`) | **supported** — `publish` `flows.rs:395`, verified by draft-flag read-back |
| Update (draft) | **supported** — `update` read-modify-write on the edit render (`flows.rs:712`) | **supported** — `update` = re-post metadata (`flows.rs:330`) |
| Update (live listing) | **supported** (edit form is the same for live) | **UNSUPPORTED** — `Uncaptured { capability: "tes.edit_published" }` (`flows.rs:890-892`); scheduler skips a live Tes listing until captured (`decisions.md:864`) |
| Unpublish / delist | **partial** — no unpublish verb; `delete` is `RemoveResource` GraphQL (`flows.rs:815`, `lib.rs:23`) | **UNSUPPORTED** — `Uncaptured { capability: "tes.unpublish" }` (`flows.rs:893-895`) |
| Delete | **supported** — `RemoveResource`, answer echoes the removed id | **supported** — draft and live routes, with state probe (`flows.rs:423,433,443,476`); 2xx is never the verdict (`lib.rs:6-9`) |
| Cover / thumbnail upload | **generated only** — `generate_thumbnail` + `thumbs_collection_key`; the four manual thumb slots, preview slot and video slot are named by the form and exercised by no capture (`write_model.rs:322-352,402-434`; `flows.rs:675-677`) | **supported** — two hops: bytes to Uploadcare (Tes's own widget/public key), then `upload-cover-image`, set as `customThumbnails` (`flows.rs:214`; `endpoints.rs:627-660`) |
| Pricing on write | **supported, one leg inferred** — both captured creates are free, so the paid create reuses the captured paid edit's money fields (`lib.rs:29-33`); `ListingPrice`/`PaidPrice`/`TaxCode` in `write_model.rs` | **supported** — price is an integer of minor units, GBP, fixed by account (`decisions.md:725-726,809`) |
| Taxonomy | **supported** — `standards.rs`, `TptCategory` in `read_model.rs` | **supported** — one GB tree, mechanical id prefix per country (`decisions.md:807-809`) |
| Analytics read | **supported** — `storeResourceTotalsAllTimeStats` on `/gateway/graphql` (`lib.rs:5-7`); `ResourceStat` | **NOT BUILT** — both device read routes are fixed to TPT; the Analytics page states the gap (`docs/notes/design/console-redesign-plan.md:94`) |
| Own-file download | **capture-gated** — signed `rc-assets` redirect observed 2026-09-13 on an Android WebView (`decisions.md:389`) | draft read serves metadata; file bytes for customer zero came from disk (`docs/design/plans/2026-08-25-m1j-…:9-10`) |
| Share / bump / relist | **absent both sides** — no such concept in the codebase and no captured endpoint; the Sharing page ships as shape only (`console-redesign-plan.md:95`) | same |
| Etsy / Shopify | disabled as source and target; API adapter unimplemented (`decisions.md:849`) | — |

Refusal semantics: an uncaptured capability is refused, never approximated (`tam-marketplace-tes/tests/flows.rs:1340-1343`); an `Uncaptured` submit terminates as `Skipped` rather than a marketplace rejection (`tam-domain/src/lib.rs:1403-1407`).

## 5. External cost lines

### Hard-coded / in-repo constants
| line | value | file:line |
|---|---|---|
| LLM spend ceiling per tenant per day | 500 cents ($5) — a loss ceiling, not a token price; "recompute against the provider's actual per-token price once one is chosen" | `crates/tam-limits/src/lib.rs:603-608` |
| Uploadcare (Tes cover stage) | **$0 to us** — Tes's own widget and public key `5f93e59f109ebf8e3ed6`, no credential of ours travels | `crates/tam-marketplace-tes/src/endpoints.rs:627-652`; `live.rs:43-46` |
| Outbound marketplace pacing | 30 requests/minute (legal, not cost) | `tam-limits/src/lib.rs:611-621` |
| Storage bounds priced into plans | 1/5/20/200 GiB | `tam-limits/src/lib.rs:133,168,186,222` |
| Upload ceiling | 256 MiB (Tes per-file ceiling 200 MB + headroom) | `tam-limits/src/lib.rs:501-506` |
| Paddle price map | runtime file `--paddle-price-map`, empty by default; unknown price grants nothing | `tam-api/src/lib.rs:104-109`; `billing.rs:84-98` |

### Modelled cost per user per month (`docs/design/commercial-model.md:49-61`)
| line | 100 users | 300 | 1,000 |
|---|---|---|---|
| AI tokens | $0.49 | $0.49 | $0.49 |
| Automation compute | $0.00 | $0.00 | $0.00 |
| Server and pipeline compute | $0.70 | $0.45 | $0.27 |
| Storage | $0.02 | $0.05 | $0.06 |
| Payments at 6.5% on $29 | $1.89 | $1.89 | $1.89 |
| Email, DNS, monitoring | $0.17 | $0.15 | $0.13 |
| Refunds and disputes | $0.52 | $0.52 | $0.52 |
| Support labour | $4.00 | $2.50 | $1.50 |
| Fixed compliance and legal, amortised | $5.17 | $1.72 | $0.52 |
| **Total** | **$12.96** | **$7.77** | **$5.38** |

Caveats on record: automation compute is $0 because it runs on hardware the founder owns — "not separately billed rather than free"; managed-browser counterfactual was $0.50–$1.70/user/mo (`:66`). Payments is the AU international 6.5% all-in, ~10.0% on Stripe Managed Payments, and MoR fees are charged on the **tax-inclusive** total, so a UK cohort at 20% VAT costs more than headline (`:67`). Support labour is $50/hr × 12 min/ticket, 0.4→0.15 tickets/user/month, no citable benchmark (`:68`). Break-even 250–300 customers against a $5,000/mo founder opportunity cost (`:69`). LTV at $29: $459 at 5% churn, $656 at 3.5%, $287 at 8%; affordable CAC $96–$219 (`:70`).

### Third-party vendor rates (external, dated)
| vendor | rate | source |
|---|---|---|
| Paddle (MoR, chosen) | 5% + 50c pay-as-you-go, no monthly fee; sub-$10 products need custom pricing; whether 5% is on gross-including-tax is **unresolved** | `docs/notes/design/billing-vendor-memo.md:56-59` |
| Stripe cards (NZ) | 2.65%/2.7% + NZ$0.30 domestic, 3.5%/3.7% + NZ$0.30 international (two Stripe pages conflict), +2% on conversion; Billing 0.7%; Tax Basic 0.5%/tx | `billing-vendor-memo.md:44-47` |
| Stripe Managed Payments | NZ not a supported business location | `billing-vendor-memo.md:33-35` |
| Hosted model, AI auto-fill | GPT-4.1 mini $0.40/$1.60 per Mtok → **$0.0033/resource**; GPT-4.1 nano → $0.00083 | `docs/notes/design/research/2026-09-12-ai-roadmap.md:286-291` |
| Self-hosted alternative | $30–$100/mo incremental CPU/RAM → $0.015–$0.05/resource at 2,000 resources; break-even 9,000–30,000 resources/month | `ai-roadmap.md:300-302` |
| TPT seller fees (customer's own cost) | Basic $29 one-time / 55% / $0.30 per resource; Premium $59.95/yr / 80% / $0.15 under $3 | `2026-09-12-pricing-and-tiers.md` §3 [12] |
| Tes royalties (customer's own) | Bronze 60% £0–999.99, Silver 70% £1,000–5,999.99, Gold 80% £6,000+ on rolling 12 months; 20p under £3; price £1–£300 | same, [13] |
| Canary seller account | $29 one-time TPT Basic Seller | `docs/design/compliance-floor.md:32` |

## 6. Open pricing decisions already on record

1. **Studio is priced but unsold.** Ships only when a fifth of subscribers exceed 300 resources or hit the migration cap twice in a quarter; called "the only pending pricing decision" (`tam-limits/src/lib.rs:399-401`, `:41-45`; `decisions.md:818`).
2. **The migration cap re-opens** on the first subscriber who hits 20/month twice in a quarter — same trigger as Studio (`tam-limits/src/lib.rs:95-99`).
3. **Subscription vs. one-time-plus-lower-recurring is unsettled.** $29/mo floor vs $299 one-time + $19/mo (LTV $518 vs $237 at 6% churn); a per-listing credit meter is the third structure named. Choosing is a founder decision (`commercial-model.md` §Pricing; `docs/design/runbook-m1j-first-charge.md:185-186`). Note the built price is $24, not $29 — the whole cost model and LTV arithmetic are computed at $29 [INFERENCE: the models are stale relative to the shipped price].
4. **Card-on-trial.** 14 trial days are in the table (`lib.rs:418`); requiring a card (8% vs 30% conversion, ChartMogul) is recommended and not visibly encoded (`commercial-model.md` §Pricing; `2026-09-12-pricing-and-tiers.md` §6).
5. **Founding ongoing discount** capped at 3 years on 2026-09-12 (`decisions.md:817`); the alternative — perpetual but bound to the tier purchased — was offered and not taken (`2026-09-12-pricing-and-tiers.md` §7 risk 3).
6. **Processor risk is live.** Paddle's AUP prohibits products that infringe third-party terms and bans CAPTCHA solving, so the chosen MoR prohibits this product by name; the Stripe written pre-approval is a hard gate that has not been recorded as answered (`compliance-floor.md:89-97`; `runbook-m1j-first-charge.md:170-174`).
7. **Operating entity and jurisdiction undecided**, forking privacy, consumer-law, insurance and customer terms — and therefore the tax treatment behind any price (`decisions.md:78`).
8. **Charging is postponed**; no automated billing until ~50 customers, first charge by manual link after the pre-approval reply (`decisions.md:169`; `milestones.md:116`).
9. **Import-included cannibalisation.** Import is bundled for subscribers; the 20-resource monthly migration cap is the only instrument stopping subscribe-migrate-cancel, and a one-month $24 must never beat the $47 rung (`decisions.md:816`; `2026-09-12-pricing-and-tiers.md` §7 risk 1).
10. **Quantity, not price, is the exposure.** At ~$27/mo typical TPT seller income, $24 is a top-decile product: the subscription market is nearer 400 than 4,000, against a 250–300 break-even; `migration_only` must carry the other ~3,600 (`2026-09-12-pricing-and-tiers.md` §7 risk 2).
11. **Unpriced/unsold surfaces that pricing depends on:** AI auto-fill promises no date and no accuracy figure (`decisions.md:828`); Tes analytics does not exist (`console-redesign-plan.md:94`); Sharing/bump/relist has no endpoint on either marketplace (`:95`); Tes live edit and unpublish are uncaptured, so a subscriber's Tes sync is partly inert.
12. **Untapped pricing lever on record:** Tes's rolling royalty bands (60→70% at £1,000) are the sharpest ROI argument and appear nowhere in the copy (`2026-09-12-pricing-and-tiers.md` §7 closing; `decisions.md:130` of the workflows note).

## Sources

- `crates/tam-limits/src/lib.rs` (plan table, ladder, founding, AI, bounds)
- `apps/landing/src/pricing.js`; `web/src/lib/entitlement.ts`; `web/src/routes/**`
- `crates/tam-api/src/{analytics,catalogue,collections,devices,duplicates,import,import_runs,jobs,migrations,resource_templates,resources,scheduler,schedules,sync_settings,billing,paddle,entitlement,admin,lib}.rs`; `crates/tam-api/src/import_batch/{mod,commit}.rs`
- `crates/tam-marketplace-tes/src/{lib,flows,endpoints,live}.rs`; `crates/tam-marketplace-tpt/src/{lib,flows,write_model}.rs`; `crates/tam-marketplace/src/lib.rs`; `crates/tam-domain/src/{lib,seller_rules}.rs`
- `apps/desktop/src-tauri/src/*.rs`
- `docs/design/{commercial-model,decisions,compliance-floor,milestones,runbook-m1j-first-charge}.md`
- `docs/notes/design/{billing-vendor-memo,console-redesign-plan,2026-09-12-one-marketplace-per-site-and-the-seller-workflows}.md`
- `docs/notes/design/research/{2026-09-12-pricing-and-tiers,2026-09-12-ai-roadmap,2026-09-12-dedup-and-collections}.md`
- External figures inside those research files were fetched by their authors on 2026-09-12 and are cited there with URLs; not re-verified in this pass.
