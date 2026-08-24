# Multi-marketplace listing sync for teacher-authors: round two addendum

Prepared 2026-08-25 from seven research lanes closing gaps a completeness critic found in the report of 2026-08-24.
One lane, the competitor check, was adversarially verified twice.
This document extends that report and does not restate it; read it first.
Where round two overturns a round-one conclusion, the round-one text is quoted and the correction sits beside it.

## 1. What round two changes

Five round-one conclusions are overturned, one is confirmed but reclassified, and one internal contradiction is resolved.
Nothing in round two rescues the product's economics; two findings improve its legal position materially, and two make the market smaller than round one believed.

### The connector selection is overturned

Round one concluded: "neither TeachersPayTeachers nor Tes imposes exclusivity, TPT's Content Guidelines explicitly permit selling the same resource elsewhere, neither marketplace offers any bulk path for *creating* listings, and no product exists today that cross-lists between them — so the pain is real, unserved, and legal to address."

Each clause of that is still true, but round one never asked whether some third destination has a sanctioned write path, and one does.
Etsy's Open API v3 exposes the complete create-listing flow for a digital download: `createDraftListing` with required quantity, title, description, price, who_made, when_made and taxonomy_id; `uploadListingFile`, documented as "Uploads a new file for a digital listing" and annotated "This endpoint is ready for production use"; `uploadListingImage`; `getSellerTaxonomyNodes`; and a listing `type` enum of physical, download or both ([Etsy OpenAPI 3.0.2 spec](https://www.etsy.com/openapi/generated/oas/3.0.0.json)).
Etsy's API Terms section 1 then grants the exact licence TPT and Tes both withhold: "a limited, non-sublicensable, non-transferable, non-exclusive and revocable license to access and use the Etsy API solely to develop, create, share and run Applications", where an Application is "any software, website, integration or other tool that accesses or utilizes the Etsy API and which you make available to Etsy sellers" ([API Terms, updated 16 June 2025, snapshot 1 August 2026](http://web.archive.org/web/20260801203021/https://www.etsy.com/legal/api)).
Section 3 goes further and assigns the seller relationship to the vendor outright: "You, and not Etsy, are (i) the sole person or entity providing the Application to Application users; and (ii) the sole person or entity responsible for invoicing and collecting any fees."
Etsy's FY2025 Form 10-K reports 5.6 million active sellers and 86.5 million active buyers against a TPT base of roughly 185,000 sellers who made any sale ([SEC filing, 19 February 2026](https://www.sec.gov/Archives/edgar/data/1370637/000137063726000019/etsy-20251231.htm)).

The consequence is that the entire section-2 legal analysis — the Site Assets clause, the identifier-disguise clause, the CFAA extraterritoriality question — simply does not arise on an Etsy connector, and an Etsy connector needs no browser client at all.
Round one's build order should be revised, though not to Etsy-first outright; section 3 gives the reasoning and the verdict.

### The desktop-agent recommendation is overturned

Round one concluded: "The Tauri route is the strongest fit and is detailed in section 3", and then deferred Windows and macOS distribution indefinitely.

That combination ships to almost nobody.
Windows is 55.95% of United States and 65.88% of United Kingdom consumer desktop traffic, with combined Apple share around 30% and 24% respectively, and Statcounter's Linux row is widely understood to be inflated by unclassified and automated traffic ([Statcounter, July 2026](https://gs.statcounter.com/os-market-share/desktop/united-states-of-america)).
Signed Tauri distribution to Windows and macOS is a verified recurring cost of roughly $970–1,145 per year, or $1,250–1,420 once the EV certificate that SmartScreen effectively forces is included, plus a Mac the founder must own to debug WKWebView adapter failures.
A Manifest V3 Chrome extension covers Windows, macOS, Linux and ChromeOS with one artifact for a single one-time Chrome Web Store registration fee, and it collapses round one's tripled webview QA matrix to one Chromium engine.
Section 2 settles this and prices it.

### The revenue ceiling is overturned, and the denominator was wrong

Round one concluded: "Realistic ceiling on current evidence is a bootstrapped lifestyle business of roughly $200k–250k ARR at a thousand paying customers, against a marketplace where only about 185,000 sellers made any sale in a trailing year."

Both of round one's uncited numbers are real but trace to a single EdSurge interview with then-TPT-CEO Joe Holland published 28 November 2022, which is company-stated, unaudited, 3.75 years stale, and predates the IXL Learning acquisition; TPT has published no figure since ([EdSurge](https://www.edsurge.com/news/2022-11-28-why-did-we-stop-hearing-about-the-teachers-making-millions-on-teachers-pay-teachers)).
More importantly the denominator is wrong.
The addressable market is not all TPT sellers, it is the TPT-and-Tes intersection, and that was measured directly: of 474 TPT store slugs harvested from TPT's own search, 16 (3.38%) have a name-identical Tes shop and only 10 (2.11%) hold any live Tes inventory, implying roughly 3,900–6,200 dual-listers platform-wide.
An independent measurement of the Tes side triangulates the same answer: Tes's unfiltered catalogue returns 1,039,401 resources against 23,266 with `orientations=American` (2.24%), held by an estimated 2,000–5,000 distinct shops ([Tes search](https://www.tes.com/resources/search/?q=&orientations=American)).
At a pool of roughly 4,000, a thousand subscribers is 20–26% of the entire market, which is a market-share assumption no bootstrapped solo product should underwrite.
Section 7 rebuilds the ceiling at 150–650 subscribers and $60–220k ARR.

Round one's incidental figure "907 resources tagged American" is also corrected to 23,266.

### The payments assumption is overturned twice

Round one assumed: "Stripe takes 2.9% + $0.30, which on $19 is $0.85."

That is a United States figure.
Stripe's Australian pricing is 1.7% + A$0.30 domestic, 3.5% + A$0.30 international, plus 2% where currency conversion is required, and Managed Payments adds a further 3.5% ([Stripe AU pricing](https://stripe.com/au/pricing)); New Zealand is 2.65% + NZ$0.30 domestic and 3.5% + NZ$0.30 international with the same 2% conversion charge ([Stripe pricing](https://stripe.com/pricing)).
United States and United Kingdom teacher-authors are international cards under either domicile, so the true cost at $19 is about $1.24 direct or $1.91 through Managed Payments, roughly 46% higher than assumed.

The second correction is more consequential than the first.
Round one's cost model treated the merchant-of-record choice as a fee comparison; it is an acceptance question, and two of the obvious candidates prohibit this product by name.
Section 5 sets that out.

### The write-mostly stance is overturned as internally contradictory

Round one stated: "Marketplace state is treated as write-mostly. The platform is the system of record; the product pushes state and records what was written, rather than re-reading TPT to discover it."
Two paragraphs earlier it mandated: "After every publish, re-fetch the created listing by its durable identifier and diff the rendered title, price, description and taxonomy against the canonical record before marking the sync green."

