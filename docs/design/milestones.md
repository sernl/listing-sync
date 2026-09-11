# Milestone plan

This plan supersedes all three earlier versions: the build order in the round-one feasibility report, the revised order in the round-two addendum, and the server-side revision that followed the founder's architecture correction.
It differs from all three because the wedge changed.
The first chargeable product is Tes GB-to-NZ inventory duplication, which is a bulk-tool job inside one marketplace, so the cross-marketplace layer moves out of the first shippable milestone and the TPT connector moves behind three others.

Sizes are founder-weeks: one person, full time, excluding legal review and customer discovery, which run in parallel.
Each milestone states what it proves and carries a kill gate written as an observable finding rather than as a feeling about progress.
The three things that can kill the product are front-loaded and every one of them is cheap: paying demand, the shape of the Tes upload path, and Tes's own position on tool-assisted authoring.

Total to first revenue from software is 11 to 15 founder-weeks.
Revenue from the commercial track starts in week one.

## The commercial track, from week one, no code

The founder is customer zero for the exact problem, and most of his own catalogue sits on Tes GB only because cross-listing by hand is not worth the time.
His own catalogue is therefore migration zero, and the payoff of the whole premise is directly measurable against it before any customer exists.

Beyond that, sell done-for-you catalogue migrations by hand, performed in the founder's own Tes session with nothing but a spreadsheet.
The addendum set the demand gate at fewer than 25 paid migrations in six months, and that number transfers, but its subject does not: the addendum's concierge job was cross-listing between Tes and TPT, and the job here is duplicating an existing GB catalogue into the US inventory.
Whether 25 buyers exist for that narrower job is untested, and it is the most important untested commercial question in the plan.
The migrations also generate the fixture corpus, the real taxonomy pairs and the uploader knowledge the adapter needs, which is why doing them by hand first is not wasted motion.

Two written enquiries go out in week one, both free, neither on the critical path.
The Etsy Personal App and then the Commercial Access request, with an application purpose that honestly names the multi-channel intent, because Commercial Access is a manual review with sole-discretion rejection and no published service level, and it must be in flight long before it is needed.
And the payment-processor pre-approval enquiry in writing before any billing code, because two obvious merchant-of-record candidates prohibit this product by name and the processor's own agreement provides a written pre-approval route.

Four further enquiries were drafted for week one and are deliberately held, because `decisions.md` states that no marketplace permission enquiries are being sent yet and that the Tes path is proved first.
Held are the duplicate-upload question to `authors@tes.com`, the four-question partnership pitch to `partnerships@tes.com` led with supply rather than with tooling, the TPT Publisher Membership enquiry, and the written request to both platforms for their Platform-to-Business data-access disclosure.
They move to the gate before the first external customer, which is the point at which the operator rather than the seller performs the act under both marketplaces' terms, and until then the founder is automating his own account.
The tension is real and is recorded rather than resolved: asking creates a written record and a refusal is harder to work around than silence, which is the reason to send early and the reason the decision record holds.

## M-1, the probes

One founder-week of work, several weeks of elapsed observation, and no production code.
These are experiments whose job is to make large decisions cheap.

| Probe | Method | What it gates |
|---|---|---|
| Tes upload request shape | devtools on the founder's own author account | browser fleet versus a plain HTTP client |
| Save-as-draft | the same session, looking for a draft state | whether ambiguous creates exist at all |
| Uploader vocabulary extraction | the authenticated upload form, GB and US | the size and the source of the taxonomy work |
| Resource deletion semantics | the founder's own dashboard | the Tes blast-radius analysis and the seller-facing deletion story |
| Session longevity | instrumented log over weeks on the founder's account | parking behaviour and whether sync is unattended |
| Tes take rate | the founder's own dashboard | the seller-facing economics argument |

The upload-shape probe is the highest-leverage hour in the entire plan.
The research found Tes serving bare Fastly with no bot-management vendor, no challenge platform and no fingerprinting script, which makes a plain multipart form POST plausible.
If it is one, `tam-browser` is deleted along with the per-session isolation design, the shared-memory work and the fifteen-to-twenty-five forced browser upgrades a year, and the dominant infrastructure cost line falls by roughly thirty times.
If it is a JavaScript-mediated chunked uploader, the browser is mandatory and every one of those costs is real.

