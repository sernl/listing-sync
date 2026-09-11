# Teachouse pricing and packaging: evidence and recommendation

- date: 2026-09-12
- author: ResearchPricing (subagent)
- scope: read-only research. Nothing in `/home/sernl/projects/edtech-workspace/listing-sync` was modified.
- repo inputs read: `docs/notes/design/2026-09-12-one-marketplace-per-site-and-the-seller-workflows.md`, `docs/design/decisions.md` (2026-09-11 entries), `docs/design/commercial-model.md`, `docs/notes/design/billing-vendor-memo.md`, `web/src/lib/pages/account/plans.ts`, `apps/landing/src/pricing.js`
- every external price below was fetched or searched on **2026-09-12**; where a page states its own verification date I give that too.

---

## 1. What Teachouse currently publishes

| surface | content | status |
|---|---|---|
| `apps/landing/src/pricing.js` | Catalogue Import one-off: **$47** ≤20, **$77** ≤50, **$127** ≤100, **$247** ≤250. Subscription **$24/mo, $240/yr**. Founding 100: 25% off year one, 20% ongoing, 20 free imports, 100 places. | live, founder-approved 2026-09-11 |
| `web/src/lib/pages/account/plans.ts` | Free / Solo $12·$120 / Studio $24·$240 / Publisher $48·$480, 14-day trial on Studio | **withdrawn from the site** by the 2026-09-11 decision; the console still renders it |
| design of record §4 | `org.plan ∈ {free, subscriber, migration_only}`; Paddle gains a second price for the one-off; `migrations_per_month` comes from a plan table | designed, not built |

The console and the landing site currently disagree. Any recommendation has to resolve that, and the resolution the founder already chose is the three-value `org.plan` set, not four tiers.

---

## 2. Competitor table: crosslisting tools for resellers

All USD unless marked. "Metering axis" is what the vendor actually charges more for.

| Product | Tier | Monthly | Annual (per mo) | Metering axis | What the tier gates | Trial / free |
|---|---|---|---|---|---|---|
| **Vendoo** [1] | Free | $0 | — | items | 5 items, 3 background removals | permanent |
| | Starter | $14.99 | $12.49 | — (unlimited items) | all marketplaces, sale detection + auto-delisting, templates, importing, delist/relist, analytics, mobile app | 14-day trial, **card required** [2] |
| | Growth | $29.99 | $24.99 | — | + AI Listing Enhancement, **bulk actions (240 at once)**, 300 PhotoRoom removals | |
| | Pro | $59.99 | $49.99 | — | + auto-send offers (6 marketplaces), marketplace sharing, 1,500 removals, listing videos, live call support | |
| | Enterprise | quote | — | >1,000 new listings/mo | — | |
| **Crosslist** [3] | Bronze | $29.99 | 2 months free on yearly; −10% quarterly | **new listings added per month** (200) | 9 images/listing, core features | 3-day money-back **or 20 listings, whichever first** |
| | Silver | $34.99 | | 500/mo | same core | |
| | Gold | $39.99 | | 1,000/mo | + 15 images, **CSV import/export**, **autodelisting**, **sales analytics** | |
| | Diamond | $44.99 | | unlimited | + 24 images | |
| | AI add-on | **+$4.99** | | credits (200/500/1,000/2,000 by tier) | AI listing creation (1 credit), AI photo edit (2), bulk create (1 per 10 images), AI photos (2), AI pricing | |
| **List Perfectly** [4][5] | Simple | $29 | — | AI credits 25/mo, barcode 25/mo, removals 25/mo | crosslist carries **images, title, description, price only**; no crop/brightness/filters | "100 Free Listings Money Back Guarantee" [4] |
| | Business | $49 | — | 50/50/50 | + brand, colour, size, quantity, SKU; image editing | |
| | Pro | $69 | — | 200/100/1,500 | + keywords/tags, MSRP, UPC, condition, shipping weight & dimensions; Shopify/Instagram/Listing Party; custom titles per marketplace; sub-accounts | |
| | Pro Plus | from $79; tiers $99 / $149 / $249+ | — | 1,000+/1,000+/3,000+ | everything, sub-accounts | |
| **Nifty** [6] | Automation | $39.99 ($25 single-platform) | $35.99 ($22) | marketplace count | Poshmark/eBay/Mercari/Depop share, follow, relist, offers; server-side; priority email | 7-day free trial |
| | Crosslisting Plus | $39.99 | $35.99 | **closet size 1,500 items + 500 "smart credits"** | inventory manager, AI listing generation, bulk actions, one-click import, sale detection + auto-delist, drafts & scheduled posts, analytics + tax reports | |
| | Crosslisting Pro | $59.99 | $53.99 | 1,500+ items, 1,000 credits | same features, higher caps | |
| | Bundle Plus / Pro | $69.99 / $89.99 | $62.99 / $80.99 | both axes | both products | |
| **OneShop** [7] | Premium (only tier) | **$45** | — | none published | single subscription, everything | free trial referenced, shape not published |
| **SellerAider** [8] | Grow Standard | $18 | yearly "20%+ off" | **one supported site** | send offers, relist, refresh, share, bulk edits, automatic messages, 24/7 support | 14-day free trial |
| | Grow Pro | $25 | | more sites | — | |
| **Flyp** [9] | single | **$9** flat | — | none — unlimited | 6 marketplaces, auto-delist on sale, Poshmark bot with scheduling and offers-to-likers | **100 days free, no card** |
| **Zipsale** (GBP) [10] | Free | £0 | — | 30 items/mo | **no auto-delisting** | permanent |
| | Basic | £25 | £21.25 | 200 items/mo | + auto-delisting | annual −15% |
| | Start | £45 | £38.25 | 500/mo | | |
| | Growth | £79 | — | 1,500/mo | | |
| | Pro | £99 | — | 3,000/mo | | |
| | Business | £149 | — | >3,000/mo | | Depop and Vinted actions are paid add-ons |

