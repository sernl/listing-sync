# Pricing restructure and the Founding 100: proposal

- date: 2026-09-18
- status: proposal for the founder's decision; nothing in the plan table, the landing page or the console changes until each numbered decision below is taken
- supersedes nothing yet; extends `2026-09-12-pricing-and-tiers.md` with three evidence packs gathered 2026-09-18 (competitor crosslisters, teacher-creator economics, SaaS pricing practice), each with its URLs in `../../../..` session artefacts and summarised here with the sources that matter
- repo inputs: `docs/design/decisions.md` (2026-09-11 and 2026-09-12 entries), `docs/design/commercial-model.md`, `crates/tam-limits/src/lib.rs` (the plan table as built), `apps/landing/src/pricing.js`

---

## 1. The three facts that decide everything else

**We are alone in the category, and the category next door is crowded and cheap.** Three independent searches on 2026-09-18 found no tool that cross-lists between TPT, Tes, Classful or Teach Simple; Teach Simple's contributor terms forbid linking to the others, so no partner API is coming. The adjacent reseller crosslisters (Vendoo, Crosslist, Nifty, List Perfectly, OneShop, Zipsale, SellerAider, Flyp) cluster at $29.99–$45 a month for a full product, with budget entries at $9 (Flyp, unlimited) and $14.99 (Vendoo Starter), and every one of them bundles import into the subscription. Sources: vendoo.co/pricing, crosslist.com/pricing, nifty.ai/pricing, tools.oneshop.com/pricing, zipsale.co.uk/pricing, selleraider.com/pricing, joinflyp.com.

**The buyer's income and catalogue size move together, and the median seller earns about the price of the product.** Third-party modelling of TPT's 2024 yearbook (seolumina.com, 2026-02-04; the only published distribution since IXL bought TPT in 2023) puts most sellers at $0–$100 a month, a median around $27, catalogue sizes of 1–20 resources for hobbyists rising to 200–600+ for full-timers, and the top 1% earning more than the other 99% combined. Tes's own FAQ (updated 2024-03-07) sets royalties at 60%/70%/80% by rolling annual sales bands of £0–999, £1,000–5,999 and £6,000+, with a 20p fee on sales under £3 and a £1 floor. TPT Premium is $59.95 a year for an 80% payout; every seller earning anything holds it.

**This persona already pays tool-band prices, and the tools they pay for have been raising them.** Canva Pro is $18 a month, Kit is $39 a month (raised July 2026), Flodesk is $25–54 a month (flat rate withdrawn December 2025). The 2026-09-12 note assumed a $5–18 habitual band; the evidence now puts it at $18–39. $24 is mid-band, not top.

---

## 2. What is wrong with the current model

| Element | As built (`tam-limits`) | Problem the evidence exposes |
|---|---|---|
| Subscriber $24/$240, 400 resources, 20 migrations/month, 2 devices, 200 AI fills | one flat tier | Flat-fee is the normal early-stage choice (37% of sub-$5M-ARR companies, Growth Unhinged 2026-05-13) but its known failure is no expansion revenue, and the survey names it the number-one monetisation complaint. Studio is reserved and unsold, so there is nothing to expand into. |
| 20 migrations a month | protects the one-off ladder from a one-month subscription | This month's acceptance trial needed 30 in one day and was refused; a seller migrating a 150-resource catalogue needs eight months on the cap. It is the cap most likely to generate a support ticket and least likely to generate an upgrade, because no upgrade exists. |
| 2 devices | technical limit surfaced as a plan line | No competitor meters devices. The phone-and-tablet household is the normal seller; the cap reads as arbitrary. |
| Free tier: 20 resources, 1 marketplace | acquisition | It maps exactly onto the hobby tier ($0–50 a month) that the income distribution says does not convert, while the carded trial converts at roughly 31% against a no-card trial's 9% (ChartMogul 2026, via pulseahead.com). Freemium is best introduced two to three years in (Campbell, lennysnewsletter.com/p/saas-pricing-strategy). |
| Catalogue Import ladder $47/$77/$127/$247/$397 | one-off, five rungs | Sound on unit economics: at Tes's £1 floor a migrated worksheet repays $1.27 in one to three sales. But five SKUs invite the "my 60 merged to 45" refund argument, and Zipsale now sells the same thing as listing credits from £0.18 an item with a twelve-month expiry, debited at commit. |
| Founding 100: 25% year one, 20% years two to three, 20 free imports, 100 places | launch offer | Well shaped in three of four dimensions (see §4); the one gap is that a seat cap alone is not a bound. |

