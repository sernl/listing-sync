# Billing vendor memo

Every page cited here was fetched on 2026-08-30.
Where two pages from the same vendor disagree, both figures are given rather than reconciled.
Nothing below is tax advice; the New Zealand GST questions need an accountant before the first paid signup.

## 1. Merchant of record, or payment facilitator

This is the decision that constrains every other one, and it is not primarily a fee decision.

Under a payment facilitator, we are the seller of record for every subscription.
Stripe states the consequence in its own words: "As a business, you're required to identify the states, provinces and countries where you have tax obligations. You must then register with the tax authorities in the applicable jurisdictions" (https://docs.stripe.com/tax/registering).
Its filing page is equally direct: "You must file and remit the tax you collect for every location where you're registered" (https://docs.stripe.com/tax/filing).
Stripe Tax calculates and collects; it does not assume the obligation.
Filing is brokered out to partners — TaxJar for the US, Taxually for global, Marosa for the EU, Hands-off Sales Tax for US and Canada — priced separately from the calculation fee.

Under a merchant of record, the vendor is the legal seller to the teacher.
Lemon Squeezy states it plainly: "Lemon Squeezy is your merchant of record. We take on the liability of tax collection and calculation and pay taxes on your behalf" (https://docs.lemonsqueezy.com/help/getting-started/fees).
Paddle describes itself as "the only complete Merchant of Record solution purpose-built for SaaS and apps" (https://www.paddle.com/pricing), and Polar's fee page repeats "Since Polar is the Merchant of Record" throughout (https://polar.sh/docs/merchant-of-record/fees).
Our own tax position then collapses to a single business-to-business supply of services to one overseas entity, rather than several hundred thousand consumer supplies across dozens of jurisdictions.

For a solo New Zealand founder the asymmetry is severe.
Inland Revenue sets the domestic registration threshold at NZD 60,000: an entity must register if "your turnover was at least $60,000 in the last 12 months, or you expect it will be at least $60,000 in the next 12 months" (https://www.ird.govt.nz/gst/registering-for-gst).
Sales to overseas customers are generally zero-rated rather than exempt, and Inland Revenue conditions that on evidence: there must be "sufficient evidence that that customer is overseas, and that the goods or service have been exported" (https://www.ird.govt.nz/gst/charging-gst/zero-rated-supplies).
So the domestic side is cheap: NZ-resident teachers attract 15%, overseas teachers attract 0%, and one NZ return covers it.
I did not find an IRD page stating whether zero-rated export turnover counts toward the NZD 60,000 threshold, so that interaction is an open question for the accountant, not an assumption to build on.

The exposure is entirely overseas, and it does not wait for a threshold in the way NZ does.
That is precisely the surface an MoR removes and Stripe Tax only instruments.
For a business with one person and no finance function, the registration-and-filing calendar is the scarce resource, not the fee difference computed in section 2.

One finding closes off the obvious hedge.
Stripe now sells its own MoR product, Managed Payments, built from the Lemon Squeezy acquisition, and it lists supported business locations explicitly: Canada and the US; the EU plus GB, CH, NO, GI, LI; and in Asia Pacific only AU, HK, JP and SG (https://docs.stripe.com/payments/managed-payments/eligibility).
New Zealand is absent.
A NZ entity therefore cannot start on Stripe Payments and switch on Stripe MoR later; that path does not exist today.

## 2. Candidates

Fee arithmetic below is applied to a USD 20.00 monthly subscription, the midpoint of the 10-30 band, and is my arithmetic on the vendors' published rates.

### Stripe (Payments + Billing + Tax + Checkout)

Not a merchant of record; we remain the seller.
Card rates conflict between two Stripe pages fetched the same day: https://stripe.com/nz/pricing gives 2.65% + NZ$0.30 domestic and 3.5% + NZ$0.30 international, while https://stripe.com/billing/pricing gives 2.7% + NZ$0.30 and 3.7% + NZ$0.30.
Both pages add 2% when currency conversion is required.
Billing is 0.7% of billing volume pay-as-you-go, with monthly plans starting at NZ$1,000/month on a one-year contract.
Tax Basic is 0.5% per transaction no-code, or NZ$0.75 per transaction via API; Tax Complete starts at NZ$150/month on a one-year contract and bundles some filing credits.
Checkout is included with Payments; a custom domain is US$10/month.
For a NZ account, nearly every customer card is an international card, so the realistic all-in is 3.5-3.7% + 2% conversion + 0.7% Billing + 0.5% Tax = 6.7-6.9% plus NZ$0.30, before any registration or filing cost.
Subscriptions, Smart Retries dunning, and the hosted customer portal are included with Billing.
Payouts to a NZ bank are standard-schedule at no listed fee; Instant Payouts cost 1.5% with a NZ$0.50 minimum.
Lock-in is low and API maturity is the highest of the four; the liability, however, stays with us.

### Paddle

Merchant of record, 5% + 50c per transaction on the pay-as-you-go tier, no monthly fee (https://www.paddle.com/pricing).
On USD 20 that is USD 1.50, or 7.5%.
Whether the 5% applies to gross including tax or to the net is not stated on any Paddle page I could retrieve; treat it as unresolved.
Paddle's page notes that products sold under USD 10 require custom pricing, which matters if the entry tier lands below that line.
Subscription billing, fraud and chargeback coverage, localized checkout and support are bundled; Retain churn recovery is included when Paddle is the billing platform, and only priced separately as a standalone product on someone else's stack.
The hosted customer portal is on by default, with authenticated deep links minted through POST /customers/{id}/portal-sessions (https://developer.paddle.com/api-reference/customer-portals/create-customer-portal-session/).
New Zealand does not appear on Paddle's unsupported-country list (https://www.paddle.com/help/start/intro-to-paddle/which-countries-are-supported-by-paddle).
Payouts are monthly by wire or Payoneer, sent by the 15th, minimum threshold USD 100, with a possible USD 15 SWIFT fee in some countries (https://www.paddle.com/help/manage/get-paid/when-and-how-do-i-get-paid).
Lock-in is real in a way the pricing page's "no lock-in periods" line does not address: that line is about contract term, whereas the switching cost is that Paddle holds the customer of record and the stored payment credentials for every live subscription.

### Lemon Squeezy

Merchant of record, 5% + 50c on the total order value, with surcharges that stack: +1.5% for international (non-US) transactions, +1.5% for PayPal, +0.5% for subscription payments (https://docs.lemonsqueezy.com/help/getting-started/fees).
On USD 20 from a US card that is 5.5% + 50c = USD 1.60, or 8.0%; from a non-US card, 7% + 50c = USD 1.90, or 9.5%.
Payout fees are additional: via Stripe, free to US bank accounts and 1% per payout to non-US accounts; via PayPal, 50c in the US and 3% capped at USD 30 outside it.
Payouts run twice monthly, held 13 days, minimum USD 50; New Zealand is on the bank-payout list (https://docs.lemonsqueezy.com/help/getting-started/getting-paid, https://docs.lemonsqueezy.com/help/getting-started/supported-countries).
Products under USD 10 again require custom pricing.
Subscriptions, dunning and a self-service customer portal are all present.
The direction of the product is the problem.
Stripe acquired Lemon Squeezy in July 2024, and the site header currently promotes a post titled "2026 Update: Lemon Squeezy + Stripe Managed Payments" whose visible excerpt reads "we're excited to share the first major milestone in that journey"; the article body returned 403 to the fetcher and I have not read it, so treat its detail as unread.
What is verifiable is that Stripe now ships a competing first-party MoR built by that team, and that it excludes NZ sellers.
Choosing Lemon Squeezy today means betting on a product whose owner sells its successor.

### Polar

Merchant of record, with plan tiers published at https://polar.sh/docs/merchant-of-record/fees: Starter free at 5% + 50c, Pro USD 20/month at 3.8% + 40c, Growth USD 100/month at 3.6% + 35c, Scale USD 400/month at 3.4% + 30c.
An organisation created today starts on Starter; the older Early Member rate of 4% + 40c is closed to organisations created on or after 27 May 2026.
International (non-US) cards add 1.5%; the separate +0.5% subscription fee applies to Early Member only.
The fee base is documented and is the gross including tax — Polar's worked example charges its percentage against a USD 37.50 total on a USD 30 product plus 25% Swedish VAT.
On USD 20 that is USD 1.50 (7.5%) from a US card and USD 1.80 (9.0%) from a non-US card on Starter; Pro reaches 5.8% plus its monthly fee, and Polar puts the crossover at roughly USD 1,379/month in sales.
Disputes cost USD 15 each regardless of outcome.
Payouts run through Stripe Connect Express and carry Stripe's costs unmarked-up: USD 2 per month of active payouts, 0.25% + USD 0.25 per payout, and 0.25% (EU) to 1% elsewhere for cross-border conversion.
New Zealand is listed as a supported payout country (https://polar.sh/docs/merchant-of-record/supported-countries).
Subscriptions, a customer portal, and usage-based billing are documented product surfaces.
The concern is company maturity rather than API quality: Polar restructured its pricing three months ago, and a young MoR carries counterparty risk that Paddle, at its size, does not.

## 3. Integration shape for this stack

The launch integration is the same shape for all four: a hosted checkout URL the SvelteKit dashboard links to, one axum route that verifies a signature and updates an entitlement row in Postgres, and a billing page that links to the vendor's hosted portal.
No candidate requires a Rust SDK, and none of the four publishes one — Stripe's official server SDKs are Ruby, Python, Go, Java, Node, PHP and .NET (https://docs.stripe.com/libraries), and Paddle's official list is Go, Node, PHP and Python.

Stripe: async-stripe is the community binding (v1.0.0-rc.8, Apache-2.0, 5.1M downloads, 747 stars, last pushed 2026-08-28) and is still a release candidate.
Signature verification is fully specified for manual implementation: split the Stripe-Signature header on commas, take t and v1, HMAC-SHA256 over timestamp + "." + raw body, constant-time compare, reject stale timestamps (https://docs.stripe.com/webhooks).
Minimal path: Checkout Session, then checkout.session.completed plus customer.subscription.* plus invoice.payment_failed, then a Billing portal session link.
Plain HTTP with hmac and sha2 is the right answer here; async-stripe buys typed models we barely need for four event types.

Paddle: paddle-rust-sdk is unofficial (v0.20.0, Apache-2.0, 28 stars, last pushed 2026-08-25) and too thin a base to gate a dependency on.
Signature verification is documented for manual implementation: Paddle-Signature carries ts and one or more h1 values, HMAC-SHA256 over ts + ":" + raw body, timing-safe compare, with the official SDKs defaulting to a five-second tolerance (https://developer.paddle.com/webhooks/signature-verification).
Multiple h1 values exist during secret rotation, so verify against any of them.
Minimal path: a Paddle hosted checkout link, then subscription.created / subscription.updated / subscription.canceled, then a portal session link.
Five seconds is a tight replay window for a webhook crossing the Pacific; implement a wider tolerance deliberately.

Lemon Squeezy: the lemonsqueezy crate is unofficial and thin (v0.1.3, last published 2025-07-21).
Verification is an HMAC-SHA256 hex digest of the raw body compared against the X-Signature header (https://docs.lemonsqueezy.com/help/webhooks/signing-requests).
There is no timestamp in the signed material, so the signature alone gives no replay bound; we would have to dedupe on event identity ourselves.
Minimal path: a hosted checkout link with custom data carrying our org id, then order_created / subscription_created / subscription_updated / subscription_expired, then the LS customer portal.

Polar: webhooks follow the Standard Webhooks specification (https://polar.sh/docs/integrate/webhooks/endpoints), which is the best-served option in Rust.
The standardwebhooks crate (v1.0.1) was last published 2024-03-04 even though the upstream spec repository is active, and axum-standardwebhooks (v1.0.0, 2025-03-14) is an axum extractor that drops straight into our router.
Minimal path: a Polar checkout link, then order and subscription events, then the Polar customer portal.
Either crate is small enough to vendor the verification logic instead, which keeps the dependency gate closed.

## 4. Recommendation

Take Paddle, as merchant of record, at 5% + 50c.

The tradeoff, stated: Paddle costs about 7.5% of a USD 20 charge against roughly 6.7-6.9% plus NZ$0.30 for the full Stripe stack, so the MoR premium here is under a point — far less than the folk figure — because a New Zealand Stripe account treats nearly every customer card as international and adds 2% for conversion on top.
For that point we stop owning foreign registration, filing and remittance, which is the obligation most likely to consume a solo founder's attention at exactly the wrong moment.
Paddle over Polar on counterparty risk and payout simplicity; Paddle over Lemon Squeezy because Stripe is building Lemon Squeezy's replacement and has excluded NZ sellers from it; Paddle over Stripe because Stripe's cheaper number is not cheap once the compliance work it hands back is priced.

Three things would reverse it.
If the entry tier prices below USD 10, Paddle's flat 50c and its under-USD-10 custom-pricing rule both bite, and Polar Starter or the Stripe stack become the better arithmetic.
If the accountant judges realistic near-term overseas exposure small enough to defer foreign registration, Stripe's stack wins on cost and on API maturity, and we revisit at the first threshold breach.
If Stripe adds New Zealand to Managed Payments' business locations, that becomes the strongest option outright, because it puts MoR economics inside the SDK and event vocabulary we would otherwise be reimplementing.

## 5. Founder questions

1. Merchant of record, or Stripe plus Stripe Tax with the registration and filing calendar staying ours?
   Recommended: merchant of record, for the reasons in section 1.
2. If merchant of record, Paddle or Polar?
   Recommended: Paddle, accepting roughly the same headline rate for a materially larger and older counterparty.
3. Will the entry subscription tier be priced at USD 10 or above?
   Recommended: yes, at USD 12 or above, so that no candidate's under-USD-10 custom-pricing rule applies and the fixed 50c stays under 5% of the charge.
4. Does the Rust API take a vendor crate as a dependency, or plain HTTP plus hmac, sha2 and subtle for signature verification?
   Recommended: plain HTTP plus the three primitives, since none of the four vendors ships an official Rust SDK and every signature scheme is fully specified.
5. Do we engage an NZ accountant before the first paid signup, specifically on whether zero-rated export turnover counts toward the NZD 60,000 threshold and on what evidence of customer location Inland Revenue expects?
   Recommended: yes, before launch rather than after, because the evidence requirement shapes what the checkout has to capture and store.