The save-as-draft probe is the highest-leverage correctness question.
A draft create followed by a publish transition splits one dangerous write into a harmless one and an idempotent one, because publishing an already-published draft is a no-op, and that is a larger correctness win than the entire network-observation design it would replace.

The vocabulary probe matters more than its size suggests.
The measured figures of 2,450 US-only and 2,390 GB-only subject and topic paths came from public sitemap enumeration, which measures the catalogue's paths rather than the uploader's selectable option sets, and the adapter has to satisfy the second.
The same probe yields the GB phase and US grade labels whose age boundaries the grade model stores as data, and which are not established anywhere in the research and must never be written into code from memory.

Egress reachability is not on this list because it is largely settled and the remaining question is conditional.
A probe on 2026-08-25 found TPT returning HTTP 200 to a plain `curl` from the founder's consumer-ISP host, while earlier research measured HTTP 403 from AI-vendor datacentre infrastructure, and TPT's `robots.txt` names `GPTBot`, `meta-externalagent`, `CCBot`, `ImagesiftBot` and `Applebot-Extended` in explicit stanzas, so the 403 was an AI-vendor block rather than a generic datacentre block.
The conditional is that if the eventual production box sits in a commercial datacentre, reachability must be re-probed before it is relied on.
Egress is fixed and declared in either case.

Kill gate: Tes adds an express anti-automation clause to the Additional Terms, or, once the held enquiries are sent at the Stage B gate, replies with a written objection naming third-party automation.
Round one wrote the first half conjunctively, requiring the clause and enforcement of the fair-usage upload limit against the product's traffic pattern; this plan splits the conjunction deliberately, because the clause alone removes the drafting-gap defence and waiting for enforcement means waiting for a customer's store to be restricted.
Tes is the only connector in the wedge, so either finding stops the build outright rather than re-sequencing it.
Nothing else at M-1 kills the product; the rest re-prices it, and saying so is more useful than manufacturing a gate.

## M0, the spike

Two to three founder-weeks.
One Tes listing, created from one file, by the server, on the founder's own author account, verified by read-back and field diff.
Unchanged in substance from the plan that preceded it.

Five things must all go green and any one of them failing stops the milestone.

A session established on the server from the seller's stored credentials completes an authenticated upload.
The session courier is out of M0 scope: this milestone tests the stored-credential model only, and the courier is evaluated only if the first kill gate below fires.
The driver pushes a real file into the form.
The submission survives Tes's up-to-three-working-day moderation gate and reaches the live state.
The resulting resource URL binds to a durable record as an external identity.
And a deliberately interrupted write is classified as ambiguous rather than as success or as failure.

The last of those is the one that is new relative to every earlier version of this milestone, and it is non-negotiable.
The ambiguous path is the one that will be exercised in production and never in development, so the transport-layer fault-injection seam is built here rather than later: HTTP 429, HTTP 403, an HTML interstitial, a truncated body, a connection reset mid-body and a redirect to the sign-in page are all exercised in continuous integration from this milestone on.
Two of those parse as success unless every post-authentication step asserts positively on an element only an authenticated page carries, which is why the assertion is positive rather than an absence check.

Kill gate, first form: a server-established session is challenged or invalidated on every attempt.
That falsifies the current stored-credential model and reopens the custody question before anything is built on top of it, and it is the event that puts the session-courier variant on the table.

Kill gate, second form: an interrupted write cannot be made to classify as ambiguous, and settles as success or failure instead.
The correctness discipline the entire product rests on is then unimplementable against this marketplace, and no amount of further engineering changes that.

## M1, the first chargeable milestone

Tes GB-to-NZ inventory duplication, 8 to 11 founder-weeks.

The GB-to-NZ duplication this milestone was named for was deleted on 2026-09-12, when the founder ruled that Tes is one marketplace with no regions; see `decisions.md`, "Tes is one marketplace with no regions, 2026-09-12".

The size is a judgement rather than a carried figure and the arithmetic is worth showing.
The server-side plan priced catalogue plus bulk create on Tes at six to eight weeks once the server owns custody, isolation and correctness, and priced the GB-to-NZ duplication feature at three to four weeks on top.
Combining them saves one to two weeks of overlap, because the catalogue is only ever built once and the mapping engine is built once, which gives eight to eleven.
It is larger than the addendum's five to seven for the same feature because that estimate assumed a browser extension running on the seller's machine and no credential custody at all, and the founder's architecture reinstates both.

