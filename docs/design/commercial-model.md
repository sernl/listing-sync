# The commercial model

This document holds the measured pools, the pricing analysis and the cost model that the design specification summarises in three sentences and does not reproduce.
It is carried from the round-two addendum wherever the two research rounds disagree, and every figure below is round two's.

## The measured pools

Three pools were measured: the TPT-and-Tes intersection at about 4,000 sellers, from 474 TPT store slugs probed under three name variants where 16 had a name-identical Tes shop and 10 had any live inventory, implying roughly 3,900–6,200 dual-listers platform-wide; Tes American-orientation shops at 2,000–5,000, from 23,266 of 1,039,401 resources with 710 sampled; and the Etsy teacher-resource segment, unmeasured with a floor of 5,000-plus items behind a display-capped market page.
The samples are drawn from search-ranked results and biased toward successful sellers, which makes these upper bounds.
One clarification matters and is a reading of the founder's decision rather than a new measurement: the 4,000 figure bounds the cross-marketplace product because it counts sellers already on both platforms, whereas the wedge is bounded by Tes's whole author base and that denominator has never been measured as a seller count, so the ceiling below is computed on the conservative pool and understates the wedge.

| Scenario | Steady-state subscribers | ARR at $29 |
|---|---|---|
| Pool 4,000, 10% converted, 6% churn | 185 | $64k |
| Pool 4,000, 10% converted, 3.5% churn | 317 | $110k |
| Pool 4,000, 20% converted, 3.5% churn | 635 | $221k |
| Pool 2,000, 10% converted, 6% churn, $19 | 93 | $21k |

## Churn and the subscriber ceiling

Steady-state subscriber count is gross monthly adds divided by monthly churn, and round one modelled 1,000 simultaneous subscribers at zero churn, which is not a scenario; ChartMogul's cohort data across more than 2,100 SaaS businesses puts top-quartile annual gross retention at 60–70 percent for average revenue per account under $50 a month, compounding to 3.0–4.2 percent monthly for top-quartile performers and nearer 5–6 percent for median ones.
Round one's "$200k–250k ARR at a thousand paying customers" is therefore replaced by $60–220k ARR at 150–650 subscribers, and the kill criteria re-anchor to subscriber count against pool share rather than raw revenue.
There is a churn hazard specific to this product's shape: it is a migration tool wearing a subscription's clothes, because the initial duplication of a back catalogue is roughly twenty times the steady-state monthly need, so expect a churn spike two to three months after signup and package against it deliberately.

Ten confirmed active dual-listers hold 173 Tes listings against 7,644 TPT listings, which is 2.3 percent of their combined catalogue, and the figure is ambiguous rather than adverse: it measures behaviour under manual friction and cannot distinguish sellers cherry-picking because Tes traffic does not justify more, in which case there is no market, from sellers cherry-picking because uploading two hundred items by hand is unbearable, in which case removing that friction is the unlock.
No amount of further desk research separates them, and the counter-evidence is the founder's own catalogue, most of which is on Tes only because cross-listing by hand is not worth the time, which makes the founder customer zero and the first milestone's payoff directly measurable against a real catalogue.
The parallel Reddit-sourced claim that sellers migrate only hand-picked best-sellers could not be verified because every access route was blocked, so it is medium confidence corroborated by the independent behavioural measurement rather than confirmed in its own right.
Amazon Ignite was this exact business with Amazon's balance sheet behind it and it is gone, with the last archived capture carrying the notice that as of January 30, 2023 Digital Educational Resources publishing capabilities would be discontinued and ASINs delisted, against a programme offering 70 percent royalty and the explicit answer that sellers could also sell elsewhere.
That is a demand-side signal about the size of the non-TPT teacher-resource market and becomes a standing question rather than a settled fact, and one distinction is a judgement rather than a research finding: Ignite is evidence against a many-channels bet, and the wedge is a second-inventory bet inside a marketplace the seller has already chosen, so the signal argues more strongly against connector three than against connector one.

## The competitive picture

