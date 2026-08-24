# The compliance floor

This is a sibling of `2026-08-25-listing-sync-design.md` and is referenced from its compliance section.
Costs are modelled from published rate cards rather than quoted, and every one should be re-priced before it is relied on.

The organising principle is to minimise what a held session can reach, marketplace by marketplace, and to make that minimisation mechanical rather than procedural.
An allow-list enforced in Rust at the driver layer is worth more than any policy document, because it converts scope decisions from discretionary into evidenced, which is the difference that matters under a statutory tort requiring intention or recklessness rather than negligence.

## The severity inversion that puts the wedge on the wrong side

A TeachersPayTeachers Virtual Assistant session cannot reach earnings, and all banking and tax identity lives in Hyperwallet behind a separate login, so a TPT incident can honestly report that no financial account data was exposed.
Tes holds the seller's bank account details in-account, reachable from the same Author Dashboard the automation traverses via the Withdraw flow.
Tes is the easier technical target and the higher-severity compliance target, and Tes is the marketplace the first chargeable product runs on.

The buyer-data assumption is only partly right and the correction is favourable.
TPT withholds buyer contact details from sellers by design and routes seller-to-buyer contact through its own support team; Tes approximates buyer location "for data protection purposes" and exposes country plus a transaction identifier rather than a name.
The sharpest third-party personal data is elsewhere — the seller's own bank details on Tes, and TPT school rosters carrying teacher first name, last name and email address — so deny-list the roster and account-administration routes alongside the payout routes, and do not persist question-and-answer page content at all.

## Stage A, before the founder's own first use

| Item | Owner | Indicative cost |
|---|---|---|
| Navigation allow-list in code, with a test that fails on any route outside it | Founder | Nil |
| Per-connection data-encryption-key envelope encryption, AAD-bound | Founder | Nil |
| Key-encryption-key escrow and recovery procedure, tested | Founder | Nil |
| Global revocation command plus a timed drill | Founder | Nil |
| Per-field audit log shipped off-box via `services.vector` | Founder | Nil |
| Forced row-level security plus the cross-tenant negative test | Founder | Nil |
| Offsite backup configured, with one measured restore | Founder | Nil |
| Written pre-approval enquiry to Stripe under SSA 1.2(a)(ix) | Founder | Nil |
| Etsy Personal App plus Commercial Access request filed | Founder | Nil |
| Canary seller account in the founder's own name | Founder | $29 one-time, TPT Basic Seller |

The allow-list needs a test that fails on any route outside it, not a code review that notices one.
The revocation command must clear every session ciphertext, destroy every tenant data-encryption key, halt the scheduler, kill in-flight leases within seconds and notify every seller; drill it and record the wall-clock time, because that number is the honest containment window.
The audit log records intended mutation, observed post-write state and the diff — the same read-back that correctness already requires, so the marginal cost is storage — and it matters because TPT gives the seller no usable record of delegated edits, which makes our log the only record either party will hold.

Two items that the third drafter placed here are moved to Stage B, and the move follows `decisions.md` rather than new evidence.
The decision record states that no marketplace permission enquiries are being sent yet and that the Tes path is proved first, so the duplicate-upload question to `authors@tes.com`, the partnership pitch to `partnerships@tes.com`, the TPT Publisher Membership enquiry and the Platform-to-Business Article 9 data-access requests all wait until Stage B.
The reasoning that makes the move coherent rather than merely obedient is that Stage A covers the founder automating his own account, which needs no more permission than any author automating their own uploads, whereas Stage B is the point at which the operator rather than the seller performs the act.

## Stage B, before the first external customer