### What the table actually says

1. **Two metering axes, and one is losing.** Crosslist and Zipsale meter *new listings added per month*. Vendoo explicitly abandoned that model — "instead of paying based on how many new items you list each month… all with unlimited items and unlimited crossposting" [11] — and Flyp, OneShop and List Perfectly never used it. The surviving axis is **feature and automation depth**, with volume as a secondary cap (Nifty's 1,500-item closet).
2. **The full-crosslister band is $25–$45/month.** Vendoo Growth $29.99, Crosslist Gold $39.99, List Perfectly Simple $29, Nifty Crosslisting $39.99, OneShop $45, Zipsale Start £45. Below that band you are buying a single marketplace (SellerAider $18) or a thin unlimited tool (Flyp $9).
3. **Analytics is a paid gate, twice.** Crosslist puts sales analytics on Gold ($39.99) and above; List Perfectly puts the analytics dashboard above Simple. This corroborates `commercial-model.md`'s claim that analytics is the documented upgrade driver.
4. **Import is never charged separately.** Every one of the eight bundles import — spreadsheet/CSV, one-click marketplace import, or both — inside the subscription. Crosslist gates *CSV* import to Gold; nobody sells import as a one-off SKU.
5. **Bulk verbs are a real gate.** Vendoo withholds bulk actions entirely from its $14.99 tier. This is the cleanest precedent for gating Teachouse's collection verbs.
6. **AI is cheap or free, never a premium.** Crosslist sells all six AI features for **+$4.99/month**; Vendoo bundles AI at its recommended $29.99 tier; Nifty folds it into "smart credits" already included in the base price.
7. **Trials require a card and are short.** Vendoo 14 days with card [2]; Nifty 7 days; Crosslist 3 days *or* 20 listings; SellerAider 14 days. Flyp's 100 days without a card is the outlier and it is a $9 product. Free tiers are tiny: Vendoo 5 items, Zipsale 30 items/month with the key feature removed.

---

## 3. Teacher-creator economics

### What the marketplaces take

| Platform | Seller cost | Payout | Transaction fee | Source |
|---|---|---|---|---|
| **TPT Basic** | one-time, non-refundable **$29** | **55%** of all sales | **$0.30** per resource | [12] |
| **TPT Premium** | **$59.95/year** | **80%** | **$0.15** per resource, only on orders under $3 | [12] |
| **TPT Publisher** (not self-authored) | one-time $29 | 50/50 split | — | [12] |
| **Tes** | free to list | **Bronze 60%** (£0–999.99 rolling 12-month sales), **Silver 70%** (£1,000–5,999.99), **Gold 80%** (£6,000+), on the VAT/GST-exclusive price | **20p/20c** on resources under £3/$3 | [13] |
| **Tes** price floor/ceiling | — | £1 min, £300 max; school licence priced at **3.5×** the single licence, same royalty % | — | [13] |
| **Teach Simple** | free | **50% of all revenues** to teachers; subscription-download model | — | [14] |
| **Classful** (digital resources) | **no platform fee** | seller keeps the rest | **5%** transaction + **2.9% + $0.30** processing | [15] |
| **Classful** (courses) | no monthly fee | 80% | 20% fee + 2.9% + $0.30 | [16] |

Two consequences for Teachouse. First, **Tes royalty levels are volume-banded on a rolling 12-month total** — a Tes author sitting at £900 has a direct, quantifiable £100 reason to cross-list more onto Tes (60%→70% is a 16.7% raise on every future sale). That is the sharpest ROI story available to this product and it is not currently used anywhere in the pricing copy. Second, TPT's two-tier structure means a seller's *existing* mental price for "software that helps me sell" is **$59.95/year**, i.e. **$5/month**.

### What teacher-sellers actually earn

| Figure | Value | Confidence | Source |
|---|---|---|---|
| Active TPT sellers, 2024 | ~233,358 | third-party analysis of TPT's yearbook + survey, not TPT-published | [17] |
| Total TPT payout, 2024 | ~$253M | same | [17] |
| Median-ish active seller | **~$27/month** | same | [17] |
| Top 1% | ~$6,300/month | same | [17] |
| Six-figure earners, 2024 | 431 people = **0.2%** | same | [17] |
| Measured dual-listers (repo's own probe) | 10 confirmed active dual-listers hold **7,644 TPT listings** and 173 Tes listings — **~764 TPT listings each** | repo measurement, search-rank biased upward | `commercial-model.md` |

I could not find a TPT- or Tes-published catalogue-size distribution; the 764-listing average above is the best number available and it is the founder's own, drawn from search-ranked (therefore successful) sellers.

### Willingness to pay, same persona

| Product | Price | Note | Source |
|---|---|---|---|
| TPT Premium Seller | **$59.95/yr** (~$5.00/mo) | the benchmark paid subscription for this exact buyer | [12] |
| Boom Learning Premium | **$6.99/month** | flexible monthly membership | [18] |
| Boom Learning Publisher | **$69.99/year** (~$5.83/mo) | the tier that lets a teacher *sell* decks | [19] |
| Boom Learning (2024 reporting) | $25/yr for 150 students; **$50/yr to publish and sell** | | [20] |
| Canva Pro | **$18/month**, **$180/year**; Business $25/user/mo, $250/yr | Canva for Education remains free to verified teachers, which suppresses paid conversion in this segment | [21][22] |
| Mr Joel's TPT Seller SEO & Listing Optimizer | **$79.99 one-time**, 180 ratings at 4.97 | sells a bulk product importer to TPT sellers today | `commercial-model.md` |
| Bearwood Labs Product Description Editor Pro | **$29.00** on TPT itself, ~5 years | | `commercial-model.md` |

**The central tension, stated plainly.** The *competitive* band is $25–$45/month. The *persona's* habitual band is $5–$18/month. Teachouse's $24 sits at the bottom of the first and the top of the second — which is the only place it can sit. It is also **~89% of the median active TPT seller's entire monthly payout**, so the subscription is not a mass-market product at any price; it is a top-decile product. The one-off ladder is what the other nine deciles buy.

---

## 4. AI add-on precedent

| Vendor | Model | Price | Source |
|---|---|---|---|
| **Crosslist** | paid add-on, credit-metered | **+$4.99/month** for 200–2,000 credits by tier; 1 credit per AI listing, 2 per photo edit | [3] |
| **Vendoo** | tier gate, no credits | AI Listing Enhancement starts at Growth **$29.99** (absent from $14.99 Starter) | [1] |
| **Nifty** | bundled as "smart credits" | 500/mo on $39.99, 1,000/mo on $59.99 | [6] |
| **Shopify Magic** | **free, included in every plan**; image generation metered with a monthly free allotment (~50/mo on Basic) | $0 | [23][24] |
| **Notion AI** | folded into the plan price; **no longer a separate add-on for new customers**; Business $20/member/mo annual. Custom Agents metered at $10 per 1,000 credits | — | [25][26] |
| **Canva** | Pro/Business include AI; Sora-powered premium video bills separately per generation | $18/mo | [21][27] |

**Read:** the industry has converged on *bundled AI with a fair-use cap*, with credits reserved for genuinely expensive generation (images, video, agents). Nobody charges a meaningful premium for text auto-fill at this price point. `commercial-model.md` already reaches the same conclusion from a different direction — AI listing rewriting is sold to this persona at $79.99 one-time, so it cannot be the differentiator. And the repo's own cost model puts AI tokens at **$0.49/user/month**, which is 2% of a $24 subscription: metering it would cost more in console surface and support than it saves.

---

## 5. One-off migration precedent — does the $47–$247 ladder hold?

| Precedent | Shape | Price | Source |
|---|---|---|---|
| **Cart2Cart** | one-time, priced by volume of migrated entities and platform pair; free demo migration, pay after demo and before full transfer, no card to start | **from $29**, commonly **$69–$999** | [28][29] |
| **Cart2Cart / LitExtension** consultant help | separate service package | **from $299** (5 hours) | [30] |
| **ExportYourStore** | one-time setup + subscription | **$199 one-time** setup for hardest channels; $29–$249/mo by listing count | `commercial-model.md` |
| **Mr Joel's TPT Seller SEO app** | one-time purchase, includes a bulk product importer, sold to TPT sellers | **$79.99**, 180 ratings at 4.97 | `commercial-model.md` |
| Every crosslisting comparable in §2 | import bundled free inside the subscription | $0 | [1][3][4][6] |

**Verdict on the ladder: keep every number.** The per-resource rates are $2.35 / $1.54 / $1.27 / $0.99, which is a clean declining curve; the $77 rung sits almost exactly on the one proven one-time cheque this persona writes ($79.99, 180 times over, at 4.97 stars); and the whole shape matches Cart2Cart's volume-priced one-time migration, which is the correct analogue because the job genuinely is bursty. `commercial-model.md`'s own modelling agrees: $299 one-time + $19/month yields ~$518 lifetime value against $237 for subscription-only at 6% churn.

**One change I do recommend, with evidence.** The ladder tops out at 250 resources. The repo's own measurement says a confirmed active dual-lister holds **~764 TPT listings**. The top rung therefore does not reach the average serious seller — the exact person with the most to migrate and the most willingness to pay. Add two rungs:

- **Up to 500 — $397** (per-resource $0.79, continues the curve)
- **Over 500 — talk to us** (Cart2Cart runs to $999 [29]; Vendoo routes >1,000 new listings/month to Contact Sales [1]; a human conversation at this volume is both normal and where the founder learns most)

Also required, and currently unstated: **the ladder must say what it counts.** Under the design of record §2 an import run reads N marketplace listings and, after the duplicate review merges pairs, commits M ≤ N resources. Price on **M, resources committed to the catalogue**, counted at commit, and say so — otherwise the first seller whose 60 listings merge into 45 resources has a support ticket and a refund argument.

---

## 6. Recommended Teachouse ladder

Names and prices. Everything the founder set on 2026-09-11 is carried unchanged except where marked.

| | **Free** | **Teachouse Subscription** | **Studio** *(deferred — see trigger)* | **Catalogue Import** *(`migration_only`)* |
|---|---|---|---|---|
| monthly | — | **$24** | $44 | — |
| annual | — | **$240** (2 months free) | $440 | — |
| one-off | — | — | — | **$47** ≤20 · **$77** ≤50 · **$127** ≤100 · **$247** ≤250 · **$397** ≤500 · quote >500 |
| trial | permanent | **14 days, card required** | — | free preview of what would be imported, no card |

### Gating matrix

| feature | Free | Subscription $24 | Studio $44 (deferred) | `migration_only` |
|---|---|---|---|---|
| **catalogue (resource cap)** | 20 | 400 | unlimited | the rung purchased (20/50/100/250/500) |
| **marketplaces connected** | 1 | all | all | all (needed for a two-sided move) |
| **import — spreadsheet** | yes, within the 20 cap | **included, unlimited** | included | included within the rung |
| **import — marketplace** | no ("Connect a plan to read your shop") | **included, unlimited** | included | included within the rung |
| **import — duplicate review** | no | yes | yes | yes |
| **cross-list / publish** | manual, 1 marketplace | unlimited, all marketplaces | unlimited | one publish pass over the imported set |
| **edit / delete on marketplaces** | yes | yes | yes | yes for 30 days after purchase, then read-only |
| **migrations per month (copy/move)** | 0 | **20 resources/month** | 100 resources/month | the purchased volume, once |
| **scheduling** | no | yes | yes | no |
| **sync pulls** | no | every 6 hours | hourly | no |
| **auto-publish rules on pull** | no | yes | yes | no |
| **templates** | 1 | 20 | unlimited | 1 |
| **collections** | 0 | 20 | unlimited | 0 |
| **labels** | 5 | 20 (marketplace auto-labels excluded from the count, per design §2) | 50 | auto-labels only |
| **analytics** | no | **yes** | yes + per-marketplace breakdown | no |
| **export (spreadsheet)** | **yes** | yes | yes | **yes** |
| **devices** | 1 | 2 | 3 | 1 |
| **AI auto-fill** *(coming soon)* | no | **bundled, 200 fills/month fair use** | bundled, 600/month | no |
| **priority support** | guides only | email, 2 business days | email, 1 business day | email for 30 days after purchase |

### Founding 100 overlay (unchanged numbers)

- **25% off year one, 20% off thereafter, first 20 resources imported free, 100 places.** All four numbers kept.
- Effective: $18.00/mo or $180/yr in year one; $19.20/mo or $192/yr ongoing. Both stay well clear of Paddle's under-$10 custom-pricing rule (`billing-vendor-memo.md` §2), and Paddle's fixed 50c is 2.8% of the discounted monthly — acceptable.
- **One clause needs re-reading, not re-pricing.** "The first 20 resources imported free" only means something if import is charged. Under my §7 recommendation it is included for subscribers, so for a Founding subscriber the clause is already true and reads as reassurance. For the `migration_only` buyer it is the live benefit: it turns the $47 rung into $0 and the $77 rung into $77 for 30 additional resources. Keep the words; point them at the one-off ladder.

### Rationale, one or two sentences each

- **Free at 20 resources / 1 marketplace.** Keeps the founder's existing free allowance, and it sits between Vendoo's 5 items [1] and Zipsale's 30/month [10]; one marketplace makes the free tier a catalogue, never a crosslister, which is the thing being sold.
- **One paid recurring tier at $24/$240.** The founder collapsed four tiers into one on 2026-09-11 and no evidence contradicts that at this scale: $24 is exactly Vendoo Growth's annual rate ($24.99) [1] and sits under every full crosslister's monthly, which is correct for a persona whose habitual software spend is $5–$18/month [12][18][21].
- **Studio deferred, with a written trigger.** Every comparable has 3–4 tiers and uses the top one for volume and automation, but a second tier only earns its console surface once someone is straining the first; **ship it when ≥20% of subscribers exceed 300 resources or hit the migration cap twice in a quarter**, and not before.
- **Catalogue cap 400 on the subscription.** Carried from the withdrawn Studio row; it is above the measured dual-lister's 173 Tes listings and below their 764 TPT listings, so it is the number that triggers the Studio conversation rather than silently blocking the target customer.
- **Import included for subscribers.** All eight comparables bundle import [1][3][4][6]; charging at the moment of activation taxes exactly the step that makes the product useful, and it is the single highest-leverage conversion risk in the current plan (see §8, risk 1).
- **Migrations metered at 20 resources/month, not per batch.** The measured steady-state behaviour is 5–20 listings [`commercial-model.md`], so 20/month covers real use while keeping the subscription from cannibalising the ladder — one month of $24 must never be cheaper than the $47 rung it replaces.
- **Duplicate review on every paid path.** It is the feature that makes the "one resource per real product" rule true; putting it behind a higher tier would let a paying seller create the exact double-listing the product exists to prevent.
- **Analytics on the subscription, not above it.** Crosslist gates analytics at $39.99 and List Perfectly above Simple [3][4], and `commercial-model.md` names analytics as the documented renewal mechanism — with only one paid tier, it must be inside it or it renews nothing.
- **Export never gated, at any tier, including free and `migration_only`.** A seller who cannot get their catalogue out will not put it in; the cost is one CSV and the trust return is the whole sales argument.
- **AI auto-fill bundled with a fair-use cap, no credit currency.** Shopify Magic is free in every plan [23], Notion folded AI into the plan price [26], Crosslist's entire AI suite is $4.99 [3], and the repo's cost model puts tokens at $0.49/user/month — a credit ledger would cost more to build and support than it could ever recover.
- **`migration_only` unlocks Import, Migrations, Export and Account.** The design of record §4 currently names only "Migrations and Account", which would leave the buyer of a product called *Catalogue Import* unable to reach the import page — flagged as a defect below.
- **Two rungs added above 250.** The founder's own measurement puts a serious dual-lister at ~764 TPT listings, so the ladder as published stops short of its best customer; Cart2Cart runs to $999 [29] and Vendoo routes high volume to sales [1].
- **14-day carded trial on the subscription.** Vendoo's exact shape [2], and `commercial-model.md` cites ChartMogul's finding of 8% median free-to-paid against 30% when a card is required.

### Defect found in the design of record

`docs/notes/design/2026-09-12-…md` §4 states: *"A `migration_only` org can sign in, buy again, subscribe, and use Migrations and Account; every other section renders its reason."* The product sold on the landing page at $47–$247 is **Catalogue Import**, which lives in §2's Import page, not §4's Migrations page. As written, a one-off buyer is gated out of the thing they bought. The capability set for `migration_only` must be `{Import, Migrations, Export, Account}`.

---

## 7. The three pricing risks to put in front of the founder

**1. Charging subscribers for import is an activation tax, and it compounds the churn spike already predicted.**
`commercial-model.md` states the product "is a migration tool wearing a subscription's clothes, because the initial duplication of a back catalogue is roughly twenty times the steady-state monthly need, so expect a churn spike two to three months after signup." The current page asks a new subscriber to pay $24 *and then* $127 before the product does anything, at the precise moment their confidence is lowest — and every competitor gives that step away [1][3][4][6]. The mirror risk is real and must be modelled, not waved away: with import included, a rational back-catalogue seller could subscribe for one month, migrate, and cancel. The 20-resources/month migration cap in §6 is the instrument that prevents it — it makes the one-off ladder strictly cheaper than serial re-subscription for any catalogue over 20, which is every real customer.

**2. The subscription's addressable population is roughly a tenth of the headline pool, and $24 is what makes that true.**
At ~$27/month for the typical active TPT seller [17], a $24 subscription is 89% of their entire platform income; only the upper decile can rationally buy it. Against the measured 4,000-seller dual-lister pool, the *subscription* market is therefore nearer 400 people, not 4,000 — while `commercial-model.md`'s break-even sits at 250–300 customers. The price is right; the **quantity** assumption is the exposure, and the mitigation is that `migration_only` must be a first-class, self-serve, genuinely-good product rather than a downgrade path, because it is what the other 3,600 sellers will buy.

**3. "20% off thereafter" is a permanent discount with no exit, and it anchors every future price.**
One hundred places at 20% off forever is roughly **$5,760/year of permanently foregone revenue** at list, and — more costly — it establishes $19.20 as the real price in the mouths of your first and loudest hundred customers, exactly the cohort that will publicise any later increase. Two options, both cheap to take now and impossible to take later: cap the ongoing discount at **three years**, or keep it perpetual but bind it to the tier purchased, so a later Studio upgrade is at list. If neither, take it deliberately and write the number down.

*Two smaller items worth a line each.* The import ladder must state that it counts **resources committed to the catalogue after duplicate merges**, or the first seller whose 60 listings merge into 45 will ask for money back. And the strongest untapped ROI argument in this whole file is Tes's rolling royalty bands [13] — an author at £900 of 12-month sales who crosses £1,000 moves from 60% to 70% on every subsequent sale, a 16.7% raise; a Teachouse page that computes the distance to the next band would sell the subscription better than any feature list.

---

## Sources

Every URL below was read or returned by search on **2026-09-12**.

1. Vendoo pricing — https://www.vendoo.co/pricing (read 2026-09-12): Starter $14.99/$12.49, Growth $29.99/$24.99, Pro $59.99/$49.99, Enterprise quote, Free plan 5 items + 3 background removals.
2. Vendoo trial shape — https://flowlister.com/vs-vendoo/ (read 2026-09-12; page states ladder verified from vendoo.co/pricing on 2026-08-25): "14-day trial requires a card but does not charge it if you cancel in time."
3. Crosslist pricing — https://crosslist.com/pricing (read 2026-09-12; page published 2026-09-10): Bronze $29.99/200, Silver $34.99/500, Gold $39.99/1,000, Diamond $44.99/unlimited; AI add-on +$4.99/mo with per-tier credits; 3-day or 20-listing money-back; quarterly −10%, yearly 2 months free.
4. List Perfectly pricing page — https://listperfectly.com/pricing/ (read 2026-09-12): tier names, per-tier AI/barcode/background-removal allowances, crosslist attribute depth per tier, sub-accounts, "100 Free Listings Money Back Guarantee".
5. List Perfectly prices — https://selleraider.com/list-perfectly-pricing/ and https://nifty.ai/post/list-perfectly-pricing (search result, 2026-09-12): Simple $29, Business $49, Pro $69, Pro Plus from $79 with tiers at $99/$149/$249+.
6. Nifty pricing — https://nifty.ai/pricing (read 2026-09-12): Automation $39.99/$35.99 ($25/$22 single platform), Crosslisting Plus $39.99/$35.99, Crosslisting Pro $59.99/$53.99, Bundle Plus $69.99/$62.99, Bundle Pro $89.99/$80.99; 1,500-item closet; 500/1,000 smart credits; 7-day free trial.
7. OneShop pricing — https://oneshop.com/pricing → https://tools.oneshop.com/pricing (read 2026-09-12): "One single subscription… $45/month".
8. SellerAider pricing — https://selleraider.com/pricing/ (read 2026-09-12): Grow Standard $18/mo one supported site, Grow Pro $25, 14-day free trial, yearly "20%+ off".
9. Flyp pricing — https://blog.vendoo.co/flyp-vs-list-perfectly and https://flip-press.com/blog/best-crosslisting-apps-2026 (search, 2026-09-12): $9/mo flat unlimited after 100 free days, no card, six marketplaces.
10. Zipsale pricing — https://www.zipsale.co.uk/blog/using-zipsale-for-your-reselling and https://selleraider.com/vendoo-vs-zipsale/ (search, 2026-09-12; zipsale.co.uk/pricing failed TLS verification on direct fetch): Free 30/mo no auto-delist; Basic £25/£21.25 200; Start £45/£38.25 500; Growth £79 1,500; Pro £99 3,000; Business £149 >3,000; annual −15%.
11. Vendoo on abandoning per-listing pricing — https://blog.vendoo.co/vendoo-pricing-explained-and-why-its-worth-the-investment (search, 2026-09-12).
12. TPT seller fees and payout rates — https://help.teacherspayteachers.com/hc/en-us/articles/360044219891-Seller-Fees-and-Payout-Rates (read 2026-09-12; page updated 2026-07-22): Basic one-time $29 / 55% / $0.30 per resource; Premium $59.95 per year / 80% / $0.15 only under $3; Publisher one-time $29 / 50-50.
13. Tes selling FAQ — https://www.tes.com/policies/help/selling-resources-faq (read 2026-09-12; page last updated 2024-03-07): royalty levels Gold £6,000+ → 80%, Silver £1,000–5,999.99 → 70%, Bronze £0–999.99 → 60%, on VAT/GST-exclusive price, rolling 12-month total; 20p/20c fee under £3/$3; price £1–£300; school licence 3.5× at the same royalty rate; £10/$10 withdrawal threshold.
14. Teach Simple — https://teachsimple.com/ (search, 2026-09-12): "Teachers create all the materials provided on Teach Simple, and 50% of all revenues go to them"; unlimited-download subscription model.
15. Classful, digital resources — https://classful.com/sell-products/ (search, 2026-09-12): no platform fee, 5% transaction fee, 2.9% + $0.30 processing.
16. Classful, courses — https://classful.com/teach-on-classful/ (search, 2026-09-12): 20% fee plus Stripe 2.9% + $0.30, no monthly fee.
17. TPT seller income distribution — https://seolumina.com/blog/how-much-do-tpt-sellers-make-in-2026-real-data-income-breakdown and https://seotpreneur.com/can-tpt-really-replace-your-teaching-income-in-2025/ (search, 2026-09-12): ~$27/month typical active seller, top 1% ~$6,300/month, 431 of 233,358 sellers six-figure in 2024 (0.2%), ~$253M paid out 2024. **Third-party analysis of TPT's yearbook and survey, not a TPT publication — medium confidence.**
18. Boom Learning monthly membership — https://www.boomlearning.com/boom-learning-news-blog/monthly-membership (search, 2026-09-12): Premium $6.99/month.
19. Boom Learning Publisher level — https://myhappyplaceteaching.com/2020/07/what-are-boom-cards-getting-started.html (search, 2026-09-12): $69.99/year Publisher, the level at which a teacher may sell decks. **Dated source; treat as an order-of-magnitude anchor.**
20. Boom Learning pricing, journalism — https://www.edweek.org/technology/opinion-more-than-12-million-students-have-used-this-new-study-aid-whats-the-deal/2024/01 (search, 2026-09-12): $25/year for 150 students; **$50/year to publish and sell Boom Cards**.
21. Canva pricing — https://socialrails.com/blog/canva-pricing (search, 2026-09-12): Pro $18/month, Business $25/user/month, Free remains free.
22. Canva annual — https://bloggingtitan.com/pricing/canva/ (search, 2026-09-12): Pro US$180/year, Business US$250/person/year.
23. Shopify Magic included free — https://www.polaranalytics.com/post/shopify-ai-features-tools-agents (search, 2026-09-12): "Shopify Magic is free and included in every Shopify plan. You do not pay an add-on fee."
24. Shopify Magic image metering — https://www.letstalkshop.com/blog/shopify-magic-free-ai-assistant-for-small-store (search, 2026-09-12): image features metered with a monthly free allotment, ~50/month on Basic as of early 2026.
25. Notion plan prices — https://lifestack.ai/blog/notion-pricing (search, 2026-09-12): Free $0, Plus $10/member/mo annual, Business $20/member/mo annual.
26. Notion AI no longer a separate add-on — https://www.layer3labs.io/guides/notion-ai-pricing (search, 2026-09-12); credit metering for Custom Agents at $10 per 1,000 credits — https://get-alfred.ai/blog/notion-pricing (search, 2026-09-12).
27. Canva premium video billed per generation — https://www.miracamp.com/learn/canva/pricing-plans (search, 2026-09-12).
28. Cart2Cart pricing model — https://cart2cart.net/pricing/ (search, 2026-09-12): priced by volume of migrated entities and by platform pair; one-time payment after a free demo migration; no card to start the demo.
29. Cart2Cart price range — https://learnwoo.com/cart2cart-vs-litextension-ecommerce-site-migration-tools-comparison/ and https://medium.com/@ontario-ou/cart2cart-vs-litextension-which-e-commerce-migration-service-should-you-choose-c69aca2e0488 (search, 2026-09-12): from $29, commonly $69–$999 by store size and complexity. **litextension.com/pricing.html served an RSS feed rather than the pricing page on 2026-09-12; LitExtension's own per-record table could not be read directly.**
30. Migration consultancy floor — https://litextension.medium.com/the-comprehensive-all-in-one-migration-service-comparison-between-litextension-and-cart2cart-6b782ddf0767 (search, 2026-09-12): at least $299 for a Basic Service support package with 5 hours of online support.

Repo-internal, read 2026-09-12: `docs/design/commercial-model.md` (ExportYourStore $29–$249/mo plus $199 one-time; Mr Joel's $79.99 one-time with 180 ratings at 4.97; Bearwood Labs $29.00; ChartMogul 8% vs 30% trial conversion; 10 dual-listers holding 7,644 TPT and 173 Tes listings; cost per user $12.96/$7.77/$5.38 at 100/300/1,000 users; AI tokens $0.49/user/month); `docs/notes/design/billing-vendor-memo.md` (Paddle 5% + 50c merchant of record, sub-$10 custom-pricing rule, entry tier recommended at USD 12 or above); `docs/design/decisions.md` 2026-09-11; `apps/landing/src/pricing.js`; `web/src/lib/pages/account/plans.ts`.