What ships, and all of it is required for the milestone to be chargeable.

The catalogue with per-organisation tenancy from the first migration, forced row-level security, and the two-tenant negative test.
The file pipeline: streamed ZIP inspection with decompression-bomb and symlink guards, PDF and PPTX probing, cover and preview generation, malware scanning, and content-hash deduplication within the tenant.
The canonical taxonomy with GB and US projections, the reconciliation queue, and grade declarations carrying the seller's own wording as provenance.
The job ledger with per-item leases and fencing, the write-attempt record as the fencing token, read-back verification behind the fetch-reason capability, and the per-field audit log shipped continuously off-box.
The session broker and the automation worker as separate units, per-connection envelope encryption, and a tested key escrow and recovery procedure, because hardware-bound key material means the first motherboard failure destroys every stored session unless the escrow exists first.
Server-controlled per-tenant rate governance, the durable circuit breaker with a half-open probe and a maximum hold, and the per-tenant mutex that stops two workers racing session-bound form tokens and manufacturing sporadic failures that look exactly like bot detection.
The hourly read-only structural canary and the pre-flight form-schema assertion, which fails closed before any write for one request and catches an overnight redeploy that a post-write diff only catches after several listings are already corrupted.
The web client, whose whole job around sync is detailed progress reporting: a virtualised product-by-inventory table, per-inventory stacked outcome bars with the raw counts beside them, a per-item step timeline on row expand, a downloadable per-item result file, a connections page with seller-initiated revoke and delete, and distinct visual states for blocked-on-seller rather than a bar that stops moving.
The per-marketplace status page, shipped here rather than later, because a customer who can see that Tes create is degraded with an estimated fix time does not open a ticket, and support labour is the largest cost line at low customer counts.
Charging by manual Stripe payment link, after the pre-approval reply lands; automated billing does not need to exist until roughly fifty customers.

What it proves is threefold.
That a bulk job of realistic size completes with per-item honesty rather than a green bar, which is the only claim a seller can verify.
That the taxonomy projection covers a real catalogue, measured as the share of terms reaching the reconciliation queue on first run.
And that somebody other than the founder pays for it.

Four kill gates apply, three carried and one proposed here.

More than roughly one in two hundred connected seller stores experiences a suspension, payout hold or listing removal attributable to the product during beta.
The tolerance is near zero rather than merely low, because the seller's store is their livelihood and Tes routes its own indemnity onto exactly the target cohort of VAT-registered authors and authors paid more than £10,000 in twelve months.

Selector fallback share exceeds 20% of steps on the Tes adapter, or unplanned selector changes exceed two per month for three consecutive months.
Fallback share is a leading indicator available in week one, which is what makes this gate evaluable long before the treadmill has consumed a quarter.

Adapter maintenance plus correctness plumbing consumes more than about a third of engineering time by the end of the milestone.

And the proposed one, which is mine rather than the research's: the reconciliation queue does not drain.
If the share of canonical terms raising a new reconciliation item does not fall materially between the first customer catalogue and the tenth, the canonical taxonomy is not converging, the hub is a treadmill rather than an asset, and the N-projection argument in the domain model is wrong in practice whatever it is in principle.
This is measurable from the first ten migrations and should be instrumented from the first one.

## The rest of the plan

| Milestone | Weeks | Proves | Kill gate |
|---|---|---|---|
| M2 analytics from seller-owned data | 2 | the renewal mechanism exists | none |
| M3 Etsy connector, cloud plane only | 3–4 | the model survives a sanctioned API | Commercial Access refused |
| M4 AI listing copy | 2–3 | copy is table stakes, not the product | none |
| M5 billing, tiers, self-serve signup | 2–3 | the business runs without the founder | none |
| M6 drift reconciliation via first-party exports | 2 | divergence is visible without enumeration | none |
| M7 TPT connector, gated and premium | 5–7 | nothing the product depends on | any written objection from TPT or IXL |

M2 restores the analytics surface that round one deleted on legal grounds without re-running the business model, and it is built strictly from seller-owned data: the Tes first-party export and authenticated dashboard endpoints, and seller-uploaded export files.
Analytics is the documented upgrade driver in both close comparables, so deleting it removes the renewal mechanism while leaving the churn, and it should not slip behind a connector.