| Item | Owner | Indicative cost |
|---|---|---|
| Jurisdiction and operating-entity decision | Founder plus adviser | Advice fee, unquoted |
| Customer terms drafted to the chosen consumer-law regime | Technology lawyer | $3,000–8,000, or part of AUD 8,000–15,000 |
| Click-through DPA, SCCs plus UK Addendum, one transfer risk assessment | Lawyer, one engagement | $1,500–4,000 |
| Automation-posture opinion covering CFAA and cross-border terms | Boutique technology or IP counsel | $3,600–13,000 |
| Privacy opinion on buyer-data collection and the small-business exemption | Privacy lawyer in the chosen jurisdiction | $1,000–3,000 |
| Technology liability plus cyber cover | Founder, via broker | About A$2,500–5,000 a year |
| EU and UK Article 27 representatives | Prighter or equivalent | €840–1,700 a year |
| Written enquiries to `authors@tes.com`, `partnerships@tes.com` and TPT | Founder | Nil |
| Article 9 data-access disclosure requested from both platforms | Founder | Nil |
| Incident runbook written to GDPR's 72-hour clock | Founder plus lawyer | Part of legal |
| Self-serve connections page with revoke and delete | Founder | Nil |
| systemd confinement of the driver units | Founder | Nil |
| Wind-down clause and export commitment in the terms | Founder | Nil |
| Stripe billing live, second processor onboarded dormant | Founder | Processing fees |

One item moves earlier than the round-two addendum placed it, and the reason is the founder's architecture decision rather than new evidence.
Round two scheduled the automation-posture opinion before the TPT connector, on the reasoning that the seller was the actor and TPT carried the sharper clauses.
Under server-side automation the operator rather than the seller performs the act under both marketplaces' terms, which converts a grey area about what a seller may automate for themselves into a sharper question about what a vendor may do on their behalf, so the opinion is needed before the first external customer on Tes rather than before TPT.
The canary decision is downstream of the same opinion.

The self-serve connections page is the transferable lesson from the closest regulated precedent.
The Plaid settlement turned on over-collection and on a login screen with "the look and feel of the user's own bank account login screen", and the relief included retention limits, deletion, and a portal where consumers manage linked accounts.
Build that portal before the complaint, not after: every held session, when it was last used, and seller-initiated revoke and delete.

## Stage C, before general availability

| Item | Owner | Indicative cost |
|---|---|---|
| CrowdSec and auditd with alerting routed to phone | Founder | Nil |
| Severity routing and alert batching | Founder | Nil |
| Quarterly restore drill and annual revocation drill, documented | Founder | Nil |
| Connected-seller cap decided and enforced | Founder | Nil |
| Per-marketplace public status page | Founder | Nil |
| Software escrow including the signing key | Codekeeper or equivalent | About $1,670 a year |

Escrow is deferred to roughly two hundred paying customers, because at about $139 a month plus $199-an-hour release processing it is seven customers at $29 and indefensible early.
Until then the commitment is contractual: sixty days' notice and the export endpoint kept running through that window.
Round two's other wind-down promises — a final signed pack with a long expiry so the client keeps working in local-only mode, and the client open-sourced — do not transfer to this architecture, because there is no client that can keep working without the server.
Under server-side automation the wind-down promise must be a data export rather than degraded operation, and the customer terms should say so.

## Payment acceptance, which is a kill risk rather than a fee question

The two default indie-SaaS merchants of record are the two worst choices for this specific product, and the disagreement between the round-one and round-two cost lanes is settled on clause text rather than on price.
Polar's acceptable-use policy names "Services to circumvent the rules, paywalls or terms of other services" as an outright prohibited category, alongside "Any product or service that enables unauthorized access to data belonging to another party".
Paddle's policy, last updated 13 April 2026, prohibits any product that "infringes upon, or enables the infringement upon copyrights, trademarks, terms and conditions, or trade secrets of another party" — the words "terms and conditions" appear expressly — and separately bans "Captcha Solving" as a category.
Two merchant-of-record candidates prohibit this product by name, and the round-one recommendation of Paddle at 5% plus 50¢ is withdrawn.

Acceptance is not a fee comparison, and discovery would come after launch when customers are already subscribed and their sync is running.
The Paddle CAPTCHA clause also couples the billing rail to the adapter roadmap, so rule out CAPTCHA solving and Paddle in the same decision, which the compliant automation envelope already does independently.