No product cross-lists between any two education marketplaces, and this is now the best-tested claim in the research set, which should be read as a warning rather than validation, because the niche is empty partly because free substitutes absorb it: Teach Simple states that its team will upload all your products for you, and TPT ships a Virtual Assistant seat expressly permitted to create new resources and update product listings.
The nearest real competitor is already selling to the exact customer — Mr Joel's TPT Seller SEO and Listing Optimizer App at $79.99 one-time with 180 ratings at 4.97 stars, advertising a bulk product importer and operating on TPT product and edit pages — and one attribution defect must not be repeated, because a six-item feature quote attributed verbatim to that product page appears on none of the three pages cited.
Two consequences follow: stop treating AI listing rewriting as a paid differentiator, since it is already sold to this persona at $79.99 one-time; and note that third-party write operations against TPT listings are an established commercial capability rather than a novel one, since Bearwood Labs has sold a Product Description Editor Pro at $29.00 on TPT itself for roughly five years.
The category exists and is commercial for every adjacent marketplace and stops precisely at education, so build risk is lower than round one assumed while the demand question is exactly as open.

## Pricing

The $19 anchor is dropped, because it rested partly on a comparator three research passes could not identify and for which round one gave no source.
Analytics-only tools sell at $5.99 and $9.99 a month, while anything that moves a catalogue sells higher: List Perfectly starts at $29 and gates analytics onto its $49 tier, Crosslist runs $29.99 to $44.99, ExportYourStore runs $29 to $249 by listing count with a one-time $199 setup for its hardest channels, and Vela runs $29.95 to $54.95 per shop for a strictly less capable product on the digital axis.
The structure to test against a flat monthly subscription is a one-time onboarding fee plus a lower recurring price, on the ExportYourStore precedent, because value here is bursty and a flat subscription fights that shape: modelled at $299 one-time plus $19 a month, lifetime value is about $518 against $237 for subscription-only at 6 percent churn.
The floor for a pure subscription is $29 a month tiered by catalogue size, and a per-listing credit meter is the alternative worth testing beside it, since the closest live analogue sells credit packs from $1.99 with no subscription and that meter fits the measured five-to-twenty-listing behaviour rather than fighting it.
Two structural additions belong with the pricing decision: restore an analytics surface built strictly from seller-owned data, because analytics is the documented upgrade driver in both close comparables so deleting it removes the renewal mechanism while leaving the churn; and require a credit card on the free trial, because ChartMogul's study of 200 products found median free-to-paid conversion of 8 percent while trials requiring a card see 30 percent.

## Cost per user

Modelled cost per user per month falls from $12.96 at 100 users to $7.77 at 300 and $5.38 at 1,000, giving contribution after support at $29 of $16.04, $21.23 and $23.62.

| Line | 100 users | 300 users | 1,000 users |
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
| Total | $12.96 | $7.77 | $5.38 |

No single line dominates at every scale, and an earlier draft of this document said one did.
Amortised fixed compliance and legal is the largest line at 100 users at $5.17 and falls to $0.52 by 1,000, at which point payments at $1.89 and support at $1.50 are the two largest.
Support is the only line whose rank is stable across the range, sitting first or second at every scale.
The automation-compute line is zero because that compute runs on hardware the founder already owns, so it is a capacity and electricity line rather than a per-user fee and should be read as not separately billed rather than free; round one's counterfactual priced the same work at roughly $0.50–$1.70 per user per month on managed browser infrastructure, and cost was never the argument for or against server-side execution and should not be revisited as one.
Payments is computed at the Australian international rate of 6.5 percent all-in on $29, rising to about 10.0 percent on Stripe Managed Payments, and merchant-of-record fees are charged on the tax-inclusive total rather than the net price, so a UK cohort at 20 percent VAT costs meaningfully more than the headline rate suggests, and whether Stripe's 3.5 percent is computed the same way is unverified and is one question to Stripe.
Support labour is the widest error bar in the model, modelled at $50 an hour and twelve minutes a ticket with the rate falling from 0.4 to 0.15 tickets per user per month, has no citable benchmark, and must be instrumented from the first ten paying customers rather than planned against.
Modelled break-even against a $5,000-a-month founder opportunity cost lands at roughly 250 to 300 customers, which is inside what the measured pool can supply where round one's 400 was not, but that figure rests on a modelled support rate and is a hypothesis to instrument rather than a result.
Lifetime value at $29 and 300-user costs is $459 at 5 percent monthly churn, $656 at 3.5 percent and $287 at 8 percent, so an LTV-to-CAC ratio of three implies an affordable acquisition cost of $96 to $219 and a twelve-month payback rule implies about $275, which makes round one's "unable to support paid acquisition" too strong: the business cannot bid in an open auction against advertisers with ten times the revenue per account, and it can afford up to roughly $100 to $200 in targeted channels.
Manage contribution after support rather than gross margin, because round one's 88–91 percent figures are arithmetically fine and strategically misleading: the two lines they exclude are support labour and founder time, and both grow with customers.