M3 cannot start before Commercial Access lands, which is why the application is filed in week one and never placed on the critical path.
It is a cloud-plane connector with no browser anywhere in its tree, and it carries per-listing cost projection in the publish flow so the seller sees the per-listing fee before committing, plus token expiry as a first-class product state rather than an error.
The firewall between it and the browser-driven connectors is an architectural invariant rather than a preference, given Etsy's prohibition on using or promoting automated systems or browser extensions against Etsy's own site, API or data.

M4 is descoped from a paid differentiator to table stakes, because an existing app already sells AI title and description generation to this persona as a one-time purchase.
Three engineering constraints stand unchanged: length is a deterministic Rust predicate rather than a model's judgement, numeric content claims are verified against a fact sheet extracted from the actual file with a failed verification as a hard publish block, and every output is a proposal with a field-level diff that the seller approves.

M7 is last among the connectors, and not because it is least valuable.
It is last because it is the only connector whose schedule someone else owns, and because the decision record requires every screen to be complete and worth paying for without it.
Absent written permission, TPT's automated-means clause reaches the read-back verification this product's correctness depends on, so a TPT connector built without permission is one that either breaks its own correctness discipline or breaches the guidelines, and there is no third option.
If it proceeds it carries the four measurements the earlier plans specified: session longevity before a one-time-password re-challenge, whether bot management challenges the declared egress, the real mandatory-field set and any velocity check, and the two undocumented bounds of preview-file size cap and minimum price.
It also carries the cross-marketplace layer for the first time: the price-parity guard as a first-class invariant, and the deterministic per-marketplace link-scrub pass, which is a correctness requirement rather than a copy-editing nicety because both marketplaces ban links to competing sales channels in listing copy.

## Deferred, and why

| Deferred | Until | Reason |
|---|---|---|
| Node-graph mapping canvas | indefinitely | a licence-and-scale problem and unusable on a phone; every incumbent ships a table |
| Public versioned developer API | a real third-party consumer exists | nothing to design against, and none of the obvious exemplars runs concurrent version trees |
| Desktop clients | indefinitely | the server-side architecture removes the need; signing and native CI are a recurring cost |
| Browser extension client | indefinitely | superseded by the server-side decision |
| Fleet pack signing and update-framework machinery | a client interpreter fleet exists | there is no fleet; the interpreter runs on the founder's own host |
| Bazel or Buck2 | never, on current evidence | the Nix integration forfeits remote execution, which is the only reason to adopt it |
| Browser pool beyond one or two sessions | measured demand | marketplace pacing binds an order of magnitude below the hardware ceiling |
| Key-management server on the same host | a second machine or a hosted service exists | a single box cannot auto-unseal without defeating the point |
| Software escrow | roughly 200 paying customers | the monthly cost is several customers; carry the obligation contractually until then |
| Agencies as a Tes segment | the one-account-per-author process is answered in writing | the Author Code permits one account per author absent written permission |
| iPad and Safari extension | indefinitely | distribution requires a containing app, a developer programme and store review |

Two of those deferrals cost something real and the cost should be named rather than buried.

Dropping the browser extension in favour of server-side automation gives up two arguments the addendum made in its favour: that a warm, already-trusted browser profile would reduce one-time-password re-challenge rates, and that residential-IP diversity is itself an incriminating signal to a network-wide bot-reputation system while stock Chrome carries the most common fingerprint on earth.
Both were design hypotheses rather than measured findings, and both are now moot, but the second cuts the other way under server-side automation: every tenant's traffic carries the same binary's fingerprint, so a fingerprint-keyed detection is a single fleet-wide event by construction.
The compensating control is not technical.
It is keeping per-tenant volumes low and write correctness high enough that no detection is triggered, which is why rate governance and the canary ship in the first chargeable milestone rather than late.

Deferring the public API is only defensible if the internal one is built properly now, which costs almost nothing: a nested versioned router, an OpenAPI document, idempotency keys on every request that starts a sync, long-running operations modelled as resources rather than as blocking calls, and cursor pagination.
What is deferred is publishing, metering, per-tier rate limiting and SDK generation, none of which gets harder by waiting.