The critic was right that these cannot both hold, and round one picked the wrong side.
Its own reasoning defeats it: an unverified write is an unlogged corruption of the customer's store, and TPT's VA terms make the seller liable "as if those actions were taken by you directly" while giving them no detailed change history.
The resolution is to change the operative rule from "never read TPT" to "never enumerate", and section 6 gives the type-level enforcement.
TPT itself ships a Product Statistics CSV export, a Sales Details download and a Privacy Center access-request flow, which demonstrates the platform distinguishes a seller obtaining their own data from a third party extracting marketplace data ([TPT help article 360042733831](https://help.teacherspayteachers.com/hc/en-us/articles/360042733831)).

### Confirmed but reclassified: no competitor exists

Round one's least-tested claim survives, tested hard.
It should nonetheless stop being read as an opportunity signal.
Section 4 states the coverage and the defects.

### Not overturned

The section-2 legal analysis of TPT and Tes stands unamended; no round-two lane found anything that softens the Site Assets clause, the identifier-disguise clause, or the Ryanair extraterritoriality holding.
Round one's stack decisions in section 4 are untouched except where the client change in section 2 removes the Windows and macOS pipeline entirely.
The Bazel answer in section 5 stands.

## 2. The client question, resolved

Ship a Manifest V3 browser extension as the primary and only client for the browser-driven connectors.
Demote Tauri to a local prototyping harness for the founder's own machine and a contingency if Google rejects the listing.
Ship no desktop binary in year one.

### The price of each route

| Route | Recurring annual cost | Platforms reached |
|---|---|---|
| Tauri, Linux only | Near zero | About 2% of the persona |
| Tauri, signed Windows and macOS | $970–1,145, or $1,250–1,420 with EV | Windows, macOS, Linux |
| Manifest V3 extension | One one-time Web Store registration fee | Windows, macOS, Linux, ChromeOS |

The Tauri figures decompose as follows.
Apple's Developer Program is "$99 USD per membership year" and is the only tier carrying "Notarization & Developer ID for Mac apps"; Tauri's own guide adds that "You also need an Apple device where you perform the code signing" ([Tauri macOS signing](https://v2.tauri.app/distribute/sign/macos/)).
DigiCert lists Organization Validated code signing at $696.00 per year and Extended Validation at $972.00, and since 1 June 2023 the CA/Browser Forum requires private keys be generated and stored in a hardware crypto module, so the key physically cannot be copied into a CI secret ([DigiCert](https://www.digicert.com/signing/code-signing-certificates)).
EV is not a vanity purchase: Microsoft states SmartScreen "checks downloaded programs and the digital signature used to sign a file" and that items without established reputation are "marked as a higher risk", and Tauri's own guide says an EV certificate "will receive an immediate reputation with Microsoft SmartScreen" whereas OV "will still show a warning to users" ([Microsoft](https://learn.microsoft.com/en-us/windows/security/operating-system-security/virus-and-threat-protection/microsoft-defender-smartscreen/)).
For a paid product sold to non-technical teachers, "Windows protected your PC" landing on the onboarding step is a conversion catastrophe.
Paid GitHub Actions runners add roughly $175–350 per year at a realistic cadence, dominated by macOS at $0.062 per minute against Windows at $0.010 ([GitHub billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)).
A Mac for interactive WKWebView debugging is a further $500–700 one-off.

There is also a legal-structure trap hiding in the cheap Windows path.
Microsoft's Artifact Signing states plainly that "Individual developers must be located in the United States or Canada", and organization identity validation "takes from 1 to 20 business days" ([Microsoft Learn](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart)).
A sole trader outside North America would have to incorporate before reaching the cheap signing route, which means round one's client choice silently forces a company-formation decision.
The extension makes that question disappear.

### Why the money is the smaller half of the argument

Round one identified adapter maintenance as the permanent recurring cost line and then tripled it: "The webview engine differs per platform — WebView2 (Chromium, evergreen) on Windows, WKWebView capped by OS version on macOS, WebKitGTK on Linux ... so the adapter QA matrix triples."
An extension runs on one engine everywhere, and the same package is accepted by Edge Add-ons.
That is a larger saving than every certificate combined.

The extension also runs in the seller's already-warm, already-trusted Chrome profile rather than a fresh Tauri `data_directory` that looks like a brand-new device on every install, which should reduce TPT's email-OTP re-challenge rate — round one's own M1 gate.
This is a design hypothesis, not a measured finding, and it is the highest-value thing to test alongside the file-injection spike.

There is a further detection argument that inverts round one's open question 9.
Cloudflare aggregates bot intelligence per JA4 TLS fingerprint across its whole network, publishing to Bot Management customers "The number of networks Cloudflare sees actively using this fingerprint", "The number of Cloudflare sites that see traffic from this fingerprint", and a `heuristic_ratio_1h` field giving "The ratio of requests with a scoreSrc value of 'heuristics' for the JA4 fingerprint in the last hour" ([Cloudflare Signals Intelligence](https://developers.cloudflare.com/bots/additional-configurations/ja3-ja4-fingerprint/signals-intelligence/)).
One rare fingerprint seen across hundreds of residential IPs on many networks hitting a narrow path set is the signature of a botnet, and the residential-IP diversity round one treated as protection is itself the incriminating signal.
WebKitGTK on Linux carries the rarest fingerprint of the three; Chrome carries the most common one on earth.

Finally, this is what the analogous industry actually ships.
Crosslist states it "uses a secure browser extension that runs inside your own browser so actions happen directly on your computer"; Vendoo's Chrome listing shows 60,000 users ([Crosslist](https://www.crosslist.com/)).
No incumbent in the category ships a desktop binary.

### What it costs the Rust story

Very little, and less than round one's own Leptos rejection cost.
The cloud plane is untouched: `tam-api`, `tam-storage`, `tam-pipeline`, `tam-ai`, the job ledger and billing all stay Rust exactly as specified.
Only the manifest, the service worker, the offscreen document, the content scripts and the step interpreter become TypeScript — on the order of two thousand lines, and there is no Rust worth having for the `chrome.*` surface.

For shared domain logic there are two routes and a threshold.
WebAssembly is permitted on extension pages but must be declared, because the default content security policy is `script-src 'self'; object-src 'self';` under which WebAssembly is disabled, and Chrome's enforced minimum adds exactly `'wasm-unsafe-eval'` with no further relaxation possible ([Chrome CSP reference](https://developer.chrome.com/docs/extensions/reference/manifest/content-security-policy)).
If WebAssembly is used, compile `tam-types` and `tam-domain` twice, keep the core synchronous and pure with no I/O so it runs identically server-side, make calls coarse-grained because every crossing is a serde marshalling cost, install `console_error_panic_hook` so a panic is not an opaque "unreachable", and run it in the offscreen document rather than the content script.
If the genuinely shared surface is under roughly 1,500 lines, skip WebAssembly entirely: generate TypeScript types from `tam-types` with ts-rs, reimplement the small logic in TypeScript, and pin the two implementations together with a shared golden-vector corpus.
That is cheaper than a `wasm-bindgen-cli` exact-version pin in the Nix build, which is precisely the cost round one cited when rejecting Leptos.

### Chromebook and iPad

Chromebooks are covered at zero marginal cost, which is the correct way to serve them, because they do not justify a decision on their own.
The classroom-plurality figure is real but is about school-purchased fleets and is 8–10 years old — Futuresource put Chromebooks at 58% of the 2.6 million mobile devices United States schools purchased in 2016 and near 60% of computers in use by March 2018 — while ChromeOS is 2.08% of United States and 3.77% of United Kingdom consumer desktop traffic, and a teacher-author authors at home.
The United Kingdom Department for Education's Technology in Schools Survey 2022-23 corroborates the Windows-first ordering for the Tes cohort: any-Windows at 95% of primary and 98% of secondary IT estates, ChromeOS at 34% and 26%, any-Mac at 6% and 28% ([DfE, 28 November 2023](https://assets.publishing.service.gov.uk/media/655f8b823d7741000d420114/Technology_in_schools_survey__2022_to_2023.pdf)).
Crostini is not a distribution channel and should be struck: Linux is off by default, unavailable on managed school devices, and setup "can take 10 minutes or more" ([Google support](https://support.google.com/chromebook/answer/9145439)).

iPad gets neither client and should be scoped out in writing.
A Safari web extension "must be distributed inside a containing iOS, iPadOS, or macOS app, and that app must be submitted through the App Store", which re-imports the $99 Apple Developer Program plus Xcode plus review ([Apple](https://developer.apple.com/documentation/safariservices/safari-web-extensions)).
State in the product requirements that authoring and syncing require desktop Chrome or Edge, and give phone and tablet a responsive read-only status and approval view — which is what round one already scoped as the mobile story.

### The three conditions that make this work

The remotely-hosted-code ban is the one serious objection and it has a verified escape hatch.
Chrome's migration guide says "In Manifest V3, all of your extension's logic must be part of the extension package" and that you "can no longer execute external logic using executeScript(), eval(), and new Function()", but it expressly permits the alternative: "Your extension loads and caches a remote configuration (for example a JSON file) at runtime" ([Chrome](https://developer.chrome.com/docs/extensions/develop/migrate/improve-security)).
So ship a fixed, deliberately non-Turing-complete step interpreter in the package and push every selector and step graph as JSON data.
Review latency makes this mandatory rather than merely elegant: review "can take up to a few weeks", with extra scrutiny for new developers, new extensions and broad host permissions ([Chrome Web Store review](https://developer.chrome.com/docs/webstore/review-process)).

Request narrow host permissions on `teacherspayteachers.com`, `tes.com` and the product's own API only, never `<all_urls>`, never the `cookies` permission, and ship unminified reviewable code because "Obfuscation is disallowed."
The manifest then becomes a machine-checkable, externally reviewed firewall, which matters for section 3.

Service-worker lifetime is not a real constraint if the architecture is right.
The worker shuts down "After 30 seconds of inactivity" but messages from an offscreen document reset the timer, and offscreen documents themselves have no lifetime limit except under the audio-playback reason ([Chrome service worker lifecycle](https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle)).
Put the job orchestrator in a single offscreen document, do the marketplace interaction in a content script in a visible tab, never put the sync state machine in the service worker, and drive adapters with MutationObserver rather than chained timers, because intensive throttling checks timers "once per minute" for pages hidden more than five minutes.

The one capability genuinely lost is local filesystem access, which removes "watch a folder and auto-import" — no loss, since the cloud plane already holds the canonical files and the import path is a picker or drag-and-drop, which is what teachers do anyway.

That leaves the single load-bearing unverified assumption of the entire product: whether a content script can place a fetched Blob into TPT's and Tes's file inputs via a `DataTransfer` and have the React uploader and Cloudflare accept a change event whose `isTrusted` is false.
This is unverified.
It fails identically for Tauri, so testing it costs nothing either way, and it must be the first thing built.

## 3. The connector question, resolved

Yes, an API-backed connector exists that materially de-risks the business, and the wedge should change — but not to Etsy-first, and not away from Tes.

### The verdict

File the Etsy Personal App and Commercial Access request in week one, before any architecture is frozen, and build the Tes GB-to-United-States duplication as the first chargeable software.
Etsy becomes connector two when approval lands.
TPT is demoted to a gated, premium, explicitly optional third connector, and every screen must be complete and worth paying for without it.
Gumroad is connector four, subject to verification.

### Why Etsy is not connector one despite being the strongest connector

The case for Etsy-first is genuinely strong and was argued forcefully by the sanctioned-API lane.
Etsy is the only Phase-1-plausible destination with a written licence to build on it; its field validation is published and machine-checkable rather than discovered by rejection, including a title regex that rejects the emoji TPT titles routinely carry and a once-each budget on `%`, `:`, `&` and `+`; and its connector lives entirely in the cloud plane, needing no extension, no session-longevity study and no Cloudflare.
That last point means the first shippable product could exist before any browser-client risk is retired at all.
Vela is a live existence proof that Etsy tolerates a commercial multi-channel tool that also publishes to Etsy's direct competitors, at 100,000 sellers and five years of operation ([Vela pricing](https://getvela.com/pricing)).

Three facts hold it back from first position.
Etsy Commercial Access is a manual gate with sole-discretion rejection and no published SLA — "Commercial Access requests are reviewed manually. Review time may vary depending on your proposed use case", and "Etsy may reject a request for Etsy API access for any reason, in our sole discretion" ([Etsy developer docs](https://developers.etsy.com/documentation/)).
Putting an unbounded external approval on the critical path of the first shippable milestone is the wrong shape for a solo founder.
The obvious workaround is foreclosed: "Seller App Access is for your own shop use only", and section 5 of the API Terms prohibits transferring or commercialising API credentials, so do not build bring-your-own-key.
Second, whether the founder's own Tes and TPT audience wants to be on Etsy at all is untested with a single customer, and the entire Etsy-first argument rests on that cross-sell willingness.
Third, Vela already owns the generic Etsy multi-channel niche.

Meanwhile Tes GB-to-United-States needs one connector, no intersection constraint, no bot-management vendor, no cross-marketplace mapping, and the founder is already a Tes author who can test it on their own account this month.
Round one identified it as "probably the sharpest and lowest-risk first product" and then scheduled it fourth; round two agrees with round one's judgement and disagrees with round one's ordering.
The measured pool supports it: 23,266 American-oriented resources in a 1,039,401-resource catalogue means the opportunity is bounded by Tes's whole author base rather than by an intersection.

If the file-injection spike in section 2 fails, this ordering inverts immediately: cut the browser client entirely and Etsy becomes connector one and the only connector.
That contingency is what makes filing the Etsy application in week one non-negotiable rather than merely prudent.

### What the Etsy connector costs the product

Etsy sellers pay "a fixed listing fee of $0.20 for each item listed", recognised over four months and nonrefundable, plus a 6.5% transaction fee.
Pushing a 500-product catalogue costs the seller $100 immediately and roughly $300 a year in renewals.
The publish flow therefore cannot be a single button; it needs per-listing cost projection before commit, selective publish, and a renewal-decision surface driven by per-listing performance.
That is extra work and it is also a durable value proposition Vela does not frame.

Operationally, Etsy access tokens expire in 3,600 seconds and refresh tokens have a 90-day functional lifetime, so a seller who does not open the app for 90 days silently disconnects and token expiry must be a first-class product state rather than an error ([Etsy authentication](https://developers.etsy.com/documentation/essentials/authentication/)).
The API Terms' display-of-data clause forbids showing listing content "more than six (6) hours older than the corresponding information on the Etsy Site", which forces a background refresh cadence — this is the one place the product must read, and Etsy explicitly permits it.
Batch writes under 500 listings with backoff and expect multi-hour syncs on large catalogues; Vela's own support material reports that "If marketplace rate limits are triggered, the sync process can take up to 24 hours."
The Enterprise threshold of three million calls per day is far above anything a product at this scale needs, so rate limits are a pacing problem, not a ceiling.

### The clause that shapes positioning, and the firewall it demands

Etsy's Prohibited Behavior list includes "Divert sales or migrate Etsy Members from Etsy, or drive traffic to external websites or services unrelated to the Etsy platform", and separately "Use or promote the use of automated systems or browser extensions to access, analyze, or scrape the Etsy Site, the Etsy API or any Etsy data ... unless expressly authorized in writing by Etsy"; the Commercial Access criteria add "Screen-scraping is not allowed."

Three rules follow.
Market and build the Etsy connector as inbound — publish your catalogue to Etsy — never as "export your Etsy shop elsewhere".
Answer the fee clause in the application: the product charges for cross-channel catalogue management, not for a passthrough of a free Etsy feature.
And make it architecturally impossible for the browser client ever to touch Etsy.

That last rule is where the client decision in section 2 helps rather than hurts, and this is the one place where combining two lanes' recommendations creates a tension worth naming.
Etsy's prohibition names browser extensions specifically, so shipping one alongside an Etsy API integration looks worse on its face than shipping a desktop app.
But the clause is scoped to Etsy's own site, API and data, and an extension manifest that declares host permissions for `teacherspayteachers.com` and `tes.com` only is a declared, Google-reviewed, machine-checkable proof of separation that a Tauri binary — which can navigate anywhere — cannot offer.
Put the separation in the engineering charter as an invariant, put the manifest in the Commercial Access application as evidence, and disclose the combination rather than let Etsy discover it.
Whether Etsy's reviewers accept that reasoning is unverified and is the single highest-value unknown on this path.

### What Etsy does not change

Etsy is not a substitute for the teacher marketplaces.
The founder's pain, credibility and distribution are on Tes and TPT.
Teacher-authors do already sell there — an archived Etsy search for "lesson plans" on 24 August 2026 returned ten results, every one labelled "Digital download", from ten distinct shops, at a $13.00 median across 27 observed price points against TPT's $3–8 norms ([archived search](http://web.archive.org/web/20260824110728/https://www.etsy.com/search?q=lesson%20plans)).
Ten results is a thin sample and the median should not go into marketing copy until it is validated against a proper sample through the API's own search endpoints after approval.
Etsy's "Teacher Resources" market page shows a display-capped "5,000+ items", which is a floor and not a size.

### The rest of the destination set

Vela is the incumbent on generic mechanics and the proof of where the wedge is.
Its own bulk-edit documentation enumerates the complete editable field set — "Photos & Videos, Title & Description, Tags & Materials, About & Production Partner, Category & Section, Personalization, Optional Attributes, Variations, Price, Quantity/Inventory, SKUs, Shipping Details" — and the word "digital" does not appear in it; its CSV import supports creation but requires images "as direct links from file hosting services" with no digital-payload field and no from-disk upload ([Vela help](https://help.getvela.com/en/articles/10625133-what-can-i-bulk-edit)).
Do not out-build Vela on catalogue mechanics.
Build the two things it structurally lacks: the digital-file payload pipeline and education-domain semantics, which are the same components round one already scoped for M2.

Gumroad's v2 API is now a full write path — `resources :links, path: "products"` with create, a presigned multipart file route with `MAX_FILE_SIZE_GB = 20`, thumbnail and cover resources, and non-expiring OAuth tokens — read from the open-source main branch, with live unauthenticated probes returning 401 rather than 404 ([routes.rb](https://raw.githubusercontent.com/antiwork/gumroad/main/config/routes.rb)).
This is read from a branch, not a pinned release, so verify against the live API with a real token before scoping.

Payhip, Ko-fi, Podia, Sellfy and Teachable are ruled out on capability: Payhip's own reference states "At the moment we only have support for interacting with the Coupon and License Key resources"; Ko-fi offers outbound webhooks only; Sellfy's API page 404s; `developers.teachable.com` returns a routing error ([Payhip](https://payhip.com/api-reference)).
Shopify and Thinkific are ruled out on value: they are the seller's own storefront with no demand side, so syncing to them delivers zero incremental discovery, which is the whole reason a seller tolerates a marketplace take rate.
Boom Learning is ruled out because Boom Cards are authored inside Boom's own Studio and there is no file to sync; Twinkl is a commissioning publisher behind enterprise bot management, not a marketplace.
Both of those rest on public marketing pages rather than seller-facing documentation and should be treated as provisional.

The small teacher marketplaces are worth one email each and nothing more until answered.
Classful charges "a seller fee (5%) and a processing fee (2.9% + $0.30)"; Amped Up Learning advertises "80%-90% commission ... with no yearly/monthly fees"; Teach Simple returns 50% under a subscription model ([Classful](https://classful.com/sell-products/)).
None advertises a bulk import or CSV path anywhere on its public seller pages, which is consistent both with "it does not exist" and with "it is a manual arrangement you have to ask for" — an absence-of-public-evidence finding, not a verified negative.
Amped Up Learning runs on BigCommerce, evidenced by `x-bc-store-id` and `x-bc-is-ha` response headers, which makes the ask concrete: a vendor-scoped app or ingest endpoint rather than "please build an API".
Whether BigCommerce's Catalog API supports attaching digital download files is unverified and must be checked before that enquiry is sent.

### The market-size warning that belongs in the kill criteria

Amazon Ignite was this exact business with Amazon's balance sheet behind it, and it is gone.
The last archived capture carries the notice verbatim: "As of January 30, 2023, Digital Educational Resources publishing capabilities will be discontinued and your ASINs will be delisted", against a programme offering 70% royalty, an invitation-only quality bar, worldwide tax handling, and the explicit answer "Can I also sell my resources elsewhere? Yes" ([archived 14 April 2023](http://web.archive.org/web/20230414215735/https://ignite.amazon.com/)).
Every Wayback capture from 31 August 2023 returns HTTP 500.
The second-largest e-commerce company entered this market with a better royalty than TPT and exited in under four years.
That is a demand-side signal about the size of the non-TPT teacher-resource market, and the assumption that sellers want to be on many channels needs direct evidence from paying customers before the TPT connector is built.

## 4. Competitor verdict, with coverage

The verdict is that no product cross-lists between any two education marketplaces, and none is close.
This is now the best-tested claim in the entire research set, and it should be read as a warning rather than as validation.

### How thoroughly it was tested

One lane ran the search and two independent adversarial passes attacked it.
Coverage achieved: the Chrome Web Store searched by channel vocabulary and by function vocabulary, in English and German; the Apple App Store via the iTunes API; Google Play; GitHub repository and code search via `gh`; the npm registry; Hacker News via Algolia; the Fiverr gig market via its search JSON; YouTube search and video metadata; TPT's Zendesk help-centre API enumerated in full at 415 articles and 430,913 characters; eduki's Zendesk help API; and fourteen to seventeen named cross-listing and feed-management vendor sites read directly, plus List Perfectly, Alura, Listera and eRank added by the verification passes.

Coverage not achieved, and this matters: general web search was unavailable to every pass, with the budget exhausted before the lane's first call and every scrapeable engine returning CAPTCHAs, 403s or antibot walls.
Reddit blocked both the lane's and the verifiers' access on every route.
Facebook returned an error page or login wall to every automated request, so the two highest-value sources — the "Teacher Seller Marketplaces" group and the "TpT Virtual Assistant Finder" group — remain unread.
Product Hunt, Upwork, Indie Hackers and individual Fiverr gig pages were blocked.
Crosslist's FAQ pages 404 and the quoted feature-request text could not be located, though the roadmap evidence itself is solid and self-sufficient.

### What survived

The Chrome Web Store returns eight to twelve TPT extensions and every one is read-only analytics, SEO or keyword research; a search for Tes returns nothing relevant at all, which is sharper than the lane framed it, since there is no evidence of tool-buying behaviour among Tes authors whatsoever.
Vendoo supports exactly eleven marketplaces and Crosslist eleven, none education; keyword tests across the remaining vendors returned zero hits for "teachers pay teachers", "tes.com", "lesson plan" or "teaching resource" ([Vendoo](https://www.vendoo.co/marketplaces)).
Crosslist's public user-voted roadmap, 175,439 bytes of server-rendered content, contains zero occurrences of teacher, tpt, education, lesson or digital, and its top new-marketplace requests are OfferUp, Vestiaire Collective, TikTok Shops, Instagram Shopping and Amazon ([Crosslist roadmap](https://feedback.crosslist.com/en/roadmap)).
GitHub has six repositories matching "teacherspayteachers", the only automation-named one at one star and last pushed 2023-04-25.
TPT's help centre still documents no bulk-create or bulk-edit path, so the platform-risk clock round one started has not advanced on the marketplace's own side.

The verification passes added two findings that strengthen the negative.
The barrier is the domain model, not the plumbing: Crosslist's Etsy page runs to 60,728 visible characters with zero occurrences of "digital", its hero example is a pair of Jimmy Choo heels, and it ships size charts and per-marketplace fee calculators.
TPT's listing model — grade level, subject, standards alignment, file bundles, licence terms, no shipping, no condition, no size, no inventory — shares almost no fields with that.
The extension plumbing transfers; the schema, the taxonomy and the customer do not.
And the freelance market independently reproduces the same shape: fifteen-plus distinct Fiverr sellers offer TPT virtual-assistant gigs at $5–$100 entry with review counts up to 120, and not one offers cross-listing to other marketplaces, while resale cross-listing gigs are dense and name their tools by product.

### The defects, carried

Four of the lane's supporting claims are wrong or unsound and must not be repeated.

The 46:1 install-base ratio is arithmetically wrong.
The lane listed eight TPT extensions and summed only six; SEO Analyzer for TPT (871 users) and SellerSpy (1,000) were dropped.
The true total is 3,503 and the ratio is 21.4:1, so a thousand customers would be 29% of the installed base, not "a majority".
The direction survives; the rhetorical force does not.

The Chrome Web Store enumeration is a result-cap artifact, not a census — the store returns roughly eight to nine results per query and different queries returned different eights.
Worse, the lane searched only by channel and never by function.
Searching by function shows the exact product category alive and commercial for every adjacent marketplace: BulkListingPro ("Etsy Bulk Upload & Listing Tool: AI Tags, CSV Import", featured, 248 users, "No API keys, no separate desktop app"), Listera, Etsy Auto Lister (uploads .pdf and .zip digital files with "built-in random delays (3-6 seconds)"), KDP AutoGo, MerchWhale, BulkBay, and several Facebook Marketplace tools.
The correct finding is that this category exists for every adjacent marketplace and stops precisely at education, which means browser-driven bulk publishing against a no-write-API marketplace is a proven shipping pattern rather than an unproven bet.
Build risk is lower than round one assumed; the demand question is exactly as open as it was.

A six-item feature quote attributed verbatim to Mr Joel's TPT product page appears on none of the three pages cited.
The substance is roughly right and the second quote from the Chrome listing is verbatim, but the attribution is false.
The underlying finding stands and matters: Mr Joel's TPT Seller SEO & Listing Optimizer App sells at $79.99 one-time with 180 ratings at 4.97 stars, and its product page does advertise a "Bulk Product Importer" and operation on TPT product and edit pages ([TPT listing](https://www.teacherspayteachers.com/Product/TPT-Seller-SEO-Listing-Optimizer-App-Create-Titles-Description-Product-Links-13645689)).
It is one feature away from the founder's core, which makes it a more plausible entrant than any resale incumbent.
Round one should stop treating AI listing rewriting as a paid differentiator.

The lane's most adverse finding — that sellers migrate only hand-picked best-sellers — rests entirely on Reddit comments that neither verifier could reach.
It should be marked medium confidence, and it is not the only evidence for that behaviour: the unit-economics lane measured it independently and behaviourally, finding that ten confirmed dual-listers hold 173 Tes listings against 7,644 TPT listings, or 2.3% of their combined catalogue.
Two independent methods pointing the same way is enough to act on; one blocked Reddit thread is not.

Two smaller items.
TPT's bulk-edit window is not as open as the lane concluded: Bearwood Labs has sold a Product Description Editor Pro at $29.00 on TPT itself for roughly five years, so third-party write operations against TPT listings are an established commercial capability, and Bearwood gives away a free catalogue-to-Excel export.
And "Alva", cited in round one as a same-persona comparable at $8.99/$18.99/$28.99, could not be identified by any pass; round one gave no source.
Treat round one's $19 anchor as unsourced and drop it.

### What follows

Reclassify the no-competitor finding.
The niche is empty partly because free substitutes already absorb it: Teach Simple states verbatim that "our team at Teach Simple will upload all your products for you", and TPT ships a Virtual Assistant seat expressly permitted to "create new resources, update your product listings ... upload new resource files" ([Teach Simple](https://teachsimple.com/become-a-contributor); [TPT help article 4412826604820](https://help.teacherspayteachers.com/hc/en-us/articles/4412826604820-What-can-a-Virtual-Assistant-do-in-my-store)).
A marketplace hungry for supply will always underprice a tool at zero, so the only defensible channels are the ones already large enough not to care — which is exactly TPT and Tes.
The VA finding also resolves round one's tenancy question favourably: a VA seat is TPT's own sanctioned multi-operator path, so an agency tier is legitimate, and the product must beat a human on cost while conceding the judgement-heavy work.

## 5. The compliance floor

Round one carried no privacy, consumer-law, insurance or payment-acceptance analysis.
This is the pre-launch checklist.
Costs marked as ranges are modelled from published rate cards, not quotes; every one should be re-priced before it is relied on.

| Item | Owner | Cost | Timing |
|---|---|---|---|
| Stripe pre-approval, describing the product accurately | Founder | Free | Before billing code |
| FastSpring priced fallback quote | Founder | Free | Same week |
| Customer terms drafted against the Australian Consumer Law | Australian lawyer | $3,000–8,000 | Before first paying customer |
| Automation-posture opinion, CFAA and cross-border terms | Boutique tech or IP counsel | $3,600–13,000 | Before TPT connector |
| Privacy Act s 6D(4) opinion on buyer-data collection | Australian privacy lawyer | $1,000–3,000 | Before agent code |
| Technology liability plus cyber cover | Founder, via broker | About A$2,500–3,000 a year | Before first paying customer |
| EU and UK Article 27 representatives | Prighter or equivalent | €840–1,700 a year | Before offering to EU or UK |
| Click-through DPA, SCCs plus UK Addendum, one transfer risk assessment | Lawyer, one engagement | $1,500–4,000 | Before first EU or UK customer |
| Incident runbook written to GDPR's 72-hour clock | Founder | Free | Before launch |
| Agent invariant: no buyer data leaves the device | Engineering | Free | Before agent code |
| Wind-down clause in the customer terms | Founder | Free | With first paid customer |
| Software escrow including the signing key | Codekeeper or equivalent | About $1,670 a year | Deferred to 200 customers |

### Payment acceptance is a possible pre-launch kill, and two lanes disagreed

The two default indie-SaaS merchants of record are the two worst choices for this specific product, and the disagreement between lanes must be settled on clause text rather than on price.
Polar's acceptable-use policy names "Services to circumvent the rules, paywalls or terms of other services" as an outright prohibited category, alongside "Any product or service that enables unauthorized access to data belonging to another party" ([Polar](https://polar.sh/legal/acceptable-use-policy)).
Paddle's policy, last updated 13 April 2026, prohibits any product that "infringes upon, or enables the infringement upon copyrights, trademarks, terms and conditions, or trade secrets of another party" — the words "terms and conditions" appear expressly — and separately bans "Captcha Solving" as a category ([Paddle](https://paddle.com/support/aup/)).

The unit-economics lane recommended Paddle at 5% + 50¢ as a cost-neutral way to remove the multi-jurisdiction VAT exposure.
That recommendation is withdrawn.
Acceptance is not a fee question, and discovery would come after launch when customers are already subscribed.
The Paddle CAPTCHA clause also couples the billing rail to the adapter roadmap: rule out CAPTCHA solving and Paddle in the same decision.

Stripe's restricted-businesses list contains no anti-automation, anti-scraping, terms-circumvention or credential clause at all; the only clause that reaches this product is the general third-party intellectual-property one ([Stripe](https://stripe.com/legal/restricted-businesses)).
FastSpring's prohibited list is short and does not mention automation or third-party terms, making it the clean documented fallback, though its pricing is unpublished and must be quoted ([FastSpring](https://fastspring.com/terms-use/seller-terms-service/)).
Lemon Squeezy is being folded into Stripe Managed Payments and should not be built on.

Stripe Managed Payments supports Australian businesses selling software and "electronically supplied business and web services", warranting that "You hold all necessary rights and licenses to distribute the product" and requiring "a low historical dispute rate", with Stripe able to "issue refunds within 60 days of purchase in certain cases" ([Stripe docs](https://docs.stripe.com/payments/managed-payments/eligibility)).
Two consequences: the churn model must assume refunds the founder does not control, and the sync-failure blast radius is now a billing-continuity risk as well as a trust one.
Note that merchant-of-record fees are charged on the tax-inclusive total, not the net price — Polar's published worked example computes its fee on a $37.50 total for a $30 product with 25% Swedish VAT — so a UK cohort at 20% VAT costs meaningfully more than the headline rate suggests ([Polar fees](https://docs.polar.sh/merchant-of-record/fees)).
Whether Stripe's 3.5% is computed the same way is unverified and should be confirmed before the model is fixed.

### The liability cap round one implicitly assumed is void

At $19 or $29 a month every customer is a "consumer" under Australian Consumer Law regardless of business use, because the prescribed threshold is $100,000 rather than the $40,000 on the face of the Act ([Competition and Consumer Regulations 2010, reg 77A](https://www.legislation.gov.au/F1996B01420/latest/text)).
Section 60 guarantees services "will be rendered with due care and skill"; section 64 voids any term purporting to exclude it; section 64A(2) permits limitation, for services not ordinarily acquired for personal use, only to re-supply or the cost of re-supply; and section 64A(3) defeats even that where reliance on it is not fair and reasonable, with the court directed to "the strength of the bargaining positions" ([ACL](https://www.legislation.gov.au/C2004A00109/latest/text)).
A solo founder against a solo teacher-author is not an obviously unequal bargaining position, so that is a live argument for the customer.

Section 267(4) then permits recovery of "any loss or damage suffered by the consumer because of the failure to comply with the guarantee if it was reasonably foreseeable" — which is precisely the suspended-store, lost-income case, and for this product it is foreseeable by definition.
So the mitigation is not contractual.
It is architectural and evidential: dry-run-by-default writes, per-field before-and-after audit logs, destructive operations absent rather than merely off, and a change history that proves which write was the seller's instruction.
Round one already prescribed all of these for TPT-terms reasons; section 267(4) makes them the primary legal defence.

The trap is that the founder's own smallness brings the contract inside the unfair-contract-terms regime rather than exempting it from it.
Section 23(4) makes a contract a small business contract where "at least one party" employs fewer than 100 people or turns over less than $10,000,000, section 23(2A) and (2C) make proposing or relying on an unfair term a contravention, section 24(4) reverses the onus, and section 25(k) names "a term that limits ... one party's right to sue another party" as an example of an unfair term.
Penalties commenced 9 November 2023 and reach $2,500,000 for an individual, with each term a separate contravention ([Treasury Laws Amendment (More Competition, Better Prices) Act 2022](https://www.legislation.gov.au/C2022A00054/asmade/text)).
Importing a United States SaaS template with six aggressive clauses is six contraventions.
This is the cheapest catastrophic risk on the list to eliminate.

The exposure is insurable, which is the resolution.
DUAL Australia's Information Technology Liability wording carries extension 3.4, agreeing to pay "all loss and defence costs arising from any claim for civil liability for unintentional contraventions of the Competition and Consumer Act 2010 (Cth), the Australian Consumer Law" ([DUAL policy wording](https://au.dualinsurance.com/hubfs/DUAL%20ANZ/DUAL%20Australia/AUS%20policy%20wording/DUAL-AU-Information-Technology-Liability-Wording.pdf)).
The word "unintentional" is load-bearing, and the audit log and dry-run defaults are what keep a sync failure characterised that way.
Three operational rules follow from the same document: never admit a marketplace terms breach in any correspondence, because exclusion 8.13(b) triggers on an admission alone without any adjudication and the cyber wording's equivalent carve-back is weaker still; promise no SLA or uptime credit, because exclusion 8.5 excludes assumed contractual obligations; and disclose the automation model in full on the proposal form, since non-disclosure is a more reliable way to lose cover than any exclusion.
Indicative pricing is A$83 a month for IT liability, A$103 for professional indemnity and A$134 for cyber, but these are all-occupation portfolio averages and the underwriter's reaction to the disclosure is the real unknown ([BizCover](https://www.bizcover.com.au/information-technology-liability-insurance/)).

### Privacy: the Australian exemption is real and nearly useless

Privacy Act section 6D still exempts businesses under $3,000,000 turnover, intact in the compilation in force today, which would put a pre-revenue solo founder outside the Australian Privacy Principles and outside the Notifiable Data Breaches scheme ([Privacy Act 1988](https://www.legislation.gov.au/C2004A03712/latest/text)).
Do not let that become the compliance posture.
It gives zero relief from UK and EU GDPR, which apply directly under Article 3(2) to "the offering of goods or services ... to such data subjects in the Union" and require designated Article 27 representatives, with the "occasional" derogation unavailable to a continuous subscription.
And it gives zero relief from the statutory tort of serious invasion of privacy, which commenced 10 June 2025, applies regardless of turnover, is "actionable without proof of damage", caps non-economic and exemplary damages at $478,550, names "intruding upon the plaintiff's seclusion" as its first limb, and directs the court to "the means, including the use of any device or technology, used to invade the plaintiff's privacy" ([Privacy and Other Legislation Amendment Act 2024](https://www.legislation.gov.au/C2024A00128/asmade/text)).
A client that injects scripts into an authenticated marketplace session and reads pages carrying buyer names and order history sits squarely inside that description.

So make the boundary an enforced invariant, not an intention: allow-list the injected script's read scope by selector to the seller's own listing form, never persist or transmit buyer-identifying page regions, keep DOM captures and screenshots absent rather than off by default, and scrub crash reports of page content.
Write the incident runbook to GDPR's 72-hour clock rather than Australia's 30-day assessment window, since the stricter standard covers both.
Name Anthropic and the storage replica provider as subprocessors from day one and strip personal information from prompts, because OAIC guidance is that "the primary purpose for collection ... should be construed narrowly" and retrofitted notice cannot cure an undisclosed purpose ([OAIC AI guidance](https://www.oaic.gov.au/privacy/privacy-guidance-for-organisations-and-government-agencies/guidance-on-privacy-and-the-use-of-commercially-available-ai-products)).

Two open questions sit under this and neither is answerable by desk research.
Privacy Act section 6D(4)(d) removes the small-business exemption from an entity that "provides a benefit, service or advantage to collect personal information about another individual from anyone else", and the literal wording plausibly catches a paid service whose agent reads buyer data from a marketplace page; the consent carve-out does not help because the buyers have not consented.
And under the EU AI Act, in force from 2 August 2026 and reaching third-country providers "where the output produced by the AI system is used in the Union", Article 50(2)'s machine-readable marking duty attaches to providers rather than deployers, and whether wrapping a third-party model under the founder's own brand makes them a provider is unresolved ([AI Act](https://publications.europa.eu/resource/celex/32024R1689)).
Article 50(4) is disposed of twice over — listing copy is not public-interest information, and the seller-approves-before-publish step is an independent carve-out, which means that step is now doing legal work as well as quality work.

### One thing that helps

The Platform-to-Business Regulation gives the Tes cohort real recourse the TPT cohort does not have.
Article 4(1) requires a statement of reasons on a durable medium before or when a restriction takes effect, Article 4(2) requires 30 days' notice of termination, Article 4(3) requires an opportunity to clarify and reinstatement with data access on revocation, and Article 11 requires a free internal complaint-handling system ([Regulation (EU) 2019/1150](https://publications.europa.eu/resource/celex/32019R1150)).
The United Kingdom retained it with "United Kingdom" substituted for "Union", so it covers the whole Tes base rather than a slice ([legislation.gov.uk](https://www.legislation.gov.uk/eur/2019/1150/article/1)).
Build this into the product: when the client detects a suspension or listing removal for a UK or EU seller, surface the statutory entitlement, capture any statement of reasons, and pre-fill the internal complaint from the audit log.
That converts the per-field history from a liability shield into a customer benefit.
State the limit plainly in marketing: it does not cover United States sellers on TPT, which is the larger cohort with the harsher terms, so round one's near-zero tolerance for product-attributable suspensions stands unchanged.
The Digital Services Act adds nothing the product must do — it is not an intermediary service — beyond an overlapping statement-of-reasons entitlement and an out-of-court route, which fold into the same recourse flow.

Article 9 of the same regulation obliges providers to state in their terms what access business users have to their own data.
That establishes "the seller's own data" as a legally recognised category distinct from "marketplace data", which supports the read posture in section 6 without authorising it.
Neither platform's Article 9 disclosure was obtained, so it is support, not authority — and asking both platforms for it in writing is free and produces an answer worth more than any inference.

One further input for round one's still-open jurisdiction question: the ICO lists New Zealand under full adequacy and Australia nowhere, so on the transfer axis alone New Zealand is the cheapest of the three candidate structures and Australia the most expensive ([ICO adequacy](https://ico.org.uk/for-organisations/uk-gdpr-guidance-and-resources/international-transfers/adequacy-regulations/is-the-restricted-transfer-covered-by-adequacy-regulations/)).

## 6. Fleet operations and the selector-pack threat model

Round one pushes declarative JSON selector packs from the founder's host into every seller's authenticated marketplace session and scored that channel at zero risk.
It is the highest-severity item in the architecture.

### The blast radius, stated precisely

The worst outcome is not credential theft.
TPT states that "Deleting a product is permanent and cannot be undone" and that afterwards "you will no longer be able to access the listing or the files associated with it from your TPT account", while buyers who already purchased can still download — so the damage is invisible to TPT's own abuse signals and falls entirely on the seller ([TPT help article 360042429472](https://help.teacherspayteachers.com/hc/en-us/articles/360042429472)).
An attacker with an arbitrary action vocabulary across the fleet could permanently destroy every customer's income-producing inventory in minutes.

One bound is better than assumed.
Payout redirection is not reachable: TPT payout methods live entirely inside Hyperwallet behind a separate authentication, and no TPT article describes changing them from within the seller session ([TPT help article 360042580932](https://help.teacherspayteachers.com/hc/en-us/articles/360042580932)).
The reachable crown jewels are irreversible catalogue deletion, store-wide price change through the sale tool, and description or link injection — the last of which can itself breach TPT's cross-channel-link ban and get the seller sanctioned.
Say this precisely on the security page; overstating it damages credibility as much as understating it.

### The action vocabulary

Two browser vendors have already litigated this exact question and reached the same answer, and it is the same answer Chrome's Manifest V3 forces on the client anyway.
Server-pushed data is acceptable; server-pushed code is not — Mozilla requires that "Add-ons must be self-contained and not load remote code for execution", and Chrome permits a cached remote JSON configuration while banning `eval` and `new Function`.

So write the rule into `tam-types` as a type rather than a convention.
A pack step is a closed enum of verbs — locate, assert-present, set-value, select-option, click, attach-file, wait-for, read-back — with typed, validated arguments, and no variant carries a string that reaches the page as script.
No verb may return arbitrary page content to the host; assertions return booleans and enums only.
No verb may touch cookies or storage, and the extension must never request the `cookies` permission at all.
No destructive verb exists in Phase 1 — not disabled, absent from the interpreter — so that no pack, valid or forged, can express deletion.
If a step ever needs to be expressed as JavaScript, that is the signal it belongs in a signed client release rather than in a pack.

Per-origin scoping is enforced by the runtime, not asserted by the pack.
Under Tauri this would have been `on_navigation` returning false; under the extension it is the manifest's host permissions and content-script match patterns, which is strictly better because it is declared, reviewed by Google, and machine-checkable by anyone including Etsy's reviewers.

### Signing

Tauri's updater signs with minisign — Ed25519 with no rotation, revocation, threshold or expiry — which is adequate as an authenticity gate and inadequate as a supply-chain design.
The extension has no minisign verifier linked, so the verification mechanism must be re-specified: either the `minisign-verify` crate compiled into the WebAssembly core, or an Ed25519 verification in the offscreen document.
Which of those is available and reviewable under Manifest V3's content security policy is unverified and must be settled before the interpreter is written.

Whatever the primitive, wrap it in a manifest supplying the four things TUF names and bare signing lacks ([TUF specification](https://theupdateframework.github.io/specification/latest/)).
A monotonically increasing manifest version the client refuses to decrease, defeating rollback.
A short `expires` timestamp, on the order of 24 to 72 hours, that the client refuses to run past, defeating indefinite freeze.
One manifest signing the set of all pack hashes together, so a TPT pack cannot be paired with a stale Tes pack.
A per-pack content hash plus a hard byte cap, defeating wrong-software and endless-data.
Four fields and four checks are a day of work; do not adopt the `tuf` crate, whose 0.3 line has been in beta since October 2021 at 215 recent downloads, on the trust path of a fleet-wide instruction channel ([crates.io](https://crates.io/api/v1/crates/tuf)).

Keep the private key off the server that serves the packs.
Sign on an offline machine or a hardware token and upload already-signed bytes, so compromising the host yields the ability to serve stale packs but not to author new ones — which converts the worst case from fleet-wide execution into a denial of service.
Additionally sign each pack to Rekor with cosign and run a monitor against your own identity, because a signature can never tell you your key is being misused and an append-only log can ([Sigstore](https://docs.sigstore.dev/cosign/signing/signing_with_blobs/)).
The client need not verify the Rekor bundle, which keeps the experimental Rust Sigstore crate off the critical path.

### Rollout, and why server validation is not enough

CrowdStrike's July 2024 outage is the proof that a data channel is not automatically safe.
Rapid Response Content was explicitly "not code or a kernel driver" but "a representation of fields and values", it was server-validated, and it still took down a global fleet because "a bug in the Content Validator" let problematic content through and the on-sensor interpreter could not gracefully handle the resulting out-of-bounds read ([CrowdStrike PIR](https://www.crowdstrike.com/en-us/blog/falcon-content-update-preliminary-post-incident-report/)).
So the client must independently re-validate every instruction against the same schema before execution, bound every loop and every selector match count, cap total actions per job, and fail closed — treating a signed pack as authenticated but not as trusted.
Fuzz the interpreter against malformed packs as a flake check from the first milestone.

CrowdStrike's own remediation list is the deployment specification, and round one contained none of it.
Roll every pack canary, then 5%, then 25%, then 100%, with automatic halt on a rise in the selector-failure rate.
Let every tenant pin a pack version and opt into a slower ring.
Publish release notes per pack version.
Round one framed packs as a maintenance convenience — "one heal fixes every customer at once" — without noticing that the same property means one bad heal breaks every customer at once.

### The kill switch already exists

Round one's job lease is the fleet kill switch and was never named as a control.
Promote it to a first-class, documented control with dimensions — global, per-marketplace, per-verb, per-tenant, per-client-version-range, per-pack-version — and make it fail closed: a client that cannot reach the API, or whose short-lived signed authorisation has expired, must refuse to automate rather than continue on its last-known pack.
Without the fail-closed half, a network partition becomes an unkillable fleet.
Model the per-marketplace gate on Mozilla's blocking process, which is version-scoped and non-overridable: "When an extension is blocked, it is disabled in Firefox and users are not able to override the block", and "If an issue is known to affect only a subset of versions, the block may be applied to the affected versions specifically" ([Mozilla](https://extensionworkshop.com/documentation/publish/add-ons-blocking-process/)).
When TPT's upload form changes, disable create on TPT for affected client versions while leaving edit and everything on Tes running, so the customer loses one capability for a day rather than the product.
Minimum-version enforcement belongs on the lease API, not the update channel, because it is enforceable even against a client that never updates.

### Observability without capturing pages

Do not build DOM snapshot capture at all — not with masking, not opt-in, not only on failure.
The decisive objection is contractual rather than privacy-based: a DOM snapshot uploaded to the founder's server reproduces "user interfaces" and "computer code" onto a third-party computer for a commercial purpose, which is exactly TPT ToS section 4.A.
Masking does not rescue it; even Sentry's implementation ships URLs by default, describes its server-side scrubbing as "a best effort approach which pattern-matches the content", and instructs users to verify masking before production ([Sentry](https://docs.sentry.io/platforms/javascript/session-replay/privacy/)).
On a seller dashboard, URLs carry product identifiers and the retained structure is the interface expression the clause protects, so a fully-masked replay fails both tests at once.
Write this into the engineering charter with the clause quoted, because it will be proposed again by whoever is debugging at two in the morning and the privacy argument alone will not hold the line.

The proven alternative is yt-dlp's, which maintains the largest site-adapter fleet in existence without ever capturing page content: a mandatory structured diagnostic carrying command configuration, application version and build, Python and OS version, request handlers, extractor count, extractor name and trace ([yt-dlp issue template](https://github.com/yt-dlp/yt-dlp/blob/master/.github/ISSUE_TEMPLATE/1_broken_site.yml)).
Emit the machine equivalent automatically on every failure — client version, browser version, pack id and version, marketplace, step id, failure code, timings — shipped with the failure rather than requested, because the whole advantage over yt-dlp is not having to ask a teacher to run a command with a flag.

Make the failure taxonomy a closed, versioned, documented enum in `tam-types` shared by client, API and dashboard, following OpenTelemetry's discipline that `error.type` "SHOULD be predictable, and SHOULD have low cardinality" with an explicit `_OTHER` fallback ([OTel semantic conventions](https://opentelemetry.io/docs/specs/semconv/registry/attributes/error/)).
Proposed members: SelectorNotFound, SelectorAmbiguous, SelectorResolvedViaFallback, PreconditionElementAbsent, NavigationCancelled, UnexpectedOrigin, SubmitNoConfirmation, ChallengePresented, SessionExpired, UploadRejected, RateLimited, VerificationMismatch, PackExpired, PackRejected, and _Other.
A privacy- and terms-safe per-failure payload is constructible from this without page content: pack and step identity, failure code, matched-node count bucketed to zero, one or many, a fixed vector of expected-anchor booleans, a structural digest over tag names and ARIA roles only with all text, attribute values and ids excluded, the page fingerprint already computed for selector caching, timings, and a URL reduced to origin plus route template with id-shaped segments replaced.
This is a design proposal rather than a sourced finding, and the digest's non-invertibility should be validated once, in writing, before shipping.

SelectorResolvedViaFallback is the number that matters.
A selector degraded to its fallback strategy is a break that has not surfaced yet, so fallback-share is a leading indicator available in week one, against a kill criterion round one could only evaluate after two quarters.
Proposed tripwire, to be adopted as a written refinement of round one's operational kill criterion: evaluate the kill criterion immediately if fallback-share on either adapter exceeds 20% of steps, or if unplanned pack releases exceed two per marketplace per month for three consecutive months.
For calibration, yt-dlp saw 708 issues labelled `site-bug` in the twelve months to 24 August 2026 out of 2,212 total, roughly 0.39 reported breaks per adapter per year across about 1,838 extractors — and that is against a technically sophisticated, self-selecting, unpaying userbase filing structured reports.
Budget six to twelve unplanned pack releases per marketplace per year against actively-rewriting targets, and expect a third of the diagnostic information for a higher fraction of incidents, because teachers email.

### The canary, split in two

Run a read-only structural probe hourly: load the upload and edit forms, resolve every selector in the current pack, emit the same failure taxonomy, and create, submit or delete nothing.
That catches the dominant failure mode — the form's structure changed — at near-zero marketplace footprint, which matters directly against TPT's performance catch-all and Tes's discretionary fair-usage limit.
Run the write round-trip weekly as an edit on one real, genuinely-authored, low-traffic product the founder owns: apply a semantically null description change, verify by read-back, revert.
Exercise the create path at most monthly, creating with the active flag off so it is never purchasable, then deleting.
Do not create and delete throwaway listings: TPT's Seller Guidelines require titles and descriptions be "truthful, accurate and free of mistakes", duplicates are prohibited, and deletion is irreversible.
Use a real account the founder holds in his own name — TPT Basic Seller is a one-time $29, Premium $59.95 a year only if Premium-only surfaces must be covered — because the identifier-disguise clause makes a fabricated persona the single worst available choice, converting a grey-area automation question into an unambiguous guideline breach ([TPT seller fees](https://help.teacherspayteachers.com/hc/en-us/articles/360044219891-Seller-Fees-and-Payout-Rates)).
Accept deliberately that the canary is the account most likely to be restricted, because it is the most regular and most machine-shaped traffic the product generates and it is concentrated in one identity.
That also means the canary decision is downstream of counsel's opinion on the automation posture, not independent of it.

### Rate governance is a safety feature, not a nicety

Shared fate in this category is real and it lands on the customer.
A May 2025 report has Poshmark suspending a seller "for 4 items that sold on eBay that were delisted by Vendoo", refusing reinstatement and warning it would extend the ban "if 'excessive removal activity persists'".
Combined with the JA4 reputation mechanism in section 2 and Tes's unpublished discretionary limit, per-tenant rate governance must ship in the first shippable milestone rather than late.
Make it server-controlled rather than a client constant so it can be lowered fleet-wide without a release, start new accounts at a conservative ceiling on the order of 20 to 30 writes per marketplace per day, raise it with account age and clean history, and surface a queue with an estimated completion time rather than an error.

Of the plausible defences, only four are enforceable and the rest are theatre.
Build staged per-tenant rate governance; ClamAV scanning plus perceptual and content-hash deduplication at ingest, which is also the highest-yield cheap originality signal; a per-tenant and fleet-wide kill switch with a defined offboarding trigger and immediate refund; and the per-marketplace canary as the early-warning signal.
Drop AI-detection classifiers on customer files, manual review of uploads, plagiarism checking against marketplace catalogues, and any acceptable-use clause whose breach the founder has no mechanism to detect.
ClamAV is worth naming publicly, because it appears on TPT's own approved-scanner list alongside Microsoft Defender, Sophos and Norton, and it is already in round one's pipeline cost model ([TPT malware article](https://help.teacherspayteachers.com/hc/en-us/articles/12734392776468-What-does-TPT-do-if-they-suspect-a-resource-in-my-store-contains-malware)).

### The read posture, resolved

Change the operative rule from "never read TPT" to "never enumerate", and enforce it in types.
TPT's extraction clause has no volume threshold, no ownership carve-out and no purpose limitation, so arguing that one read is textually permitted is fragile and should stop.
The defensible argument is different in kind: TPT itself provisions a Product Statistics CSV export, a Sales Details download, a sales-tax export and a Privacy Center access-request flow, which demonstrates the platform's own distinction between a seller obtaining their own data and a third party extracting marketplace data.

Adopt a three-tier hierarchy.
Tier one is the first-party export where one exists — TPT's Product Statistics CSV, Tes's `/api/v2/dashboard/exportResources`.
Tier two is a single authenticated fetch of the seller's own listing by a durable id the product already holds, caused by and immediately following a write the seller authorised, one fetch per write.
Tier three is nothing.
Enforce it by making the fetch function take a `FetchReason` constructible only from a completed write receipt, so link-following, listing pages, search and pagination are unrepresentable rather than merely forbidden.
The consequences are that verification moves into the first milestone as non-optional, and round one's M7 shrinks to export-based drift reconciliation and stops being the most legally exposed feature in the product.
What is settled is the marginal exposure of one id-addressed read tied to an authorised write; what is not settled is either platform's Article 9 disclosure, and asking for it is free.

### The data floor round one deleted with the credential vault

Round one removed the entire customer-data-protection section when it removed the credential vault.
Three concrete holes remain and all are cheap to close.

PostgreSQL has no native transparent data encryption; its own documentation describes storage encryption at "the file system level or the block level" and warns it "does not protect against attacks while the file system is mounted" ([PostgreSQL](https://www.postgresql.org/docs/current/encryption-options.html)).
So say exactly that in the customer terms — full-disk encryption at rest, TLS in transit — and add application-level envelope encryption for the object-store blobs holding customers' resource files, so a storage compromise alone does not yield plaintext catalogues.
Reinstate per-tenant data-encryption keys for files specifically: the files are the asset, and unlike a credential they cannot be rotated after disclosure.

Row-level security silently does nothing under the obvious setup, because "Table owners normally bypass row security as well, though a table owner can choose to be subject to row security with ALTER TABLE ... FORCE ROW LEVEL SECURITY" ([PostgreSQL](https://www.postgresql.org/docs/current/ddl-rowsecurity.html)).
Round one's "enforces org_id scoping on every query" is therefore an in-code convention with no backstop.
Use a non-owner application role without BYPASSRLS, force row-level security on every tenant table, and carry a CI negative test asserting a cross-tenant select under the app role returns zero rows.
Watch the covert-channel case the same documentation warns about: referential-integrity checks bypass row security, so a unique constraint on a per-tenant natural key leaks the existence of another tenant's row and every unique index must be tenant-scoped.

The NixOS default Postgres backup is not a backup for a product holding customers' income-producing inventory.
`services.postgresqlBackup` is a same-host daily `pg_dumpall` at 01:15 with no WAL archiving and no offsite target, so the recovery point objective is up to 24 hours, the recovery time objective is unmeasured, and a host loss destroys the backups with the database ([nixpkgs module](https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/services/backup/postgresql-backup.nix)).
The fix is `services.pgbackrest`, but the module hard-disables `cipher-pass`, `s3-key`, `s3-key-secret` and the SFTP passphrase as `readOnly` and `internal` to avoid storing secrets in the Nix store, with a standing "TODO: Support passing encryption key safely" ([nixpkgs module](https://github.com/NixOS/nixpkgs/blob/master/nixos/modules/services/backup/pgbackrest.nix)).
So declarative point-in-time recovery to an SFTP repository on a second host works today; S3 or Backblaze offsite with repository encryption needs a sops-rendered config outside the module.
Budget half a day to two days, and check pgbackrest issue 2621 first in case a supported secret mechanism has landed.
Then put a restore drill on a quarterly schedule and record the measured recovery time, because GDPR Article 32(1)(c) requires "the ability to restore the availability and access to personal data in a timely manner" and 32(1)(d) requires "regularly testing" it — an untested backup is documented non-compliance, not merely a risk ([Article 32](https://gdpr-info.eu/art-32-gdpr/)).
Publishing the measured number is also a sales asset for a solo-founder product selling to people's livelihoods.

Software escrow at roughly $139 a month plus $199-an-hour release processing is seven customers at $29 and is indefensible early ([Codekeeper](https://codekeeper.co/pricing)).
Defer it to 200 paying customers and carry the load contractually until then: 60 days' notice, the export endpoint kept running through that window, a final signed pack with a long expiry so the client keeps working in local-only mode, and the client open-sourced under a permissive licence.
When escrow does arrive it must hold the signing key, not just the source, because losing it means never publishing another update.
Decide deliberately whether open-sourcing the client leaks the interpreter and action vocabulary to a competitor; the packs are server-delivered and can be excluded, but the interpreter would be public.

## 7. Revised unit economics and market sizing

### Market size

| Pool | Measured size | Method |
|---|---|---|
| TPT and Tes intersection | About 4,000 sellers | 474 TPT slugs probed against Tes shops |
| Tes American-orientation shops | 2,000–5,000 shops | 23,266 of 1,039,401 resources, 710 sampled |
| Etsy teacher-resource segment | Unmeasured, floor of 5,000+ items | Display-capped market page |

The intersection measurement is the load-bearing one and it was reached twice independently.
Probing 474 unique TPT store slugs under three name variants found 16 with a name-identical Tes shop and 10 with any live inventory, and spot-checks confirmed genuine identity matches rather than name collisions; the sample is drawn from search-ranked results and so is biased toward successful sellers, making these upper bounds ([example](https://www.tes.com/teaching-resources/shop/bespokeela)).
Use 4,000 as the planning denominator and state it, so the ceiling is derived rather than asserted.

The most damaging measurement in round two sits underneath it.
The ten confirmed active dual-listers hold 173 Tes listings against 7,644 TPT listings — 2.3% of their combined catalogue ([example](https://www.tes.com/teaching-resources/shop/sciencespot)).
Tes shop pages cap at about 20 visible items with no working pagination, so several of those are lower bounds, but even assuming 100 items each the crossing stays under 10%.
Round one's usage model assumes "40 listing writes per month" against a 200-product catalogue; the observed behaviour is a handful of hand-picked items over years, which is consistent with the community advice round one already found.
If sellers only ever move best-sellers, the addressable job is 5 to 20 listings, not 200, and that is a one-off migration rather than a subscription.
This must be settled with money before the migration engine is built, and section 9 gates on it.

### Ceiling, recomputed

Steady-state subscriber count is gross monthly adds divided by monthly churn, and round one modelled 1,000 simultaneous subscribers at zero churn.
ChartMogul's cohort data across 2,100-plus SaaS businesses puts top-quartile annual gross retention at 60–70% for average revenue per account under $50 a month, which compounds to 3.0–4.2% monthly for top-quartile performers and nearer 5–6% for median ones ([ChartMogul retention report](https://chartmogul.com/reports/saas-retention-report/)).

| Scenario | Steady-state subscribers | ARR at $29 |
|---|---|---|
| Pool 4,000, 10% converted, 6% churn | 185 | $64k |
| Pool 4,000, 10% converted, 3.5% churn | 317 | $110k |
| Pool 4,000, 20% converted, 3.5% churn | 635 | $221k |
| Pool 2,000, 10% converted, 6% churn, $19 | 93 | $21k |

The optimistic row requires converting one in five dual-listers on earth and holding top-quartile retention for a sub-$50 product indefinitely.
Replace round one's "$200k–250k ARR at a thousand paying customers" with $60–220k ARR at 150–650 subscribers, and re-anchor the kill criteria to subscriber count against pool share rather than to raw revenue.
An Etsy connector widens the denominator by an amount nobody has measured, which is a reason to measure it, not a reason to raise the figure.

There is a specific churn hazard round one did not name.
This product is a migration tool wearing a subscription's clothes: the initial cross-listing of a back catalogue is roughly twenty times the steady-state monthly need, so expect a churn spike two to three months after signup when the recurring job shrinks.
Package against it deliberately.

### Price

Round one's $19 anchor rests partly on "Alva", which no pass could identify and which round one did not source.
Drop it.
The evidence splits and the split is informative: analytics-only tools sell at $5.99 and $9.99 a month, while anything that actually moves a catalogue sells higher ([SEO Mantis](https://www.seomantis.com/pricing)).
List Perfectly starts at $29 and gates analytics onto its $49 tier; Crosslist runs $29.99 to $44.99 and gates analytics to its top two tiers; ExportYourStore runs $29 to $249 by listing count and charges a one-time $199 setup for Amazon or Walmart; Vela runs $29.95 to $54.95 per shop for a strictly less capable product on the digital axis ([ExportYourStore](https://exportyourstore.com/pricing/)).
This persona also demonstrably buys at higher prices than $19 a month: Bearwood Labs sells 18 products to TPT sellers at $19–$69 one-time with bundles at $117, $159 and $261.

So price at $29 a month minimum, tier by catalogue size, and add a one-time onboarding fee for the hardest connector on the ExportYourStore precedent.
That fee monetises the migration burst, which is where the measured value actually is, and it front-loads cash before churn can bite; modelled at $299 plus $19 a month it yields lifetime value of about $518 against $237 for subscription-only at 6% churn.
Consider per-listing credits as an alternative meter, since the closest live analogue in the adjacent market — BulkListingPro — sells credit packs from $1.99 with no subscription, and that meter fits the measured 5-to-20-listing behaviour exactly rather than fighting it.
Round one's implicit conclusion that a failing subscription means a failing business does not follow.

Also restore an analytics surface, which round one deleted on legal grounds without re-running the business model.
Analytics is the documented upgrade driver in both close comparables, so deleting it removes the renewal mechanism while leaving the churn.
Build it strictly from seller-owned data: Tes's first-party CSV export and authenticated dashboard endpoints, and seller-uploaded TPT sales and product-statistics exports.
That carries none of the exposure round one identified in pulling live listing state back from TPT, and it is the same tier-one read path section 6 already permits.

### Cost per user

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
| Total cost per user per month | $12.96 | $7.77 | $5.38 |
| Contribution after support at $29 | $16.04 | $21.23 | $23.62 |
| Contribution margin after support | 55% | 73% | 81% |

Every line below the first four is new relative to round one.
Payments is recomputed at the Australian international rate of 6.5% all-in on $29, rising to about 10.0% and $2.90 on Stripe Managed Payments; the merchant-of-record premium of roughly $1.01 per user per month, about $12,000 a year at a thousand users, is the correct price to pay to delete EU VAT, UK VAT and United States sales-tax registration, and should not be revisited as a cost decision.
Refunds are modelled at 1.5% of monthly revenue plus 0.2% disputes.
Support labour is modelled at $50 an hour and 12 minutes a ticket, with the ticket rate falling from 0.4 to 0.15 per user per month as onboarding matures; that rate is the widest error bar in the model, it has no citable benchmark, and it must be instrumented from the first ten paying customers rather than planned against.
Fixed compliance and legal amortises roughly $6,200 a year of ongoing legal, insurance and Article 27 representative costs, and excludes the first-year one-off opinions in section 5.

Two lines round one carried are now gone.
Code signing and paid native CI disappear with the desktop binary, which is worth roughly $700 a year plus a Mac.
That, together with support falling as onboarding matures, moves modelled break-even against a $5,000-a-month founder opportunity cost from roughly 400 customers down to roughly 250 to 300 — squarely inside what the measured pool can supply, which round one's 400 was not.
That is a modelled figure resting on a modelled support rate; treat it as a hypothesis to instrument, not a result.

Lifetime value at $29 and 300-user costs is $459 at 5% monthly churn, $656 at 3.5%, and $287 at 8%.
An LTV-to-CAC ratio of three implies an affordable customer acquisition cost of $96 to $219, and a twelve-month payback rule implies about $275.
Round one's "unable to support paid acquisition" is too strong as stated: the business cannot bid in an open auction against advertisers with ten times the revenue per account, but it can afford up to roughly $100 to $200 in targeted, small-absolute-dollar channels.

Round one's gross-margin figures of 88–91% are arithmetically fine and strategically misleading, because they exclude the two lines that dominate at realistic scale — support labour and founder time.
Manage contribution after support instead.

## 8. Go-to-market

### The tension, correctly named

The tension is not loud versus quiet marketing.
TPT's own Community Guidelines settle where you may market and, read together, remove the judgement call: "Don't send or post content that constitutes unsolicited or unauthorized invitations, advertisements, or promotional materials through our services, or to members of our services whose contact information you've obtained through our services", followed immediately by "This section does not limit a Seller's ability to purchase promotional space ... or advertise on other platforms or services" ([TPT Guidelines for All TPT'ers](https://help.teacherspayteachers.com/hc/en-us/articles/360043018571--Guidelines-for-All-TPT-ers)).
So never post in TPT's Seller Forum, never message sellers using details obtained through TPT, never scrape a seller list for outreach — each is a named violation by the account holder doing it — and treat everything off-platform as expressly permitted rather than furtive.

Visibility per se is also not the kill vector.
Vendoo publishes, in its own marketing, that "Poshmark's terms of service prohibit third-party automation tools" and that "In 8+ years ... we have never heard of a single account banned specifically for using a bot", alongside a ranked comparison table of six competing bots ([Vendoo blog](https://blog.vendoo.co/poshmark-bots-what-you-need-to-know-about-using-bots)).
The tools that were actually killed by platform attention — Mass Planner, Instagress, Archie in 2017 — automated actions against other users, not the operator's own assets; content-publishing tools operating on the account holder's own assets were not killed ([Fstoppers](https://www.fstoppers.com/social-media/mass-planner-shut-down-instagram-end-bot-era-176654)).
That externality line, not loudness, is what predicts death.
Write it into the public description and never cross it: the client touches only the seller's own store, only listings the seller owns, and never reads, ranks, messages, follows, reviews or scrapes anything belonging to another seller or buyer.
This makes round one's read-path abstinence a marketing asset, and it means competitor-intelligence features must be ruled out in writing rather than deferred — which also settles round one's open question 8.

Do not copy Vendoo's rhetorical posture.
Publicly conceding that a marketplace's terms prohibit you hands a future counsel a written admission, and section 5 shows that an admission alone triggers the wilful-breach exclusion in both insurance wordings without any adjudication.
Say what the product does and what it refuses to do; never publish a paragraph analysing whether a marketplace's terms forbid it.

The real residual tension is different and round two sharpens it.
Every high-leverage distribution channel for this persona is watched by the marketplaces, and the Etsy Commercial Access application requires honest disclosure of the whole product to one of them.
TPT Forward offers no third-party vendor or exhibitor path at all — the only route in is a presenter application where "We'll take a look at your store and social media", which is mandatory disclosure to the exact party whose objection is the top kill criterion ([TPT seller blog](https://sellerblog.teacherspayteachers.com/tpt-forward-2027/)).
The resolution is that the product must be describable in one sentence that is simultaneously true, marketable, and unobjectionable to every marketplace reading it.
The sharpest such sentence is compliance, not growth: TPT's own rules require that you "Keep your prices consistent if you offer your resources on other platforms" and forbid hyperlinks to alternative sales channels, and holding both by hand across a USD-GBP boundary is genuinely hard ([TPT guidelines on other sites](https://help.teacherspayteachers.com/hc/en-us/articles/360044219551-Guidelines-around-utilizing-other-sites)).
Lead with price parity and per-marketplace link scrubbing; never lead with bulk upload or automation.

Attend TPT Forward 2027 as a paying attendee for customer discovery only, and do not pursue the presenter route in year one.
Similarly, do not apply for Cloudflare Verified-bot status before TPT has been approached directly: Cloudflare has formalised exactly this category, including an "Intermediary" operator label and the observation that an intermediary "introduces transitive trust", but declaring under it publishes the product in a public directory and makes it blockable with one rule ([Cloudflare verified bots](https://developers.cloudflare.com/bots/concepts/bot/verified-bots/)).
Hold it as the designated response if TPT ever engages constructively, since it converts the product from an evasion story into a declared partner in one step.
Do adopt Cloudflare's internal model now by carrying a stable per-tenant identifier through the job ledger, which is what makes per-customer throttling and offboarding possible at all.

### Channels, in order

Require a credit card on the free trial.
ChartMogul's study of 200 products found median free-to-paid conversion of 8%, but "Free trials that require a credit card see 30% free-to-paid conversion – more than 5x ones that don't require one" ([ChartMogul conversion report](https://chartmogul.com/reports/saas-conversion-report/)).
At these volumes that single decision moves affordable acquisition cost more than any channel choice.

The persona's aggregators are seven active podcasts, most of whose hosts also sell courses and run Facebook groups, so each is simultaneously a slot, a list, a community and a potential affiliate: Teacher Business School (144 episodes, most recent 2026-08-12), Two Wacky Teacherpreneurs (83, 2026-08-08), Becca's Teacherpreneur Academy (123, 2026-07-15), Small Business Savvy (194), The Rebranded Teacher (220), Routine Your Dream (112) and The Creative Teacher Podcast (124) ([iTunes search](https://itunes.apple.com/search?term=teachers%20pay%20teachers%20seller&entity=podcast&limit=25)).
Approach them as recurring revenue-share affiliates first and advertisers second, because that is the only acquisition structure whose cost scales with success rather than preceding it.
Then buy three single-episode reads with distinct tracking URLs for under $500 total and let measured acquisition cost decide the rest; host-read rates run $18–26 CPM in 2026, so a 60-second read is $37.50 on a 1,500-download show and $150 on a 6,000-download one ([rate guide](https://influencerfee.com/guides/podcast-sponsorship-rates-2026)).
No download figure for any of these shows could be verified, so every number above the CPM is modelled — ask each host for their media kit, which is standard and free.

Join the six largest TPT-seller Facebook groups personally in week one and read the pinned rules, recording for each whether third-party tool mentions are permitted, confined to a weekly promo thread, or banned.
Membership numbers and rules could not be retrieved by any automated means across three lanes, so this is manual work.
That same afternoon closes the two most important untested items in the whole research set: whether a competing tool exists, and — in the "Teacher Seller Marketplaces" group run by someone who has been dual-listing for over eight years — whether sellers want whole-catalogue sync or only hand-picked migration.

Send the Tes partnership enquiry now.
The intake is live with four published questions and is a brand-marketing funnel rather than a product one, so the realistic outcome is a conversation ([Tes partnerships](https://www.tes.com/corporate/partnerships)).
Answer them in Tes's own terms and lead with supply: Tes's United States inventory is structurally separate from its GB inventory and thinly stocked, no bulk path exists to fill it, and every hour of author time saved converts directly into United States catalogue depth Tes cannot otherwise buy.
Offer three things that cost little and that Tes will care about — enforce the Author Code in the tool by blocking duplicate uploads and refusing external URLs in titles, descriptions and previews; respect the fair-usage limit with conservative rate governance; give Tes a named contact and a kill switch.
Ask for exactly one thing: written confirmation that an author using such a tool on their own session is within the Author Code.
That letter removes Tes from the kill-criteria list and is worth more than any feature.

## 9. Revised milestone plan

This supersedes round one's section 6.
Sizes are founder-weeks, one person full time.
The ordering front-loads the three things that can kill the product — paying demand, the file-injection assumption, and Etsy's approval — and every one of them is cheap.

| Milestone | Build weeks | Kill gate |
|---|---|---|
| M0 Paid concierge migrations, plus four free enquiries | 1 | Fewer than 25 paid migrations in six months |
| M0b File-injection spike | 1 | Injection rejected by both uploaders |
| M1 Tes GB-to-US duplication, first chargeable software | 5–7 | Web Store rejection with no viable resubmission |
| M2 Etsy connector, cloud plane only | 3–4 | Commercial Access refused |
| M3 Analytics from seller-owned data | 2 | None |
| M4 TPT connector, gated and premium | 5–7 | Any TPT or IXL written objection |
| M5 AI listing copy | 2–3 | None |
| M6 Billing, tiers, self-serve signup | 2–3 | None |
| M7 Drift reconciliation via first-party exports | 2 | None |

M0 is a business, not a build.
Sell 25 done-for-you catalogue migrations at $499–999, performed by hand on the founder's own machine in his own Tes and TPT sessions, with no software beyond a spreadsheet.
The precedent is explicit: ExportYourStore lists a fully-managed service across all four tiers and charges a flat one-time $199 for its hardest channels.
Twenty-five paid migrations is a far more severe test of demand than any interview, it earns while it tests, and it generates the fixture corpus and selector knowledge the adapters need.
It also directly answers the question the 2.3% crossing measurement raises, which no amount of further desk research can settle.
In the same week and in parallel, all free: file the Etsy Personal App and then the Commercial Access request with an Application Purpose that honestly names the multi-channel intent; send `authors@tes.com` the duplicate-upload question round one already flagged; send `partnerships@tes.com` the four-question pitch from section 8; send TPT the Publisher Membership enquiry round one recommended; join the six Facebook groups; and ask both platforms in writing for their Platform-to-Business Article 9 data-access disclosure.
Every one of those answers is decision-relevant and none costs anything but an email.

M0b is one week and it gates every browser connector.
Prove that a content script can fetch a Blob from the product's own API and place it into the Tes and TPT file inputs via a `DataTransfer` such that the uploader accepts a change event whose `isTrusted` is false.
Prove alongside it that a content script survives a soft single-page navigation mid-upload and can detect when it did not.
If injection fails, cut the browser client entirely, delete M1 and M4, and the product becomes Etsy-only in the cloud plane — which is precisely why M0's Etsy application must already be in flight.

M1 replaces round one's M0 and M3 with the single thing that is both testable today and chargeable.
Tes GB-to-United-States inventory duplication needs one connector, no cross-marketplace mapping, no bot-management vendor, and the founder's own author account.
It ships the extension with its packaged step interpreter and remote JSON packs; the cloud catalogue with per-tenant row-level security forced from the first migration; the file pipeline with ClamAV and content-hash deduplication; the leased job ledger with the fail-closed kill switch; server-controlled per-tenant rate governance; read-back verification behind the `FetchReason` type; the per-field audit log; the structured failure taxonomy; the hourly read-only structural canary; and the per-marketplace public status page.
The GB-to-US taxonomy projection is real work — 2,450 US-only and 2,390 GB-only subject paths, a four-phase to five-phase remap, systematic spelling differences — and it proves the mapping engine against a bounded problem before it is pointed at the harder case.
Charge by manual Stripe payment link; automated billing does not need to exist until roughly fifty customers.
Ship the status page in this milestone rather than later: a customer who can see that Tes create is degraded with an estimated fix time does not open a ticket, and support labour is the largest new line in section 7.

M2 is the cloud-plane Etsy connector and cannot start before Commercial Access lands, which is why the application is in M0.
It is OAuth with PKCE, `createDraftListing`, `uploadListingFile` with a real PDF, `uploadListingImage`, taxonomy lookup, patch to active, and read-back binding of the listing id — with the deterministic Etsy field validator that strips emoji and enforces the once-each budget on `%`, `:`, `&` and `+`, and with per-listing cost projection in the publish flow so the seller sees the $0.20-per-four-months charge before committing.
The 90-day refresh-token expiry becomes a first-class product state with a proactive re-auth nudge.
Enforce the firewall: no Etsy code path in the extension, and a manifest that does not and cannot name `etsy.com`.

M3 restores the analytics surface round one deleted, built strictly from seller-owned data — Tes's CSV export and dashboard endpoints, and seller-uploaded TPT product-statistics and sales exports.
It is two weeks and it is the renewal mechanism, so it should not slip behind the TPT connector.

M4 is TPT, deliberately fourth, gated behind the M1 spike round one scheduled, priced as a premium add-on, and designed so every screen is complete and worth paying for without it.
It carries the four measurements round one specified — session longevity before an OTP re-challenge, whether bot management challenges an ordinary seller machine, the real mandatory-field set and any velocity check, and the two undocumented bounds of preview-file size cap and minimum price — plus the ten-minute title-character-counting experiment.
It also carries the cross-marketplace layer: the canonical listing with per-marketplace projections, the price-parity guard as a first-class invariant, the deterministic link-scrub pass, and grades modelled as a canonical age interval carrying the seller's original declaration as provenance.
A written objection from TPT or IXL at any point stops it.

M5 is AI listing copy, descoped.
It is no longer a paid differentiator, because Mr Joel's app already sells AI title and description generation to TPT sellers at $79.99 one-time with 180 ratings, so price only the sync engine and treat generation as table stakes inside it.
Round one's three engineering constraints stand unchanged and one of them now does legal work as well: length is a deterministic Rust predicate rather than a model's judgement; numeric content claims are verified against the extracted fact sheet with a failed verification as a hard publish block; and every output is a proposal with a field-level diff that the seller approves, which is also the human-review carve-out under AI Act Article 50(4).

M6 and M7 are unchanged in substance.
M7 is no longer last-because-dangerous: routing drift reconciliation through TPT's own Product Statistics CSV export and Tes's `exportResources` removes the legal objection, so it shrinks from four weeks to two and can move earlier if a customer asks.

Kill criteria carried from round one stand in full.
Five are added or sharpened.
Fewer than 25 paid concierge migrations in six months stops the build outright, replacing round one's softer revenue gate.
Etsy refusing Commercial Access, or approving it only on terms that forbid naming TPT and Tes as destinations, removes the de-risking path and forces a re-decision.
Google rejecting the Web Store listing with no viable resubmission removes the client.
Fallback-share above 20% of steps on either adapter, or more than two unplanned pack releases per marketplace per month for three consecutive months, triggers the adapter-treadmill kill criterion immediately rather than after two quarters.
And the Amazon Ignite precedent becomes a standing question rather than a fact: the second-largest e-commerce company built this exact marketplace with a better royalty than TPT and closed it in under four years, so the assumption that sellers want to be on many channels must be evidenced by paying customers before M4, not assumed.

## 10. What is still unknown

Ranked by how much each blocks, with what it would take to know.

Whether a content script can put a file into TPT's and Tes's uploaders is the single load-bearing technical unknown and it gates every browser connector.
One week of work in M0b settles it.

Whether Etsy's Commercial Access review approves an Application Purpose that names TPT and Tes as sync destinations, given the divert-sales clause, and how long review takes.
Etsy publishes no service level and reserves sole-discretion rejection.
Asking is free and it is on the critical path; guessing is not an option.

Whether the 2.3% catalogue-crossing rate reflects seller preference or tooling friction.
These are opposite conclusions: if sellers cherry-pick because Tes traffic does not justify more, there is no market; if they cherry-pick because uploading 200 items by hand is unbearable, that is exactly the unlock.
Twenty structured interviews with the 16 identified dual-listers, whose contact details are public on both platforms, plus the 25 paid migrations in M0.

The actual support ticket rate per user per month.
It is the widest error bar in the cost model — at 0.4 tickets the business is marginal at 100 customers and at 0.1 it is comfortable — and no citable benchmark exists for a browser client driving two hostile marketplaces.
Instrument it from the first ten paying customers.

Whether Privacy Act section 6D(4)(d) makes the founder an Australian Privacy Principle entity from day one regardless of turnover.
A wrong answer means notifiable-breach duties and section 13H penalty exposure.
This needs an Australian privacy lawyer, not more desk research.

Whether the founder is a provider or only a deployer of an AI system under the AI Act when wrapping a third-party model under his own brand, since only the provider characterisation triggers Article 50(2)'s marking duty, which is in force now.
Scope it with an EU adviser at the same time as the Article 27 representative appointment.

Which signature primitive is available to a Manifest V3 extension for pack verification, and whether it survives Web Store review.
The minisign verifier that Tauri linked for free is not there any more.
Settle it before the interpreter is written.

Whether Google approves an extension whose single purpose is automating another site's seller dashboard.
Vendoo's 60,000-user listing is a strong precedent for resale but TPT and Tes are unproven.
Consider submitting a minimal read-only version early to establish a review relationship before the real submission.

Whether Stripe Managed Payments' 3.5% is computed on the tax-inclusive transaction total as Polar's published example shows for its own fee, since that materially changes the UK and EU cohort cost.
One question to Stripe.

Whether Tes will confirm in writing that an author using a session-driven tool is within the Author Code, and what Tes's take rate actually is.
Round one never established the take rate and it is needed to make the multi-channel economics argument to a seller.
Both are answerable by the emails already in M0.

What is actually in TPT's Product Statistics CSV export.
If it carries product id, title, price and status it replaces the entire TPT drift-reconciliation read path; if it carries only views and sales it does not.
One logged-in seller account settles it in five minutes and it changes the scope of M7.

Whether Vela intends to add digital-download support or education-vertical channels, since its own pricing FAQ says it plans to integrate more channels.
If digital payloads are on its roadmap the wedge narrows sharply.

Whether Tes has an equivalent irreversible-deletion semantic to TPT's.
The Tes author help centre was not located in any lane, and the founder, an existing Tes author, can answer it from his own dashboard immediately.
The blast-radius analysis for the Tes adapter is incomplete until he does.

Whether the client's TLS fingerprint actually matches stock Chrome, testable today against a JA4-reporting endpoint.
And what machines TPT and Tes sellers actually author on, since every device figure in round two is either consumer-wide or school-fleet and neither measures the seller's own authoring machine — ten minutes of questions in a Facebook group settles it better than any market report.

Two things could not be resolved at all and should stay marked.
"Alva", round one's same-persona price comparable, could not be identified by three separate attempts and round one gave no source; treat the $19 anchor as unsourced.
And the Reddit-sourced behavioural claim about selective migration remains unverifiable, because Reddit blocked every access route in every lane — it is corroborated by an independent behavioural measurement, which is why it is acted on, but it is not itself confirmed.
Facebook groups, Product Hunt and Upwork were unreadable throughout, and Tes Global's Companies House filings are image-only scans with no text layer, so nothing was established about Tes's revenue or the size of its Resources segment.
