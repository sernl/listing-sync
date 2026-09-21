# Pricing model re-evaluation: what Teachouse sells, what it costs, and the model that turns it into an income

- date: 2026-09-20
- status: proposal for the founder; supersedes `2026-09-18-pricing-restructure-and-founding-100.md` where the two disagree, and re-opens decisions the 2026-09-11/12 entries in `docs/design/decisions.md` closed
- brief: re-think the whole model from scratch against (a) what is now proven to work, (b) the true cost of running it, (c) the market position, under the constraint that the founder needs profit quickly and near-zero cost so that this becomes a full-time income
- evidence: three packs gathered 2026-09-20, filed beside this note as `2026-09-20-pricing-evidence-{product,infra,market}.md`; every number below points at one of them or at a repo path

---

## 1. Conclusion first

**Sell moves, not seats.** The unit the seller values, the unit that costs us anything, and the unit they already reason in is *a resource moved from one marketplace to the other, with cover, adapted description, mapped taxonomy and converted price*. Price that unit. Sell it two ways: as a one-off pack (the bulk of revenue for the next twelve months) and as a monthly allowance inside a subscription whose real job is keeping the two shops in step afterwards.

Everything below follows from six facts:

1. **The product now works end to end.** 154 resources went TPT → Tes live with covers in 88 minutes of wall clock and zero failures, and 30 more before that, with the desktop app on a wired workstation. That is 51 hours of a teacher's time at ~20 min a listing `[INFERENCE: hand-listing time; no published figure]`, or roughly $3,000 of Fiverr VA gigs at the $10–20 per upload plus $10 per cover rates on the market today (market pack §3).
2. **It costs us almost nothing to run.** The whole homelab draws ~100 W ≈ £19/month; Teachouse's marginal share is under £1/month and stays under £26/month at 100× today's load (infra pack). Worker CPU during the 308-operation trial was under 0.5%; wall time is entirely marketplace latency. Server-side storage is ~1 MiB a resource because bundles stay on the seller's device. An AI metadata fill costs $0.0006–$0.016 (market pack §5). The only real costs are Paddle's 5% + 50c and the founder's support time.
3. **We are alone, and demand is latent, not articulated.** No competitor cross-lists any two education marketplaces. No indexed thread anywhere asks for one by name (market pack §7). The pain is expressed as "is Tes even worth it", and the documented seller behaviour is a 10–20 resource test catalogue watched for 3–6 months. Pricing must make the first purchase small and the second purchase obvious.
4. **The buyer is a top-decile seller or nobody.** The typical active TPT seller clears ~$27/month; sellers who work at it hold 40–200 resources and clear $300–1,000/month; the dual-listers we measured hold ~764 TPT listings each (product pack §5, market pack §2). Any recurring price above $10 is a top-decile product; the other nine deciles buy once or not at all.
5. **The recurring need is thin and the burst is huge.** The commercial model already names it: "a migration tool wearing a subscription's clothes"; the initial move is ~20× the steady-state monthly need, so monthly subscribers churn at month two or three. A subscription priced monthly against that shape leaks; a subscription billed yearly, with the burst sold separately, does not.
6. **Infra is not a scaling problem for a year; RAM is the only ceiling.** The two 8 GB nodes have ~2.8 GiB free and swap; Postgres pods are BestEffort. That caps concurrency, not seller count, and needs a requests/limits fix rather than money (infra pack, "Biggest capacity ceiling").

---

## 2. What we are actually selling

A feature is worth pricing only if it is built, it is gated, and a seller would pay for it. Scored from the product pack (§3–4):