Stripe's restricted-businesses list contains no anti-automation, anti-scraping, terms-circumvention or credential clause at all; the only clause that reaches this product is the general third-party intellectual-property one, and the Services Agreement provides written pre-approval at 1.2(a)(ix).
Approach Stripe first and in writing, before any billing code is written, because asking creates a written record and discovering the answer after launch is worse than a refusal.
FastSpring's prohibited list is short and does not mention automation or third-party terms, making it the clean documented fallback, though its pricing is unpublished and must be quoted and it pairs an open-ended termination right with a 180-day balance hold.
Lemon Squeezy is being folded into Stripe Managed Payments and should not be built on.

Two consequences follow regardless of which processor is chosen.
Stripe Managed Payments can "issue refunds within 60 days of purchase in certain cases", so the churn model must assume refunds the founder does not control, and the sync-failure blast radius becomes a billing-continuity risk as well as a trust one.
And a second processor stays onboarded and dormant, with at least one month of operating cost held outside the processor's balance, because Stripe retains discretionary suspension hooks this business will sit near for its entire life.

## The jurisdiction fork, unresolved

The operating entity and jurisdiction are undecided, and the decision record carries that as an open question forking the privacy regime, the consumer-law regime, the insurance market and the customer terms.
It is not resolved here, and the checklist above names the obligation rather than the statute wherever the statute depends on the fork.

The ICO lists New Zealand under full adequacy and Australia nowhere, so on the transfer axis alone New Zealand is the cheapest of the three candidate structures and Australia the most expensive.
The Article 27 representative obligation attaches regardless of adequacy, because the "occasional" derogation is unavailable to a continuous subscription service, so that line does not move with the fork.
Tax residency, the founder's actual residence, and which consumer-guarantee regime applies were not researched and belong to an adviser.
CFAA extraterritorial exposure also forks, since the Ryanair district court held "protected computer" reaches computers outside the United States, as does which geographic variant of the Tes General Terms binds — from a New Zealand egress they resolve to Tes Aus Global Pty Limited under Australian law, and `/en-gb/policies/general-terms-business` serves a different document.

### What follows under an Australian answer

The round-two analysis of consumer law, liability and privacy is conditional on an Australian answer, and if the answer is different this block is replaced rather than amended.

Every customer is a "consumer" under Australian Consumer Law regardless of business use, because the prescribed threshold is $100,000 under Competition and Consumer Regulations 2010 reg 77A rather than the $40,000 on the face of the Act.
Section 60 guarantees services "will be rendered with due care and skill", section 64 voids any term purporting to exclude it, section 64A(2) permits limitation only to re-supply or its cost, and section 64A(3) defeats even that where reliance is not fair and reasonable with the court directed to "the strength of the bargaining positions".
Section 267(4) then permits recovery of "any loss or damage suffered by the consumer because of the failure to comply with the guarantee if it was reasonably foreseeable", which is precisely the suspended-store, lost-income case.
The unfair-contract-terms regime compounds it: section 23(4) makes this a small business contract, sections 23(2A) and (2C) make proposing or relying on an unfair term a contravention, section 24(4) reverses the onus, section 25(k) names a term limiting one party's right to sue as an example, and penalties commenced 9 November 2023 reach $2,500,000 for an individual with each term a separate contravention.

The mitigation is therefore not contractual, and this is the finding that most shapes the engineering.
It is architectural and evidential: dry-run-by-default writes, per-field before-and-after audit logs, destructive operations absent rather than merely off, and a change history that proves which write was the seller's instruction.
Those controls were already prescribed for marketplace-terms reasons; section 267(4) makes them the primary legal defence.

The residual exposure is insurable and the insurance imposes three operating rules.
DUAL Australia's Information Technology Liability wording carries extension 3.4, agreeing to pay "all loss and defence costs arising from any claim for civil liability for unintentional contraventions of the Competition and Consumer Act 2010 (Cth), the Australian Consumer Law", where "unintentional" is load-bearing and the audit log and dry-run defaults are what keep a sync failure characterised that way.
Never admit a marketplace terms breach in any correspondence, because exclusion 8.13(b) triggers on an admission alone without any adjudication.
Promise no service-level agreement and no uptime credit, because exclusion 8.5 excludes assumed contractual obligations, and give sellers no broad indemnity for the same reason.
Disclose the automation model in full on the proposal form, since non-disclosure is a more reliable way to lose cover than any exclusion.
Indicative pricing is A$83 a month for IT liability, A$103 for professional indemnity and A$134 for cyber, but those are all-occupation portfolio averages and the underwriter's reaction to the disclosure is the real unknown.
A TPT "Site Assets" claim would be pleaded as intellectual property infringement, which the cyber policy excludes, so the IT Liability policy is the one that must respond.