---

## 3. Proposed structure

Decisions are numbered; each stands alone.

**D1 — Position against the tool band, and say why on the page.** Lead the pricing page with marketplace coverage ("the only tool that lists to TPT and Tes") rather than feature counts, because the honest answer to "Flyp is $9" is "Flyp cannot see TPT". Keep the subscription at $24 a month and $240 a year (two months free, 17%, under Campbell's 20% ceiling and below the 28% market drift). Do not raise it yet: the median seller's whole income is about $27 a month, and the tool-band evidence argues for a second tier above, not a higher floor.

**D2 — Sell Studio now, at $44 a month or $440 a year, as the expansion tier.** The 2026 monetisation survey's prescribed fix for flat-fee is a premium edition at 50–100% over base, which is $36–48; the already-specified $44 sits inside it. Gate it on the seams competitors have proven convert: bulk verbs across the whole catalogue, 100 migrations a month, three devices, the hourly sync pull, 600 AI fills, one-day support. Do not gate it on catalogue size alone; the field gates depth of automation, not counts. This replaces the 2026-09-12 "reserved until a fifth of subscribers exceed 300 resources" rule, which measured a trigger that a single tier can never produce.

**D3 — Raise the Subscriber migration cap from 20 to 50 a month and drop the device cap to a soft limit.** Fifty covers a side-income catalogue (20–80 resources) in one or two months and still leaves the ladder as the fast path for a 250–500 catalogue. Devices: keep the entitlement token's device binding for revocation, but stop counting devices against a plan; if a bound is needed for abuse, make it five and say nothing about it on the page.

**D4 — Replace the free tier with a 14-day carded trial of Subscriber, and keep a genuinely free read-only catalogue.** A seller can import into the catalogue and browse it free forever (this is the top-of-funnel that costs us storage only); any write to a marketplace needs the trial or a plan. That preserves the acquisition value of "see your whole catalogue in one place" without giving the hobby tier a free crosslister.

**D5 — Keep the one-off Catalogue Import ladder on the landing page, but make it a credit balance in the product.** Buying a rung credits that many resource imports, valid twelve months, debited at commit after duplicate merge, top-up in-app at the rung's per-resource rate. The rungs stay ($47/20, $77/50, $127/100, $247/250, $397/500) because they are readable; the balance removes the refund argument and matches the one live precedent (Zipsale listing credits). Relabel the rungs by seller tier as well as count ("side income: up to 50"), since the data says the two co-vary.

**D6 — Add one ROI instrument to the pricing page: the Tes royalty-band calculator.** "You are £X from your next band" is the only quantified, seller-specific argument available (a 60→70% jump is a 16.7% raise on every later sale), it needs only the seller's rolling twelve-month Tes sales figure, and no competitor can make it. On TPT the equivalent sentence is "$24 is about seven extra sales a month at an 80% payout".

**D7 — Run a discrete-choice willingness-to-pay study on the founding cohort before locking Studio's price.** Every number above is competitor-anchored. The recommended instrument is three realistic bundles plus "would not buy", incentive-aligned, not Van Westendorp (Berman, lennysnewsletter.com/p/the-ultimate-guide-to-willingness). The founding list is exactly the sample.

---

## 4. Founding 100: keep it, bound it, do not deepen it

The question was whether it is a good idea at all. The evidence separates two things that are often confused.

A *lifetime deal* (perpetual licence, AppSumo-style 70/30 split, unbounded infrastructure liability, refund rates near 17%, shallow feature adoption) is the pattern that damages early SaaS companies (freemius.com/blog/saas-lifetime-deals, dodopayments.com, lifetimedealtech.com). Founding 100 is not that: it is a time-boxed discount on a subscription with no perpetual right. The severe failure modes do not apply.

A *founding-member offer* with a cap, a date and explicit terms is the pattern that worked for ConvertKit ($29 a month locked for the first 1,000), Superhuman ($30 a month for the first 1%) and Notion ($4 a month for the first 10,000). The practice literature converges on: cap **and** date, discount at or under 20% ongoing, the reversion to list written into the offer at sale time, and non-price value (allowances, priority support, a feedback seat) instead of a deeper discount.

Measured against that:

| Term | Verdict |
|---|---|
| 25% off year one | conventional; keep |
| 20% off years two and three | at Campbell's ceiling; keep because it ends. Do not extend. |
| 20 resources imported free | the best part of the offer; keep and relabel in whole units ("your first 20 resources on us") |
| 100 places | necessary but not sufficient as a bound |
| no end date, no written reversion | **the gap.** State "closes at 100 members or 2026-12-31, whichever first", and "renews at the then-current list price from year four". Say it now, not at renewal. |
| applies to the metered legs | decide explicitly: discount the platform fee only, not AI fills, whose gross margin is about 50% industry-wide |

One additional, honest use for the cohort: the product thesis rests on the 2.3% Tes-to-TPT catalogue ratio among dual-listers being unserved demand rather than absent demand. Nothing public resolves it. A hundred founding members who agree to share post-import Tes revenue for a year is the only instrument that will, and that is worth more than the discount costs. Write the feedback obligation into the offer as the price of the discount.

So: keep the Founding 100. Add the date and the reversion sentence. Ask for the data.

---

## 5. What changes if the founder accepts all of it

`tam-limits`: Subscriber `migrations_per_month` 20→50, `devices_max` 2→5 (unadvertised), Free becomes catalogue-only with `publish_marketplaces_max` 0 and a trial flag, Studio sold at $44/$440 with the gates in D2, `migration_only` becomes a credit balance with a twelve-month expiry and a per-resource top-up rate per rung. `FOUNDING` gains `closes_at` and a `reverts_to_list_after_years: 3` that the console renders. The landing page leads with coverage, carries the calculator, and states the founding bound. Paddle gains the Studio price and a top-up price per rung.

None of it is built until each decision is taken; the migration cap change (D3) and the founding bound (D4 terms) are the two that should not wait, because the first blocks the acceptance trials and the second is cheaper to state before the first hundred sign up than after.

---

## Sources

Primary, read 2026-09-18: vendoo.co/pricing; crosslist.com/pricing; nifty.ai/pricing; tools.oneshop.com/pricing; zipsale.co.uk/pricing and /listing-credits-zipsale; selleraider.com/pricing; joinflyp.com; help.teacherspayteachers.com/hc/en-us/articles/360044219891 (TPT seller fees, updated 2026-07-22); tes.com/policies/help/selling-resources-faq (updated 2024-03-07); lennysnewsletter.com/p/saas-pricing-strategy (Campbell); lennysnewsletter.com/p/the-ultimate-guide-to-willingness (Berman); growthunhinged.com/p/the-state-of-b2b-monetization-in-2026 (Poyar, 2026-05-13); freemius.com/blog/saas-lifetime-deals.

Secondary, directionally reliable: seolumina.com (TPT income distribution model, 2026-02-04); pulseahead.com (ChartMogul trial conversion splits); stylefactoryproductions.com (Canva pricing history); automationatlas.io (Kit pricing); emailtooltester.com (Flodesk pricing); datadab.com and launchadvisor.co (founding-offer examples and terms); teachsimple.com contributor onboarding; Fiverr TPT VA listings ($28–91).

Gaps: no TPT- or Tes-published seller income or listings-per-seller distribution exists; List Perfectly's dollar prices are JS-rendered and were not re-confirmed; the 2026 monetisation report's full text is paywalled.