| Feature | Built | Seller value | Our marginal cost | Verdict |
|---|---|---|---|---|
| Marketplace catalogue import (read your whole shop, either side) | yes | high: "see everything in one place" | storage ~1 MiB/resource, no compute | **give away**; it is the funnel and it costs nothing |
| Duplicate review / merge before commit | yes | medium: protects them from double listings | none | include on every paid path (already decided) |
| Cross-list create with cover, plain-text description, taxonomy map, USD→GBP rule, licence | yes, both directions | **the product**: 20 min → 24 s per resource | marketplace latency only | **the priced unit** |
| Publish / go live | yes, both | part of the move | none | part of the unit |
| Update draft | yes, both | low today | none | included |
| Update live listing | TPT yes; **Tes uncaptured** | medium | — | do not promise "keeps prices in sync" on Tes until captured |
| Delist / unpublish | TPT via delete; **Tes uncaptured** | medium | — | same caveat |
| Delete | yes, both | low | none | included |
| Scheduling, sync pulls, auto-publish rules | yes | medium for active sellers who add resources weekly | scheduler only | **subscription-only** |
| Templates / collections / labels | yes, capped per plan | low–medium | none | subscription-only above token counts |
| Analytics | TPT only; Tes not built | medium (documented upgrade driver in comparables) | none | subscription-only; label it "TPT analytics" honestly |
| Export (CSV) | yes | trust | none | never gated (already decided) |
| AI auto-fill / UK adaptation | **not built** | high if it does the US→UK rewrite (spelling, year groups, KS/GCSE) | cents | do not price until shipped; when shipped, bundle |
| Devices (desktop/Android holding sessions) | yes | necessary, not valued | none | stop metering; 5 silent cap |
| Support | email | trust | **founder hours: the only real cost** | price the tier by support promise, not SLA |
| Sharing / bump / relist | not built either side | — | — | remove from any page |

Two things the seller cannot buy elsewhere and we should name on the page: the Tes royalty-band distance ("£X more sales and every later sale pays 70% instead of 60%") and the £3 fee trap (a $3.99 TPT resource at ×0.75 lands at £2.99 and pays 20p a sale; the pricing rule should round it to £3). Both are built from data we already hold and no VA or competitor can compute them.

---

## 3. The model

### 3.1 One currency: moves

