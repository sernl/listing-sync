# Subscription tiers, 2026-09-27: three paid plans and a free one, with no launch offer

- date: 2026-09-27
- status: proposal, implemented on `feat/tiers` (`crates/tam-limits/src/lib.rs`, the landing pricing section and the console Billing page). Open to the founder's and sellers' feedback; every figure lives in one table, so a change is one edit.
- supersedes: the Founding 100 offer (`2026-09-18-pricing-restructure-and-founding-100.md`, `2026-09-20-pricing-model-re-evaluation.md` §3.4) and the "Move with me" service (§3.6 of the same note). Everything else in `2026-09-26-pricing-re-evaluation.md` still stands: Look stays free forever, packs keep their prices, the capacity argument is unchanged.
- evidence: `2026-09-26-pricing-evidence-competitors.md` (23 vendors, fetched first-hand on 2026-09-26) plus the fetches below (2026-09-27).
- the founder's ask: drop "Move with me" ($99) and the Founding 100; present subscription tiers as an established product, three paid tiers and a free one, monthly and yearly prices in USD; keep today's price points close unless the evidence says otherwise.

---

## 0. Answer

| | **Look** | **Starter** | **Sync** | **Studio** |
|---|---|---|---|---|
| Monthly | Free | **$12** | **$29** | **$59** |
| Yearly (per month) | Free | **$96** ($8) | **$240** ($20) | **$480** ($40) |
| Yearly saving | – | 33% | 31% | 32% |
| Moves | 5 once | 10 a month | 25 a month | 100 a month |
| Unused moves stack to | – | 30 | 75 | 300 |
| Resources | 500 | no cap | no cap | no cap |
| Storage | 256 MB | 5 GB | 20 GB | 200 GB |
| Templates | 1 | 5 | 20 | no cap |
| Collections | – | 5 | 20 | no cap |
| Labels | 5 | 20 | 20 | 50 |
| Statistics (analytics) | – | – | yes | yes |
| Scheduling | – | yes | yes | yes |
| Edits kept in step on every marketplace | – | daily | every 6 hours | every hour |
| Automatic publishing rules | – | – | yes | yes |
| Watermarked previews | yes | yes | yes | yes |
| Import and export | yes | yes | yes | yes |
| Marketplaces | TPT and Tes | TPT and Tes | TPT and Tes | TPT and Tes |
| Devices | 5 | 5 | 5 | 5 |
| AI description fill (coming soon) | – | 50 a month | 200 a month | 600 a month |
| Support | guides | email, 2 days | email, 2 days | priority email, 1 day |

- **Sync is unchanged** ($29 / $240, 25 moves). It sits on the category midpoint (§2) and nothing in the new evidence argues for moving it.
- **Starter is new**, at $12 / $96. It is the step between free and Sync that the category has and we did not: Etsy Plus $10, Listelf $9.99, Flyp $9, Vendoo Starter $14.99, Zipsale Part Time £15.
- **Studio is now sold**, at $59 / $480 rather than the deferred $44 / $440. The deferred figures gave a 17% yearly saving beside Sync's 31%, and sold four times Sync's moves for 1.5 times the price. At $59 it matches the "pro" rung of every cross-lister that publishes one (Vendoo Pro $59.99, Nifty Pro $59.99, Sellfy Business $59 a month yearly).
- **Move Packs stay**, all five rungs at today's prices (§4).
- **Founding 100 and "Move with me" are withdrawn.** Nobody is harmed by withdrawing them (§5).

---

## 1. What each plan is for

One sentence each, which is also what the cards say.

- **Look**: bring your shops in, see every resource in one place, and try five moves. The trial, with no clock.
- **Starter**: one teacher, one or two shops, a few new resources a month, edits kept in step once a day.
- **Sync**: a teacher who adds resources every week and wants statistics and automatic publishing.
- **Studio**: a big catalogue, a team or a shop moving hundreds of resources: 100 moves a month, hourly updates, no caps on templates or collections, priority support.

The ladder climbs on **moves**, as the 2026-09-20 note decided. The other axes follow the moves: a seller who moves more needs more templates and collections and wants edits sent sooner. Import, export, watermarked previews, both marketplaces and five devices are the same on every plan. The reasons:

- **Import and export are never gated.** A seller who cannot get their catalogue out will not put one in (decided 2026-09-12, pinned by a test).
- **Watermarked previews** are drawn in the browser (`web/src/lib/pages/resources/PreviewMaker.svelte`) and cost the server nothing. They are also the feature a trial seller most wants to see.
- **Marketplaces.** There are only two, and a cross-lister that sells one is not a cross-lister.
- **Devices.** The marketplace work runs on the seller's own device (`2026-09-26-pricing-re-evaluation.md` §1.1). Five devices cost us nothing, and a cap would only create support tickets.

## 2. Comparable tools, 2026 prices

Prices are USD a month unless stated. "Yearly" is the per-month price when paid yearly.