On privacy, the Australian small-business exemption is real and nearly useless.
Privacy Act section 6D exempts businesses under $3,000,000 turnover, which would put a pre-revenue solo founder outside the Australian Privacy Principles and outside the Notifiable Data Breaches scheme, and that must not become the compliance posture.
It gives zero relief from UK and EU GDPR, which apply directly under Article 3(2) to "the offering of goods or services ... to such data subjects in the Union".
It gives zero relief from the statutory tort of serious invasion of privacy, which commenced 10 June 2025, applies regardless of turnover, is "actionable without proof of damage", caps non-economic and exemplary damages at $478,550, names "intruding upon the plaintiff's seclusion" as its first limb, and directs the court to "the means, including the use of any device or technology, used to invade the plaintiff's privacy".
And section 6D(4)(d) removes the exemption from an entity that "provides a benefit, service or advantage to collect personal information about another individual from anyone else", whose literal wording plausibly catches a paid service whose automation reads buyer-adjacent data from a marketplace page — an open question needing a privacy lawyer rather than more desk research.

## Two data-protection mechanics worth building for

GDPR Article 34(3)(a) removes the individual-notification duty where measures render the data unintelligible, which is the concrete return on per-tenant data-encryption keys, and it helps only for the stolen-dump case and not for a compromised running process ([GDPR Article 32](https://gdpr-info.eu/art-32-gdpr/)).
The incident runbook is written to GDPR's 72-hour clock rather than Australia's 30-day assessment window, because the stricter standard covers both.
Name Anthropic and the storage replica provider as subprocessors from day one and strip personal information from prompts, because OAIC guidance is that "the primary purpose for collection ... should be construed narrowly" and retrofitted notice cannot cure an undisclosed purpose.

## One EU question that is in force now

Under the EU AI Act, applicable from 2 August 2026 and reaching third-country providers "where the output produced by the AI system is used in the Union", Article 50(2)'s machine-readable marking duty attaches to providers rather than deployers, and whether wrapping a third-party model under the founder's own brand makes them a provider is unresolved.
Article 50(4) is disposed of twice over — listing copy is not public-interest information, and the seller-approves-before-publish step is an independent carve-out — which means that approval step now does legal work as well as quality work and must not be removed as friction.

## One regulation that helps

The Platform-to-Business Regulation gives the Tes cohort real recourse the TPT cohort does not have, and the United Kingdom retained it with "United Kingdom" substituted for "Union", so it covers the whole Tes base rather than a slice.
Article 4(1) requires a statement of reasons on a durable medium before or when a restriction takes effect, Article 4(2) requires thirty days' notice of termination, Article 4(3) requires an opportunity to clarify and reinstatement with data access on revocation, and Article 11 requires a free internal complaint-handling system.
Build this into the product: when a suspension or listing removal is detected for a UK or EU seller, surface the statutory entitlement, capture any statement of reasons, and pre-fill the internal complaint from the audit log.
That converts the per-field history from a liability shield into a customer benefit.
State the limit plainly in marketing — it does not cover United States sellers on TPT, which is the larger cohort with the harsher terms — so near-zero tolerance for product-attributable suspensions stands unchanged.

Article 9 of the same regulation obliges providers to state in their terms what access business users have to their own data, which establishes "the seller's own data" as a legally recognised category distinct from "marketplace data" and supports the read posture without authorising it.
Neither platform's Article 9 disclosure has been obtained, so it is support rather than authority, and asking both platforms for it in writing is free once Stage B lifts the hold on marketplace correspondence.