A **move** is one resource committed to the other marketplace, counted at commit after duplicate merge, whether it ends as a draft or live. Covers, description adaptation, taxonomy, pricing rule and licence are inside the move. Moves are bought as packs or accrue monthly on the Sync plan. Unused moves expire twelve months after purchase (Zipsale's listing-credit precedent, market pack §1). No other metered currency: no credits for AI, no device counting, no per-marketplace fees.

### 3.2 Four ways to be a customer

| | **Look** (free) | **Move packs** (one-off) | **Sync** (yearly, or monthly) | **Studio** (unsold) |
|---|---|---|---|---|
| price | $0 | $47 / 20 · $77 / 50 · $127 / 100 · $247 / 250 · $397 / 500 · quote above | **$240 / yr ($20/mo)** · $29 monthly | $440 / yr · $44 monthly, when triggered |
| catalogue import, both marketplaces, read-only | unlimited (storage-capped at 500 resources) | unlimited | unlimited | unlimited |
| moves | **5, once, for life** | the pack, 12-month validity, top-ups at the rung's per-resource rate | **25 / month, roll up to 75** | 100 / month |
| edit on marketplace after a move | — | 90 days | always | always |
| scheduling, sync pulls (6 h), auto-publish rules | — | — | yes | yes, hourly |
| templates / collections / labels | 1 / 0 / 5 | 1 / 0 / auto-labels | 20 / 20 / 20 | unlimited |
| TPT analytics, Tes band calculator, £3 flags | calculator + flags | calculator + flags | all | all |
| AI adaptation (when shipped) | — | one fill per move in the pack | 200 fills / month | 600 |
| export | always | always | always | always |
| devices | 5, unadvertised | 5 | 5 | 5 |
| support | guides | email, 30 days after purchase | email | email, priority |

### 3.3 Why each row is where it is

**Look is the trial, and there is no other trial.** Import is free forever because it costs ~1 MiB a resource and it is the "see your whole shop in one place" moment. Five lifetime moves let a seller watch one real listing appear on the other side with its cover — that is the "aha", and it is well below the 10–20 resource test catalogue the market pack documents, so it cannot substitute for the product. No 14-day clock, no card capture, no trial-expiry support tickets. ChartMogul's opt-in/opt-out figures (8.9% vs 31.4%) argue for a card-required trial on conversion, but the desktop-app install is already the friction that qualifies buyers, and every crosslister comparable runs no-card; adding a second gate on top of the install would be the outlier (market pack §6).

**Packs carry the business for the first year.** Nine deciles of sellers cannot pay a recurring $20 and do not need to; they have a catalogue to move once. The rungs stay as built (`tam-limits` `IMPORT_LADDER`) because they are readable, approved, live on the page, and sit on the one proven cheque this persona writes (Mr Joel's $79.99 one-time, 180 ratings). What changes is the semantics: a pack is a balance of moves, debited at commit, with a 90-day edit window (Tes live edit is uncaptured, so 30 vs 90 days costs nothing and reads generous). Per-resource the rungs run $2.35 → $0.79, against a manual substitute of $10–30 per resource; there is room to raise them later, none to lower them.

**Sync is yearly-first, and the monthly price is the penalty.** $240/yr is the price already approved; $29 monthly (up from $24) is priced so that the page can say "$20 a month, billed yearly" and make the annual the obvious choice. The reasons are all cash: annual takes twelve months of revenue before the month-2–3 churn hazard bites; Paddle's fixed 50c costs 4 percentage points less on annual (market pack §4); and cash up front is what turns this into a full-time income. $29 monthly sits on Vendoo Growth ($29.99) and inside the $18–39 tool band this persona already pays (market pack §3).

**Sync includes 25 moves a month, rolling to 75.** That is the subscribe-migrate-cancel guard: one month at $29 buys 25 moves (~$1.16 each), which is dearer than every rung above 50, so the ladder is never undercut, while a seller adding five resources a week never touches a pack. It replaces the 20-a-month cap that blocked our own trials (the cap was only ever there to stop the leak; the leak is now stopped by the allowance size and by yearly billing). It also removes the `migration_only` plan as a separate plan: a pack buyer is a Look account with a balance, which is one fewer plan to gate, render and explain.

**Studio stays unsold, with a real trigger.** It has the code and the price; what it lacks is anyone straining Sync. Sell it on the first of: ten paying Sync accounts, or one "quote" request above 500 resources. Until then the "Talk to us" rung routes to the founder, which is where the most is learned per hour.

**Devices are not a plan line.** Nobody in the category meters devices; the token binding stays for revocation; the cap is five and unspoken.

**AI is bundled, not metered, and not on the page until built.** A fill costs cents (market pack §5) and every comparable bundles text AI. The "coming soon" chip stays; the plan table's `ai_fills_per_month` stays as a fair-use bound, never as a purchasable currency.

### 3.4 Founding 100, re-cut for cash

Keep the offer, make it **annual-only**, and bound it:

- **$180 for the first year** of Sync (25% off $240), **$192 for years two and three** (20% off), list price from year four — stated on the page at sale time.
- **20 moves free** on top of the monthly allowance (the best-received line in the offer; keep it in whole units).
- **Closes at 100 members or 2026-12-31**, whichever first.
- **The price of the discount is data:** members agree to share their Tes sales for twelve months. That is the only instrument that will ever settle whether the 2.3% Tes-to-TPT catalogue ratio is unserved demand or absent demand (`commercial-model.md`).
- Discount applies to the plan, never to packs.

Annual-only halves the eventual take-up `[INFERENCE]` but a hundred prepaid founders is **$18,000 in the bank** before churn is possible, which is three and a half months of the $5,000/month break-even in `commercial-model.md`. A monthly founding member at $18 is $216 of promised revenue with a churn hazard at month two; the annual is $180 of collected revenue. Lifetime deals are ruled out on the evidence (81% of tracked LTD products shut down; this product's maintenance cost per user is permanent and rising because it chases two marketplaces' private APIs — market pack §6).

### 3.5 A service tier, because the founder's hour is the scarce input

Add one line the ladder does not have: **Move with me — $99**, a 45-minute screen-share in which the founder runs the seller's first import and first pack beside them. Sold as an add-on to any pack and included in the >500 quote. It is the highest-margin thing on the page (cost: one founder hour), it converts the sellers who will never install a desktop app unaided, and every session is a live discovery interview. Cap it at five a week so it never becomes the job.

---

## 4. What it earns

Net of Paddle (5% + 50c on the checkout price; UK VAT is charged on top and passed through, and whether the 5% is on the gross-including-VAT is still unresolved in `billing-vendor-memo.md` — model it as a further ~1 point of drag on UK buyers):

| SKU | gross | net to us |
|---|---|---|
| Pack 20 | $47 | $44.15 |
| Pack 50 | $77 | $72.65 |
| Pack 100 | $127 | $120.15 |
| Pack 250 | $247 | $234.15 |
| Pack 500 | $397 | $376.65 |
| Sync monthly | $29 | $27.05 |
| Sync yearly | $240 | $227.50 |
| Founding year one | $180 | $170.50 |
| Move with me | $99 | $93.55 |

Marginal cost per SKU: electricity < £0.01, storage < £0.01, AI < $0.20 even for a 500-pack at Haiku prices. The only cost that moves with volume is support, and the model above is shaped to keep it low: no trial clocks, no device disputes, no credit currency to explain, no plan a pack buyer has to understand.

Three scenarios, all on the measured pool (~4,000 dual-listers, top-decile ~400) and the documented behaviours (10–20 resource test first, packs for the catalogue, yearly Sync for the active minority):

| | Month 3 | Month 6 | Month 12 |
|---|---|---|---|
| Look accounts | 150 | 400 | 900 |
| Pack buyers (cumulative; 10% of Look, modal rung $127) | 15 → $1,800 | 40 → $4,800 | 90 → $10,800 |
| Founding annual (of 100) | 30 → $5,100 | 70 → $11,900 | 100 → $17,000 |
| Non-founding Sync yearly | 0 | 10 → $2,275 | 40 → $9,100 |
| Sync monthly, steady | 5 → $135/mo | 15 → $405/mo | 30 → $810/mo |
| Move with me | 5 → $470 | 15 → $1,400 | 30 → $2,800 |
| **Cash collected, cumulative** | **≈ $7,600** | **≈ $22,000** | **≈ $47,000** |
| **Run-rate at that month** (packs + monthly + annual/12 + service) | ≈ $1,500/mo | ≈ $3,000/mo | ≈ $4,200/mo |

The 10% Look→purchase rate is ChartMogul's opt-in median (8.9%) rounded, and the pack:Sync split follows the income distribution; both are assumptions to instrument from the first fifty accounts, not results. The founding count is the lever with the widest range: at 50 founders the month-12 cash is ~$38,000; at zero it is ~$30,000 and the business is a pack business. On run-rate, break-even against the $5,000/month opportunity cost is not reached inside twelve months on these numbers (month 15–18 on the same slopes); on cash, the founding prepayments cover months 1–9 of that opportunity cost by month 12. The gap between the two is exactly why the founding offer is annual-only.

---

## 5. What is deliberately not in the model

- **No lifetime deal, no perpetual founding discount.** Evidence in market pack §6; the three-year cap taken on 2026-09-12 stands.
- **No per-marketplace pricing.** There are two marketplaces; a third (Etsy/Shopify) is disabled in code. Charging per connection would tax the only thing the product is for.
- **No GMV take-rate.** OneShop and Flyp charge 13–55% of sales; a teacher's marketplaces already take 20–45%. A third hand in the till is the one model this persona will refuse.
- **No Tes sync promise.** Live edit and unpublish on Tes are uncaptured. The page must say "publish to Tes, keep TPT in sync" until they are; a refund is cheaper than a promise.
- **No AI SKU.** Bundled fair use when shipped.
- **No enterprise / agency tier.** The VA who runs ten sellers' stores is real (Fiverr $35/gig) and would pay, but each store needs its own device session; revisit when the first VA asks.

---

## 6. Risks the model does not fix

1. **Paddle's AUP prohibits products that infringe third-party terms** and the Stripe pre-approval gate is unanswered (`compliance-floor.md:89-97`). Every dollar above runs through a processor that could close the account. Onboard the dormant second processor before the first charge; this is the largest single risk to income and it is not a pricing decision.
2. **Demand is unvalidated.** No one has asked for this by name. The model is built so that the first purchase is small and the free tier is real; if Look accounts do not convert at ~10% within 60 days, the price is not the problem and the note to reread is `commercial-model.md` on the 2.3% ratio.
3. **RAM on the 8 GB nodes.** The first concurrency spike, not the first hundred sellers, is what breaks the cluster. Set Postgres requests/limits and stop it being BestEffort before any marketing push. Zero cost.
4. **Support is the founder.** Every SKU above is priced on the assumption of email support with no SLA. The `email_1_day` promise in the Studio row should be reworded to "priority" before Studio is ever sold.
5. **The tariff caveat:** the nodes report +12:00 and the Garage zones are `auckland-*`; the electricity estimate uses the Ofgem cap. Whichever tariff applies, the number is a rounding error against a single pack.

---

## 7. Changes if accepted, in order

1. `tam-limits`: `subscriber.monthly_cents` 2400 → 2900; `migrations_per_month` 20 → 25 with a 75 rolling cap (new field); `devices_max` 5 on every plan, dropped from the page; Free gets `import_marketplace: true`, `marketplaces_max: MAX`, `resources_max: 500`, `migrations_lifetime: 5` (new field); `migration_only` folds into a move balance on any plan with `edit_days_after_purchase: 90`; `FOUNDING` gains `annual_only: true`, `closes_at: 2026-12-31`, `data_share_months: 12`; a `services` table with "Move with me" at 9_900. Studio unchanged, still `sold: false`, trigger rewritten.
2. Landing: lead with coverage ("the only tool that lists to TPT and Tes"), the two calculators (Tes band distance, £3 flags), "$20 a month, billed yearly" as the Sync headline, packs relabelled by seller size as well as count, founding terms with the date and reversion, "Move with me" beside the >500 rung.
3. Paddle: one new monthly price, one annual founding price, one service price; pack prices unchanged.
4. Console: a move balance on the Account page; remove the device count; retire the `migration_only` plan rendering.
5. Ops before launch: Postgres requests/limits; second processor dormant; the Tes live-edit capture is the next engineering priority because it is the difference between "publish to Tes" and "sync with Tes" on the page.

---

## Sources

Filed beside this note, gathered 2026-09-20:

- `2026-09-20-pricing-evidence-product.md` — plan table, founding constants, feature inventory with gate fields, TPT/Tes capability matrix, cost constants, open decisions; every claim at `file:line`.
- `2026-09-20-pricing-evidence-infra.md` — node specs, cgroup-measured utilisation, storage, throughput of the trial runs, Ofgem-priced running cost, capacity ceiling.
- `2026-09-20-pricing-evidence-market.md` — crosslister prices and meters, marketplace seller terms, tool spend, Paddle/Stripe/Lemon Squeezy rates, AI per-fill costs, SaaS practice evidence, demand-signal search; every number with its URL and secondary/stale markers.

Prior notes carried forward: `2026-09-18-pricing-restructure-and-founding-100.md`, `2026-09-12-pricing-and-tiers.md`, `docs/design/commercial-model.md`, `docs/notes/design/billing-vendor-memo.md`, `docs/design/compliance-floor.md`.