| Tool | Tiers (monthly · yearly) | What the tiers meter | Free tier | Source |
|---|---|---|---|---|
| **TPT seller membership** | Basic $29 once · Premium **$59.95 a year** | payout share (55% vs 80%) and per-resource fee | buyers only | [help.teacherspayteachers.com/…/360040842852](https://help.teacherspayteachers.com/hc/en-us/articles/360040842852-What-membership-types-are-available); [/360044219891](https://help.teacherspayteachers.com/hc/en-us/articles/360044219891-Seller-Fees-and-Payout-Rates) (first-hand 2026-09-26) |
| **Etsy Plus** | **$10** a month, rolling 30 days | includes 15 listing credits ($3) and $5 of Etsy Ads credit | Etsy standard (pay $0.20 a listing) | [craftybase.com/blog/should-you-subscribe-to-etsy-plus](https://craftybase.com/blog/should-you-subscribe-to-etsy-plus) `[SECONDARY, 2026-03-23]`; listing fee first-hand at [help.etsy.com/…/360000344908](https://help.etsy.com/hc/en-us/articles/360000344908-Fees-and-Listing-Multiple-Quantities) |
| **Sellfy** | Starter $29 · **$22** · Business $79 · **$59** · Premium $159 · **$119** (yearly saves 25%) | annual sales (up to $10K / $50K / $200K), file size, email credits; **priority support only on Premium**; product migration from Business | none (14-day trial, no card, 30-day money-back) | [sellfy.com/pricing](https://sellfy.com/pricing/) (first-hand 2026-09-27, page updated 2026-08-05) |
| **Payhip** | Free $0 · Plus $29 · Pro $99, monthly only | % of sale: 5% / 2% / 0%; features identical | yes (5%) | [sellfy.com/blog/payhip-pricing](https://sellfy.com/blog/payhip-pricing/) `[SECONDARY, 2026-08-12]`; payhip.com/pricing returns 403 to fetchers |
| **Gumroad** | $0 | 10% + $0.50 a sale (30% via Discover) | the whole product | [gumroad.com/pricing](https://gumroad.com/pricing) (first-hand 2026-09-26) |
| **Shopify Starter** | **$5** a month | sell through social and links, no storefront; 5% transaction fee | none | [shopify.com/starter](https://www.shopify.com/starter). Not offered to new stores as of 2026-09-26 per [ecomchief.com](https://ecomchief.com/blogs/news-1/shopify-starter-plan-2026) `[SECONDARY]` |
| **Canva for Education** | free for verified K-12 teachers | – | everything | [canva.com/education/teachers](https://www.canva.com/education/teachers/) (first-hand 2026-09-26) |
| **Vendoo** (cross-lister, the closest shape) | Starter $14.99 · $12.49 · Growth $29.99 · $24.99 · Pro $59.99 · $49.99 | features and AI or photo credits; items unlimited | none (14-day trial, card) | [vendoo.co/pricing](https://www.vendoo.co/pricing) (first-hand 2026-09-26) |

What the table says:

1. **Three paid rungs is the norm.** Sellfy, Vendoo and List Perfectly publish three or more paid plans, Payhip two above a free one. One paid plan beside a launch offer is the shape of a product still finding its price; a ladder is the shape sellers already compare.
2. **The entry rung sits at $5–$15.** Etsy Plus $10, Shopify Starter $5, Vendoo $14.99, Listelf $9.99, Flyp $9. The seller fee a TPT seller already pays is Premium at $59.95 a *year*, about $5 a month. $12 is under Vendoo's entry and a little over Etsy's, and $96 a year is less than two TPT Premium memberships.
3. **The middle rung sits at $29.** Sellfy Starter, Payhip Plus, Vendoo Growth $29.99, Crosslist Bronze $29.99, List Perfectly Simple $29. Sync stays at $29.
4. **The top rung sits at $59–$79.** Vendoo Pro $59.99, Nifty Pro $59.99, Sellfy Business $79 ($59 yearly), List Perfectly Pro $69. Studio at $59 is the bottom of that band. The one feature that consistently marks a top rung is **priority support**; Sellfy reserves it for Premium.
5. **Yearly discounts run 17–33%**, with the median at "2 months free". Keeping each yearly price at about **a third off**, as Sync already is, lets every card say "$20 a month, billed yearly" in the same breath.
6. **Nobody in the set sells a launch cohort or a paid onboarding call** beside the plans. Sellfy's migration is a feature of its Business plan. "Founding 100" and "Move with me" were the parts of our page that read as a start-up; removing them is what makes the product read as established.

## 3. Why these numbers

- **Starter 10 moves at $12** is $1.20 a move, a little above Sync's $1.16 ($29 / 25), so upgrading never costs more per move. Ten moves is five new resources a month onto both marketplaces, or ten onto one: the teacher who makes about one resource a week.
- **Studio 100 moves at $59** is $0.59 a move, half Sync's. That is the volume discount a catalogue of hundreds needs, and it undercuts the top pack's $0.79 a move only for a subscriber who keeps using the allowance month after month.
- **Storage** follows the capacity note: covers cost about 100 KB each and bundles stay on the seller's device, so 5 GB covers a Starter catalogue many times over. Studio keeps the 200 GB it was already given.
- **The daily sync on Starter** is a genuine difference that costs nothing to explain ("edits sent once a day"). It also keeps the scheduler's load proportional to what is paid.
- **Statistics and automatic publishing rules start at Sync.** Both matter only to a seller who publishes often, so they are what separates a weekly seller's plan from an occasional one's.
- **AI fill** stays "coming soon" on every paid card, without a count on the landing card (the founder's PDF feedback). The allowance is in the table (50 / 200 / 600) so the day it ships is one status change.

## 4. Move Packs: keep all five

Keep them, unchanged ($47 / 20 · $77 / 50 · $127 / 100 · $247 / 250 · $397 / 500, valid 12 months).

- They serve a buyer the tiers do not: **the seller who moves a shop once**. Starter would take ten months to move 100 resources. Sync would take four, which is about $116 plus a cancellation to remember, against $127 today for a Pack 100 with nothing to cancel.
- A pack does not compete with a tier. A subscriber can top up with one, and it is a ledger credit, not a plan (the 2026-09-20 decision), so it needs no extra gate.
- Zipsale's credits (from £0.18 an item, 12 months) are the one precedent, and packs are the one-off path the capacity note found cheapest to support.
- The alternative, dropping the two smallest rungs because a month of Starter or Sync is cheaper per move, would take away the only way to buy moves without subscribing. Revisit if fewer than 10% of pack buyers buy under 100 moves in the first two quarters.

## 5. Withdrawing Founding 100 and "Move with me"

- **Founding 100** (`founding_yearly`, $180 first year, $192 in years 2–3, 20 extra moves). Production runs Stripe in test mode, so no live payment has been taken. Before deploying, check `select count(*) from billing_subscription where provider_price_id = '<founding price id>'`.
  - **None** (expected): nothing to do beyond archiving the Stripe price.
  - **Any**: grandfather them. In Stripe, move each subscription to the new `pro_yearly` price with a coupon that gives the same money: 25% off the first year, then 20% off for two more years (the Discounts slice's coupon mechanism). Their 20 bonus moves are already in `move_ledger` with source `founding` and do not expire; that history stays. The ledger's `founding` source and `service_booking` stay in the schema as history; no new rows are written.
- **"Move with me"** (`move_with_me`, $99). No longer sold. Existing `service_booking` rows are history and stay. Anyone with a booked session keeps it; the founder honours it by hand.
- **Existing Sync subscribers** keep their plan, price and moves exactly; Sync did not change. The only new thing they see is that Studio is now on sale above them.
- **Operator Studio grants** (founder trials) keep working: the operator rung still only ever raises Studio's allowance.

## 6. What changes where

- `crates/tam-limits`: `Plan` gains `Starter` (wire `starter`) between Free and Subscriber. `PriceKey` becomes `starter_{monthly,yearly}`, `sync_{monthly,yearly}`, `studio_{monthly,yearly}` and `pack_{20,50,100,250,500}`. `Founding`, `FOUNDING`, `Service` and `SERVICES` are gone, and so are `founding_yearly` and `move_with_me`. Each recurring key names the plan it grants (`PriceKey::plan`).
- Migration `0090_tiers.sql` widens `entitlement_grant_plan_known` to include `starter`.
- The checkout grants whichever plan the price key names, not always Sync, and a change of price in the billing portal moves the grant with it.
- The landing page and the console Billing page draw the four plans from the same generated table, with a monthly/yearly toggle in the console.

## 7. Stripe objects

Create in test mode first, then live. One Product per plan and one Price per cadence:

| Product | Price key | Interval | Amount (USD) |
|---|---|---|---|
| Teachouse Starter | `starter_monthly` | month | 12.00 |
| Teachouse Starter | `starter_yearly` | year | 96.00 |
| Teachouse Pro | `pro_monthly` | month | 29.00 (exists) |
| Teachouse Pro | `pro_yearly` | year | 240.00 (exists) |
| Teachouse Studio | `studio_monthly` | month | 59.00 |
| Teachouse Studio | `studio_yearly` | year | 480.00 |
| Move Pack 20 / 50 / 100 / 250 / 500 | `pack_*` | one-off | 47 / 77 / 127 / 247 / 397 (exist) |

Archive the Founding and "Move with me" prices, and remove both from the price map, because a key the build does not know fails the map's parse at start-up.

## 8. Triggers that re-open this

- Starter takes more than 60% of new paid subscriptions for two quarters: the gap to Sync is too wide, so reconsider analytics on Starter or Sync's price.
- Fewer than 5% of subscribers are on Studio after two quarters: Studio is priced for a customer we do not have yet, so try $49 / $408.
- Any Studio subscriber exhausts 300 accrued moves twice in a quarter: offer the operator rung and consider a fourth rung.
