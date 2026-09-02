# Vendoo: architecture, pricing, competitors, and the teacher-space gap

Read-only public-source research, conducted 2026-09-02.
Every URL below was fetched on 2026-09-02 unless a different date is stated beside it.
No account was created, no sign-in performed, and no marketplace contacted as an authenticated user.

## Executive summary

Vendoo is a thin cloud catalogue plus a fat Chrome extension: the web app is a React single-page app talking to Google Firestore directly from the browser, and every marketplace write happens inside the seller's own Chrome.
The extension's manifest is the clearest architectural statement they publish — MV3, a service worker, `cookies` and `webRequest` permissions, host permissions on nine marketplace domains, and a `declarativeNetRequest` rule set that rewrites `Origin`, `Referer` and `Access-Control-Allow-Origin` so extension-issued `fetch` calls are accepted by each marketplace as if they came from that marketplace's own listing page.
That is not DOM form-filling; it is the same plain-HTTP adapter approach we already have in Rust, relocated into a browser and given CORS cover by the browser's own rule engine.
The extension carries a generic page-script executor (`execPageScriptContent.js`) injected on every marketplace host, and `externally_connectable` limited to `web.vendoo.co`, so the web app drives the extension and the marketplace-specific logic can be updated without shipping a new extension version.
There is no `nativeMessaging` permission anywhere, and no Firefox, Edge or Safari listing was found — Chrome only.
The mobile apps are real and do crosslist to eight marketplaces without the extension, but they cannot import, cannot bulk delist or relist, and cannot do Facebook Marketplace or Shopify; and Vendoo states plainly that sale detection and auto-delist require the seller's computer to be on and connected.
Pricing is now unlimited-items subscription — $14.99 / $29.99 / $59.99 monthly, roughly 17% off annually, 14-day trial — which invalidates most competitor write-ups still quoting Vendoo's old per-item tiers and a $4.99 all-marketplaces add-on.
The support burden is exactly the fragility you would predict: Trustpilot sits at 4.2 with 23% one-star, and the recurring complaints are missed sale detection, marketplaces silently disconnecting, and support advising sellers to leave the computer running overnight.
The competitor set is architecturally split, and one finding contradicts our legal memo: PrimeLister's cloud automation asks the seller for their Poshmark username and password and stores them encrypted server-side, so server-side credential-holding does exist in the comparable set.
In the teacher space there is no cross-listing tool at all — the only comparable artefact found is marketplace-side migration, notably TeachBuySell's "Import from TPT", which scrapes a public TPT store URL into draft listings; the whitespace the founder is aiming at appears to be genuinely empty.

## 1. Vendoo's system architecture

### The web app and marketing site

`https://www.vendoo.co/` is a Webflow site.
Its markup loads `cdn.prod.website-files.com/.../webflow.*.js`, jQuery from `d3e54v103j8qbb.cloudfront.net`, plus Optimizely, HubSpot tracking and Visual Website Optimizer.
It is marketing only and tells us nothing about the product runtime.

`https://web.vendoo.co/` is the product.
The HTML shell is 4.3 KB and carries a comment stating it "Replaces Gatsby's src/html.tsx SSR template (removed with Gatsby)" and that head tags now come "from src/components/meta/GlobalHeadTags.tsx via react-helmet".
The single module script is `/assets/index-C191F85a.js` with a matching hashed CSS file, which is the Vite build layout, so the app is a React SPA that was migrated off Gatsby onto Vite.
The shell pre-connects to `https://firestore.googleapis.com`, `https://js.stripe.com`, `https://cdn.vendoo.co` and `https://fonts.gstatic.com`, and loads `https://cdn.vendoo.co/rw.js` with a `data-rewardful` attribute.

Three architectural facts follow from that shell.
The client talks to Firestore directly, so the primary datastore is Google Firestore and a large part of what would be a backend API is Firestore security rules plus Cloud Functions rather than a service the browser calls by name.
Billing is Stripe, client-side Elements.
Affiliate attribution is Rewardful.

`https://vendoo.statuspage.io/` returns HTTP 200 but redirects to `/inactive`, and `status.vendoo.co` does not resolve: there is no public status page, which is itself a signal given how often users report marketplace connections failing.
`https://www.vendoo.co/careers` states there are no current openings, so no job posting was available to corroborate the stack from the hiring side.

### The Chrome extension

The extension is *Vendoo Crosslist Extension v3*, ID `mnampbajndaipakjhcbbaihllmghlcdf`, listed at
`https://chromewebstore.google.com/detail/vendoo-crosslist-extensio/mnampbajndaipakjhcbbaihllmghlcdf`.
The store page reports version 3.1.10, last updated 26 September 2025, 70,000 users, 3.5 out of 5 from 69 ratings, and 1.32 MiB.
The developer is listed as Vendoo Inc, Silver Spring, Maryland, with a D-U-N-S number, declared a "Trader" under EU rules, and the privacy panel asserts the developer "will not collect or use your data".

The CRX package was fetched from Google's public update endpoint
`https://clients2.google.com/service/update2/crx?response=redirect&acceptformat=crx2,crx3&prodversion=120.0&x=id%3Dmnampbajndaipakjhcbbaihllmghlcdf%26uc`
and only `manifest.json` and `corsRules.json` were read; no code was copied or executed.
The package is 1.38 MB with twenty entries, of which four are JavaScript bundles shipped with their source maps: `service_worker.js` (270 KB), `execPageScriptContent.js` (552 KB), `vendooWeb.js` (62 KB), `interceptRequest.js` (61 KB) and `scripts/patch.js` (61 KB).

The manifest, verbatim in its structure:

- `manifest_version` 3, background is a single `service_worker`.
- `permissions`: `declarativeNetRequest`, `declarativeNetRequestWithHostAccess`, `background`, `tabs`, `cookies`, `alarms`, `storage`, `webRequest`.
- `optional_permissions`: empty.
- No `nativeMessaging` permission, and no native host anywhere in the package.
- `externally_connectable.matches`: `https://web.vendoo.co/*`, `https://internal-beta.vendoo.co/*`, `https://enterprise.vendoo.co/*`, `https://enterprise-internal-beta.vendoo.co/*`.
- `host_permissions`: `*://*.vendoo.co/*`, `*://*.web.app/*`, `*://*.depop.com/*`, `*://*.kidizen.com/*`, `*://*.facebook.com/*`, `*://*.mercari.com/*`, `*://*.grailed.com/*`, `*://*.poshmark.com/*`, `*://*.tradesy.com/*`, `*://*.myshopify.com/*`, `*://*.vestiairecollective.com/*`, `*://*.vinted.co.uk/*`, `*://*.vinted.com/*`, `*://*.ebay.co.uk/*`, plus `https://garage-pictures-0.s3.amazonaws.com/*`, `*://*.s3.amazonaws.com/*`, `*://*.s3-accelerate.amazonaws.com/*`, `*://*.filepicker.io/*`, `https://api.mapbox.com/*`, `https://cdn1.identitymind.com/dfp-wrapper/*` and `*://*.braze.com/*`.

Content scripts are declared in three groups.
`vendooWeb.js` runs on the four Vendoo web origins and is the page-side bridge.
`execPageScriptContent.js` runs on `www.facebook.com`, `web.facebook.com`, `www.mercari.com`, `www.kidizen.com`, `www.depop.com`, `*.tradesy.com`, `www.grailed.com`, `*.poshmark.com`, `*.etsy.com`, `*.myshopify.com`, `*.admin.shopify.com`, `*.vestiairecollective.com`, `*.vinted.co.uk`, `*.vinted.com`, `*.ebay.co.uk` and the single narrow pattern `*://*.ebay.com/lstng?draftId=*&mode=AddItem`.
`interceptRequest.js` runs on `www.facebook.com` only.
`web_accessible_resources` exposes `scripts/patch.js` to those same marketplace origins, which is the standard way an MV3 extension escapes the isolated world and runs code in the page's own JavaScript context.

Five things follow, and they are the substance of this section.

First, the pairing of `cookies` and `webRequest` host permissions with the marketplace domains means the service worker can issue authenticated `fetch` calls itself, using the seller's existing marketplace cookies, from the extension's own origin.
It does not have to drive the page.

Second, `corsRules.json` proves that is exactly what happens.
Sixteen `declarativeNetRequest` rules rewrite request and response headers per marketplace.
For Depop the rule sets request `origin: https://www.depop.com` and `referer: https://www.depop.com/products/create/`, and forces response `Access-Control-Allow-Origin: *`.
Mercari gets `referer: https://www.mercari.com/sell/`; Poshmark gets `https://poshmark.com/create-listing`; Grailed `https://www.grailed.com/sell`; Kidizen `https://www.kidizen.com/items/new`; Tradesy `https://www.tradesy.com/sell`; Vestiaire `https://www.vestiairecollective.com/submit-an-item.shtml`; Vinted `https://www.vinted.co.uk/items/new` and `https://www.vinted.com/items/new`.
Facebook is special-cased with `accept: */*`, `origin: https://www.facebook.com`, `referer: https://www.facebook.com/marketplace/create/item?ref=marketplace_vendoo`, and a response `Access-Control-Allow-Origin: https://www.facebook.com` rather than `*`.
Note the literal string `ref=marketplace_vendoo` — Vendoo self-identifies to Facebook in the referer it forges.
Shopify has two rules setting `origin` to `https://myshopify.com` and `https://app.shopify.com`; `filepicker.io` and `s3-accelerate.amazonaws.com` are given Grailed and Depop referers respectively, which places the image-upload legs on the same forged-origin path; `api.mapbox.com` gets `referer: https://web.vendoo.co/`; and eBay appears once, as `*://*ebay.co.uk/*` with `origin: https://www.ebay.co.uk`.

Third, the browser's own rule engine is being used to defeat the browser's own same-origin protections.
This is legal in the sense that Chrome ships the API and reviews the extension, but it is the mechanism by which Vendoo's "the request comes from the user's machine" posture is achieved, and it is worth naming precisely because it is the client-side equivalent of what our Rust adapters do with an explicit header map.

Fourth, `execPageScriptContent.js` is a generic executor rather than one script per marketplace, and it is 552 KB shipped with a 2.1 MB source map.
Combined with `externally_connectable` restricted to Vendoo's own web origins, the shape is: the SPA sends a message to the extension, the extension executes the marketplace-specific procedure, and the procedure body can be updated server-side.
That is how selector and endpoint drift gets fixed without a Chrome Web Store review cycle — the same problem our selector-rediscovery design addresses, solved by keeping the volatile part off the client.

Fifth, the absence of `*.ebay.com` and `*.etsy.com` from `host_permissions` while both appear in `content_scripts` is the sharpest available evidence for the API-versus-extension split.
Vendoo's site footer carries the two standard disclaimers, "This application uses the eBay API but is not endorsed or certified by eBay, Inc." and the same for Etsy, and no equivalent API sentence for any other marketplace.
On that reading, eBay.com and Etsy.com writes go server-side through official APIs, the eBay.co.uk leg needs the extension because it is not covered the same way, and the single eBay content-script pattern `/lstng?draftId=*&mode=AddItem` exists to finish a draft in eBay's own listing UI.

### Browsers

Chrome only.
No Firefox add-on, Edge add-on or Safari extension was found for Vendoo.
The help article "Get Started With Vendoo" (`https://help.vendoo.co/en/articles/6260267-get-started-with-vendoo-create-an-account-download-the-extension-connect-your-marketplaces`, dated 9 July 2026) names only the Chrome Web Store, and both connection-troubleshooting articles instruct the seller to be "logged into eBay on Chrome" and "logged into Etsy on Chrome" (`.../6778896-i-can-t-connect-my-ebay-account` and `.../6778943-i-can-t-connect-my-etsy-account`, both dated 9 July 2026).
Chromium-family browsers such as Edge and Brave can install a Chrome Web Store extension, but Vendoo does not say so and does not support it in writing.

### How connection works

The get-started article describes no credential entry and no OAuth redirect.
The seller presses connect per marketplace; if not already signed in, "Vendoo will prompt you to sign in", and if already signed in on the browser, Vendoo "will automatically connect to them".
That is browser-session detection through the extension's `cookies` permission, which matches the manifest exactly, and it is the mechanism behind Vendoo's public "we don't ask for your credentials" positioning that our legal memo already records.

## 2. The mobile apps

### The listings

iOS: *Vendoo: A Seller's Best Friend*, `https://apps.apple.com/us/app/vendoo-a-sellers-best-friend/id1612168777`.
Version 3.2.7, dated 20 August, 122.8 MB, Productivity, 4+, free, no in-app purchases listed, requires iOS 15.1 / iPadOS 15.1, and also runs on macOS 12.0+ with Apple silicon.
Rating 4.5 from 2,700 ratings.
Release notes are generic ("performance improvements and bug fixes") across recent versions.
Privacy discloses Contact Info, Usage Data and Diagnostics as linked to the user, across Analytics, Product Personalization and App Functionality; nothing declared as used to track.

Android: `https://play.google.com/store/apps/details?id=co.vendoo.mobile`, developer Vendoo, Inc.
Rating 4.1 from 726 reviews, 50,000+ downloads, updated 20 August 2026.
The description still calls it "the Vendoo **beta** mobile app".
Data safety declares no data shared with third parties, collection of "Personal info, App activity and 2 others", encrypted in transit, deletion on request.
The Play description lists connectable marketplaces as eBay, Poshmark, Etsy, Depop, Grailed, Mercari, **Kidizen** and Vestiaire Collective — note Kidizen, which does not appear on the current web marketplaces page.

The version strings present in the Play page markup run 1.0.1, 1.4.0, 1.9.2, 2.0.2, 2.1.2, 2.1.3, 2.2.2, 3.1.0, 3.1.4, 3.1.7, 3.1.8, 3.2.0, 3.2.4, 3.2.7, which is consistent with the iOS 3.2.7 current version and a shared release train across the two platforms.

### What mobile can and cannot do

The authoritative statement is the help article "Can I use Vendoo from my phone or tablet?"
(`https://help.vendoo.co/en/articles/9299090-can-i-use-vendoo-from-my-phone-or-tablet`, dated 9 July 2026, author Ben Martinez).

Available on mobile: creating drafts, photo editing and background removal, listing and crosslisting, inventory management, custom labels, individual delist and relist, mark as sold and multi-delist, accessing analytics, customer service chat, help centre.

Not available on mobile: importing, bulk delist and relist, sale detection and auto delist, CSV download, stale-listing warning system, Facebook Marketplace, and Shopify.

The same article says the optimal experience is a computer with the Chrome extension, and that a computer is mandatory to begin because importing is impossible from the mobile app or mobile browser.
"Which Marketplaces does Vendoo Support?" (`https://help.vendoo.co/en/articles/6260300-which-marketplaces-does-vendoo-support`, 9 July 2026) states all marketplaces work on desktop and all except Facebook Marketplace and Shopify on mobile.

The single most important sentence for our purposes is on the mobile marketing page (`https://www.vendoo.co/mobile-app`):
"your computer must be on and connected to the marketplaces for sale detection and auto delist to work in the mobile app".
The mobile app is therefore a full authoring and posting client but not an autonomous one — the extension on a running desktop is the daemon that watches for sales, and mobile inherits that dependency.

The mobile connection path is separate from the extension: the get-started article directs mobile users to Settings > Marketplaces inside the app.
A Play review dated 8 April 2026 says "I cannot currently connect to Depop because the screen refreshes away from the magic link page when I go to my email", which is a description of an in-app browser handling Depop's e-mail magic-link sign-in.
That places an embedded webview login in the mobile app, with the resulting session used by the app to post directly — architecturally the same thing our client-side note proposes with a Tauri webview.
Vendoo never states this in writing, so the mechanism is inferred from the review and from the fact that mobile posts to eight marketplaces with no extension present; it is listed under Unverified.

The mobile technology stack could not be established from public evidence.
No APK-analysis site in the results published a library list, and neither store listing names a framework.
Unverified.

## 3. Per-marketplace integration method

Sources: Vendoo's own help centre and site footer, plus the extension manifest and `corsRules.json` read on 2026-09-02.
"Extension" below means the marketplace domain appears in the extension's host permissions or CORS rules, which is direct evidence the extension issues authenticated requests to it.

| Marketplace | Method | Evidence | Mobile | Notes |
|---|---|---|---|---|
| eBay (.com) | Official API, plus extension for the draft-finish page | Footer: "uses the eBay API but is not endorsed or certified by eBay"; `.com` absent from `host_permissions`; content script only on `/lstng?draftId=*&mode=AddItem` | Yes | Connection still requires being signed in on Chrome |
| eBay (.co.uk) | Extension | `*://*.ebay.co.uk/*` in `host_permissions`; CORS rule 16 | Yes | The only eBay domain with a CORS rewrite |
| Etsy | Official API, extension present on-page | Footer: "uses the Etsy API but is not endorsed or certified by Etsy"; `*.etsy.com` in `content_scripts` but not `host_permissions`, and no CORS rule | Yes | Connection also gated on being signed in on Chrome |
| Poshmark | Extension | `host_permissions` + CORS rule 7 (`referer: /create-listing`) | Yes | Also a Pro-tier "Marketplace Sharing" automation target |
| Mercari | Extension | `host_permissions` + CORS rule 6 (`referer: /sell/`) | Yes | Most-complained-about connection in reviews |
| Facebook Marketplace | Extension, with a dedicated request interceptor | `host_permissions`, CORS rule 2, and `interceptRequest.js` content script on Facebook only | No | Desktop-only per help article |
| Depop | Extension | `host_permissions` + CORS rule 1 (`referer: /products/create/`) | Yes | Sharing tool target |
| Grailed | Extension | `host_permissions` + CORS rule 3; Filestack image rule 4 uses a Grailed referer | Yes | Sharing tool target |
| Vestiaire Collective | Extension | `host_permissions` + CORS rule 11 | Yes | |
| Shopify | Extension (app-store-billed) | `*.myshopify.com`, `*.admin.shopify.com`; CORS rules 9 and 10 | No | $9.99/month billed by Shopify from 15 October 2025 because "Shopify requires all apps in their App Store to use Shopify Billing" |
| Whatnot | Beta; method not evidenced | Help article marks Whatnot "(BETA)"; absent from the extension manifest entirely | Yes (listed) | Likely API or server-side; UNVERIFIED |
| Vinted | Extension | `*.vinted.com`, `*.vinted.co.uk` in `host_permissions`; CORS rules 12 and 13 | Not stated | On `/marketplaces` page as `vinted-us`; absent from the help-centre list |
| Kidizen | Extension | `host_permissions` + CORS rule 5 | Yes (Play listing) | Absent from the current web marketplaces page |
| Tradesy | Extension (legacy) | `host_permissions` + CORS rule 8 | No | Tradesy shut down in 2022; the rule is vestigial |

Two supporting hosts are worth noting because they say something about the product rather than a marketplace.
`cdn1.identitymind.com/dfp-wrapper/*` is a device-fingerprinting wrapper, which is what Vendoo needs to satisfy some marketplaces' risk checks from an automated client.
`*.braze.com/*` is customer messaging.

The published marketplace count is inconsistent across Vendoo's own surfaces, which is itself a finding: the help centre names ten with Whatnot in beta, `https://www.vendoo.co/marketplaces` claims eleven and includes Vinted, the homepage FAQ names ten without Vinted, and the mobile stores say eight.
A Play review from 23 July 2026 makes the same complaint: "I also thought they had 11 marketplaces, and that is one of the reasons I signed up. However, there are only 7 available", to which Vendoo replied "Marketplace availability can change as platforms update or discontinue services".

## 4. Pricing and packaging

From `https://www.vendoo.co/pricing`, read 2026-09-02.

The current, visible plan set is subscription with unlimited items:

| Plan | Monthly | Annual (per month, billed yearly) | Included |
|---|---|---|---|
| Starter | $14.99 | $12.49 | Unlimited items, all marketplaces, sale detection and auto-delisting, listing templates, importing, delist and relist, analytics, mobile app iOS/Android; annual adds live support 7 days a week |
| Growth | $29.99 | $24.99 | Everything in Starter plus AI Listing Enhancement, bulk actions up to 240 listings at once, up to 300 PhotoRoom background removals |
| Pro | $59.99 | $49.99 | Everything in Growth plus auto send offers on 6+ marketplaces, Marketplace Sharing for Poshmark/Depop/Grailed, up to 1,500 background removals, listing videos (annual copy says "for Poshmark and eBay") |
| Enterprise | Contact sales | — | For businesses creating over 1,000 new monthly listings |

Trial: "Get 14 days of free access to Vendoo when you sign up", full premium features, payment method required, billing starts automatically on day 14.
Annual billing is marketed as "Up to 2 months free" and works out at roughly 17% off.
Refunds: none on annual plans; annual plans cannot be modified until the cycle ends; monthly plans can be changed between cycles.
Sales tax applied by state.
Social proof on the page: "Trusted by 40,000+ Resellers in The US".

The same page still contains, lower down, a complete legacy item-count ladder: Free $0 / 5 items, Starter $8.99 / 25, Simple $19.99 / 125, Plus $29.99 / 250, Pro $49.99 / 600, Advance $99.99 / 2,000, Expert $149.99 / 4,000, with the philosophy "you're only charged for the new items you add each month; once you have them in Vendoo you can crosspost them unlimited times".
Both structures are present in the delivered markup, but the tab control at the top of the page switches between monthly and yearly views of the *new* structure, so the item-count ladder is stale content rather than a live offer.

This matters because most third-party comparisons are wrong about Vendoo's price.
Several 2026 write-ups state Vendoo requires a $4.99/month "All Marketplace add-on", or an $11.99/month bundle for import, bulk delist and all marketplaces, giving an effective $31.98/month.
The current pricing page includes "All Marketplaces" and "Importing" in the $14.99 Starter tier with no add-on named anywhere on the page.
Treat every competitor-authored Vendoo price as stale.

Gating of the two clients: the extension is not separately priced and is required to begin (importing is desktop-only); the mobile app is included from Starter upward and, per the mobile page, "can be used as part of their existing plan at no extra cost".
The only marketplace-specific charge is Shopify's $9.99/month, billed by Shopify rather than Vendoo.

Company context: Vendoo, Inc. is Y Combinator W22, founded 2017 by Thomas Rivas, Ben Martinez, Chris Amador and Josh Dzime-Assison, beta June 2019.
Reported total funding is small and inconsistently stated across aggregators — $125K to $650K across YC and a Google for Startups accelerator — and headcount is reported at 45 to 55.
Those aggregator figures (Crunchbase, Tracxn, CB Insights, Extruct) were not independently confirmed and are secondary.

## 5. Support burden: what actually breaks

Trustpilot, `https://www.trustpilot.com/review/vendoo.co`: 4.2 overall, 234 reviews, 185 in the last twelve months, distribution 71% five-star and 23% one-star.
That bimodal shape — very few three- and four-star reviews — is the signature of a product that either works for you or fails structurally.

Recurring failure classes, with sources.

Sale detection and auto-delist not firing.
Terri Davila, 17 July 2026, four stars: "issues with Vendoo delisting items when I sell on Ebay or Etsy... always have to go in and manual delist".
Adam Greene, 23 July 2026, one star: "Ive no been banned from Poshmark becuase Vendoo didn't update the quantity of an item" — a marketplace suspension attributed to a sync failure, which is the worst-case outcome of this class.
A further Trustpilot reviewer reports "endless amounts of item sold, yet the item not de-listed from other apps by Vendoo" and, notably, that support's remedy was "leave the computer open over the night so that the software can recognise the sales and delete the listing!?"

The computer-must-stay-on dependency, stated by users as a defect.
App Store review "Buyer beware", 21 June 2025: "you have to use chrome and have a computer on 24/7" for sales to register.
This is the direct consequence of the architecture in section 1: the watcher lives in the extension.

Marketplace connections dropping, especially Mercari.
Trustpilot, 1 December 2025, four stars: "issues with Mercari that remain frustrating but Vendoo says it's due to changes Mercari made".
A Polish-locale Trustpilot review: "Constant connectivity issues. Basics don't work consistently but they keep piling on new features."
App Store, 14 June 2024: "I can no longer, import from Mercari".
This is selector and endpoint drift surfacing as a support ticket, and it is the fragility we would inherit in any adapter-based design.

Mobile-versus-desktop gaps as a complaint rather than a documented limit.
App Store, 3 July 2022: "we still do not have the ability to import items on the mobile app".
App Store, "A Sellers Worst Nightmare!!": "marketplaces connected on desktop will not automatically appear connected".
Play, 8 April 2026: the Depop magic-link webview loses state when the user leaves the app to fetch the e-mail, and Poshmark "keeps giving me errors, as well, but not every time"; the same reviewer concludes "I have to use the computer or iOS app on my iPad, if I want to do anything".

General instability.
App Store, 4 May 2024: "every month there have been major glitches on the app for Mac and on desktop".
App Store, 13 June 2024: "it marks the item sold and decreases the inventory...but it's still listed".
eBay community forum, third-party crosslister thread: "I wasted $70 that I don't have on this junk extension that doesn't do what I need it to" (`https://community.ebay.com/t5/Selling/Third-Party-Crosslisters-Like-Vendoo-FLYP-cant-tranfer-listings/td-p/35034415`).

Support responsiveness and billing.
Ammo, 25 July 2026: "3 attempts made to contact, no response received. Appalling customer service".
Play, 18 May 2026: the chat widget being absent on both desktop and app, with Vendoo replying that support is "available 7 days a week during business hours, not 24/7".
Ricardo, 30 July 2026: refund refused after an AI feature under-delivered.

The pattern to carry forward: the two things that generate the most anger are a missed sale detection (because it costs the seller a suspension or a double-sale) and a silent connection drop (because the seller does not learn about it until damage is done).
Both are observability problems as much as automation problems.

## 6. The competitor set

Architecture claims below are marked by source strength.
Manifest-derived rows were read the same way as Vendoo's on 2026-09-02; vendor-page rows are the vendor's own words; comparison-blog rows are competitor marketing and are flagged.

| Tool | Execution model | Marketplaces | Mobile | Extension | Price (2026) |
|---|---|---|---|---|---|
| Vendoo | Chrome extension does the marketplace writes; eBay.com and Etsy via official API; sale detection needs the desktop on | 10-11 claimed | iOS + Android (Android "beta"), 8 marketplaces, no FBMP/Shopify, no import, no bulk, no sale detection | Chrome only, MV3, `cookies`+`webRequest`+DNR CORS rewriting, no nativeMessaging | $14.99 / $29.99 / $59.99 monthly, 14-day trial |
| List Perfectly | Chrome extension, broadest host permissions of any manifest read here | eBay(.com/.ca), Etsy, Poshmark(.com/.ca), Mercari, Depop, Grailed, Facebook, Kidizen, Shopify, Instagram, Vestiaire, Whatnot, Vinted, size.ly | Mobile app gated to Pro Plus tier | Chrome, MV3 `flpmljgbaphneikdjhmekdpiamkejfon`, v1.0.181.0, permissions `tabs storage management cookies offscreen declarativeNetRequest browsingData`, no `externally_connectable` | $29 / $49 / $69, Pro Plus $99 / $149 / $249 |
| Crosslist | Self-described "API-first whenever a marketplace supports it", extension for the rest | 11+ named incl. eBay, Poshmark, Vinted, Mercari, Depop, Etsy, FBMP, Grailed, Whatnot, WooCommerce | iOS + Android, "all core functionality, except autodelisting for some marketplaces" | Yes, browsers unspecified | $29.99-$44.99/month; 3-day money-back under 20 listings |
| PrimeLister | Split: crosslisting and relisting in the extension, Poshmark/Depop/eBay automation in the cloud | 8 | Cloud automation reachable from any device, no app install | Chrome extension v2 for crosslisting | Crosslisting $49.99, Poshmark automation $25, Depop $144/yr, eBay automation extra; ~$104.99 for all four |
| Flyp | Chrome extension; computer and browser must stay running; no automatic sale detection (seller marks sold) | 6: eBay, Depop, Poshmark, FBMP, Etsy, Mercari | — | Chrome | $9/month after a 100-day free trial |
| OneShop | Central dashboard plus mobile; auto-delists on sale | 4-5: eBay, Mercari, Depop, Poshmark (+Tradesy in some listings) | iOS + Android | — | $45/month flat, 7-day trial |
| Zipsale | API-first with a narrow extension: the manifest only reaches Vinted UK, Facebook, Vestiaire and Depop, so the rest go server-side | eBay, Etsy, Depop, Poshmark, Shopify, WooCommerce, ASOS, Vinted, FBMP | Web dashboard | Chrome, MV3 `enlmlibjldpednbbdachlpindfoaplng`, v0.0.90, `externally_connectable` to `web.zipsale.co.uk` | From £15/month + VAT, or credits from £0.18/item, add-ons from £10/month; free tier exists but excludes autodelisting |
| SellerAider | "Web (Chrome Extension) + Mobile" by its own description | 11+ | Yes, "needs improvement" by its own admission | Chrome | Standard $12/month after a $9.99 first month; Pro $29.99 after $19.99; 14-day trial, no card |

Three cross-cutting findings from this table.

The architecture split is real and it is the axis the market competes on.
Extension-resident execution (Vendoo, List Perfectly, Flyp) is cheap to build and inherits the desktop-must-be-on defect.
Cloud or API execution (Zipsale, PrimeLister's automation, and Crosslist's API-first claim) removes that defect and buys server-side reliability, at the cost of holding something durable on the server.

That cost is the second finding, and it directly amends our legal memo.
`docs/notes/legal/marketplace-terms-assessment.md:268` states "Server-side credential-holding automation appears nowhere in the comparable set."
PrimeLister's own documentation contradicts that for its cloud Poshmark automation: the seller must "simply enter your Poshmark account credentials (username/email and password) into the tool", "your credentials are encrypted and securely stored in the cloud", and connecting without saving those credentials "is not" currently possible
(`https://docs.primelister.com/features/poshmark-automation-tool`).
PrimeLister's stated rationale for the move is operational rather than legal: extensions have "the need for your PC to be on, rapid unstable updates, persistent bugs, and Chrome browser issues".
So the comparable set does contain a server-side credential-holding operator, running against a no-API marketplace, publicly documented, apparently unenforced against.
That is not a licence for us to do the same, but it removes "nobody does this" from the argument, and the founder should see it before the architecture fork is decided.

Third, Zipsale is the cleanest architectural model for a company in our position.
Its extension manifest reaches only the four platforms with no usable API — Vinted UK, Facebook, Vestiaire, Depop — while the pricing page carries API disclaimers for eBay, Etsy, Shopify, Facebook Marketplace and Vinted, and the rest of the catalogue runs server-side.
Minimum client surface, maximum server-side reliability, and the client exists only where the law and the API landscape force it.
That is the shape our client-side note argues for, arrived at independently by a UK competitor.

## 7. The teacher space

### Is there a cross-listing tool for teacher-resource sellers?

No.
Nothing in the searches below surfaced a product that lets a TPT seller push one resource to two or more teacher marketplaces from a single inventory.
Every crosslisting tool found is built for physical resale — eBay, Poshmark, Mercari, Depop, Etsy, Vinted, Grailed, Facebook Marketplace — and none of them lists TPT, Tes, Classful, Made By Teachers, Teach Simple, Amped Up Learning or Boom Learning as a target.

Searches run, all 2026-09-02:

- "cross-listing tool for Teachers Pay Teachers sellers bulk upload to multiple teacher marketplaces"
- "TPT seller diversify multiple marketplaces upload same resource tool automate"
- "reddit teacherspayteachers sellers tool to upload products to Classful Made By Teachers automatically cross post"
- "'cross-list' OR 'crosspost' teaching resources TPT Tes Classful tool software 2026 launch startup"
- "Classful 'import from TPT' import your TPT store bulk upload sellers"
- "'Made By Teachers' OR 'Teach Simple' OR 'Amped Up Learning' import your TPT products seller onboarding bulk"
- "'Teach Simple' contributor 'TPT' import store bulk upload resources onboarding help"
- "'Amped Up Learning' bulk upload sellers spreadsheet import products help"
- "Boom Learning sell decks import 'Amped Up Learning' seller store upload bulk"
- "Etsy sellers import TPT listings tool digital downloads bulk lister teacher resources"
- "Teachers Pay Teachers API for sellers bulk upload products help center 'bulk'"
- "'import your TPT store' OR 'import from Teachers Pay Teachers' marketplace seller onboarding 2025 2026"
- "teachshare.com sell your resources seller import TPT store creator launchpad commission"
- "Tes author 'bulk upload' resources multiple files upload tool tes.com author academy"

The nearest adjacent artefacts are generic Etsy bulk-listing tools — BulkListingPro and "Easy Listing Uploader" — which take a spreadsheet and create Etsy listings.
They import nothing from TPT; the seller supplies the CSV.

### Do the marketplaces themselves offer an "import from TPT"?

One does, verifiably, and it is the single most instructive artefact in this whole report.

**TeachBuySell** (Australia, `https://teachbuysell.com.au/`) ships a first-party "Import from TPT" tool, documented at `https://teachbuysell.com.au/help/import-tpt-products`.
Mechanics as documented on the page today:
the seller pastes their public TPT store URL (`teacherspayteachers.com/store/your-store-name`);
the tool confirms the store name and product count, then fetches, "usually taking under a minute";
TPT categories are matched to TeachBuySell subjects, year levels and resource types;
a price conversion is applied, defaulting to 45% for USD to AUD ("a $4.00 USD product becomes $5.80 AUD");
draft listings are created "with titles, descriptions, prices, thumbnails and preview files";
in-description links to other TPT products are "rewritten to point at your imported TeachBuySell listings automatically", and links to non-imported products are removed;
duplicates are detected by prior import or identical title and "never changed by an import";
review tooling includes a needs-attention filter with one-click apply-to-all, bulk edit of selected rows, find-and-replace across the batch, and per-row exclusion;
failures are surfaced per listing with one-click retry, and a summary e-mail is sent.
Access is gated: "To access the import tool, you'll first need to complete a quality review by publishing your first listing."
The one thing it does not do is the thing that matters most: "we can't transfer files you sell there" — the seller must download each resource from TPT and drag it into the Files & Previews tab before publishing.

Note a temporal contradiction worth flagging.
Search-index snippets of the same URL describe an older flow — "Email my TPT products", CSV files "split into 50-record batches", "Do not modify the headers", upload the CSV back into the tool.
The page as fetched on 2026-09-02 describes a live store-URL fetch with no CSV step.
The live page is the current behaviour; the CSV description is a stale index of an earlier version.
Both are recorded here because the older design tells us what they built first and the newer one tells us where it went.

**Classful** (`https://classful.com/sell-products/`) documents no bulk upload, no TPT import, no CSV import and no migration assistance.
Its published onboarding is sign up, upload resources individually, set prices, engage.
Fees are "a seller fee (5%) and a processing fee (2.9% + $0.30) per transaction" with no subscription, and sellers may list elsewhere ("Yes! We believe in you!").
A seller blog post dated 18 November 2024 (updated 16 January 2025), `https://snmsavedsinger.wordpress.com/2024/11/18/seller-platforms-comparison-tpt-classful-and-more/`, attributes bulk-import-from-TPT specifically to **TeachShare**, not to Classful or Amped Up Learning: "bulk upload from tpt is relatively painless", but "you have to manually edit words like 'tpt' or 'previews' or anything not applicable", "any links won't link (or they'll link to tpt)", and "once you've bulk uploaded, every new listing needs to be manually uploaded".
That is a single secondary source about a feature I could not confirm on TeachShare today: `teachshare.com/sell`, `/sellers`, `/creators`, `/import` and `/help` all return 404, and the homepage now presents TeachShare as an AI lesson-planning platform (Pace, Create, Differentiate, Assess, a Chrome Extension "Copilot", a Professional Hub) with the marketplace as one surface among several.
Classified as unverified, and possibly withdrawn.

**Made By Teachers**, **Teach Simple**, **Amped Up Learning** and **Boom Learning**: no import-from-TPT feature found on any of them.
Made By Teachers documents individual upload with 80% royalties and explicit permission to sell elsewhere.
Teach Simple runs a curated queue — every product is reviewed by a QA team, new contributors sit in a slower queue, and after ten quality-passing uploads the account is flagged for express approval within 48 hours (`https://teachsimple.com/blog/contributors/new-contributor-onboarding/` and `.../express-product-approvals/`).
Amped Up Learning's seller onboarding is an e-mail to `askus@ampeduplearning.com`, with 80-90% commission and no fees (`https://ampeduplearning.com/sell-with-us/`).
Teacha! (Snapplify) publishes an upload guide and a roughly two-working-day review, again per-resource.

**TPT itself** publishes no seller API and no bulk upload.
The seller help centre documents per-product upload and a "Create a Similar Listing" duplicate-and-edit affordance as the only batching mechanism.
Payouts: Basic sellers 55% with a $0.30 per-resource transaction fee, Premium 80% with $0.15.

**Tes** publishes an author uploader guide split into five sections (title and description, files and cover images and video, and so on) and no bulk uploader was found; Tes author support is `authors@tes.com`.

### What this means

Two distinct product shapes exist in this space, and only one of them is occupied.

The occupied shape is marketplace-side migration: a challenger marketplace builds a one-way importer from the incumbent, because acquiring sellers is its bottleneck.
TeachBuySell is the working example, TeachShare was reported as one, and both stop at the file boundary because resource files are behind TPT's authenticated download.

The empty shape is the seller-side inventory: one catalogue the seller owns, pushing to many marketplaces and pulling back sales, with the seller's own files held once.
Nobody occupies it.
That is Vendoo's shape, and in the teacher space it is vacant.

The asymmetry that makes it attractive is also worth naming.
Physical resale needs sale detection and delisting because inventory is one-of-one and a double-sale is a real failure.
Digital resources are infinite-inventory, so the hardest and most complaint-generating part of Vendoo's product — auto-delist and the always-on watcher — mostly does not apply to us.
What replaces it is cheaper: price and metadata drift, per-marketplace vocabulary mapping, and revision propagation when a seller updates a file.
That is a materially easier reliability problem than the one generating Vendoo's 23% one-star rate.

## 8. Cross-reference to our build

For each Vendoo capability, what we have, from the design notes and the tree.

| Vendoo capability | Us | Evidence |
|---|---|---|
| Inventory as source of truth, independent of any marketplace | Partial | Products and mappings exist in `tam-storage`, but the only creation path is `import_one` from an existing marketplace listing; `ProductRepo` has no update or delete (`docs/notes/design/creation-flow.md`, section 1) |
| Author a new product in the app | Missing | "there is no path by which a seller authors a product that was never listed anywhere"; the seven-endpoint authoring API is designed, not built (creation-flow.md sections 1-2) |
| Import an existing store | Have, one direction | `import_one` under the first-party-export capability, with an operator binary and a sync drain as its callers |
| Crosspost one item to many marketplaces | Have | `POST /{v}/jobs` plus the engine pump; `lower` turns intent into Create / Create+Publish / Revise; `requires_bound_on` gates publish behind the binding create |
| Full CRUD on a live listing | Partial | Create, publish and revise exist; `ItemOperation::Remove` exists but is enqueued only by the migrate drain; product update and delete are absent from the repo layer |
| Connect a marketplace account from the UI | Missing | `web/src/routes/connections` renders list and revoke only; the sole sealing path is the operator command `tam-session-broker link-from-jar` (platform-linking.md, opening) |
| Credential custody | Have, server-side | Four-role Postgres fence with `tam_broker` as the only reader of `connection_secret`; this is precisely the pattern the legal memo flags as the exposed one |
| Sale detection and auto-delist | Missing, and largely not applicable | No sale-detection code anywhere; digital resources are infinite-inventory, so this is not the same requirement |
| Analytics console | Partial | TPT all-time and time-resolved stats read on demand in `crates/tam-marketplace-tpt`; nothing persists a row, so `web/src/routes/analytics` has no durable series to read (analytics-console.md, Verdict) |
| Browser extension | Missing, by design so far | `docs/notes/design/client-side-architecture.md` recommends Tauri v2 with a login-only webview and keeps the extension "reachable as a later addition rather than a fork" |
| Native mobile apps | Missing | No iOS or Android target in the tree; `crates/` is server-side plus adapters |
| Client-side execution of marketplace requests | Missing, designed | The sans-io `Transport` seam means the adapters run unchanged against a client `ReqwestTransport`; the server becomes a control plane sending declarative intent (client-side-architecture.md section 2) |
| Subscription gating and kill switch | Partial | Billing foundation and dormant checkout landed in the UI console phase; the Ed25519 entitlement artifact carrying subscription plus per-marketplace grants is designed, not built |
| Templates, labels, bulk actions | Partial | `web/src/routes/templates`, `library`, `queue`, `jobs`, `sync`, `purchases`, `status`, `notifications`, `admin` exist as routes; bulk actions over many listings are not evidenced |
| Public status page | Missing | We have `web/src/routes/status` and `admin/health`, which is more than Vendoo publishes — their statuspage redirects to `/inactive` |

The single largest structural gap is the one the founder has already named: our marketplace request originates on our servers, and Vendoo's originates on the seller's machine.
Everything else in the table is ordinary product work.

## Open questions for the founder

1. Does PrimeLister's documented server-side credential storage change the architecture fork, or is it evidence to note and not follow?
   Recommendation: note it, do not follow it — it removes "nobody does this" from the risk argument but the memo's exposure analysis is unchanged, and TPT is a far more litigious counterparty than Poshmark.
2. Do we copy Vendoo's split, where the mobile app posts directly and the desktop client is the only thing that can watch for changes, or do we require parity?
   Recommendation: accept the split. Vendoo's mobile limitations generate complaints only because sale detection matters for physical goods; for digital resources the mobile app can be fully capable.
3. Chrome-only, or Chrome plus a native desktop client?
   Recommendation: native desktop first per the existing client-side note, with the extension deferred — Vendoo's Chrome-only posture is a competitive weakness, not a model.
4. Is marketplace-side "import from TPT" — the TeachBuySell shape — a product we should offer *to* challenger marketplaces as well as to sellers?
   Recommendation: park it. It is a real business but a different one, and it competes with our sellers' interests.
5. Whatnot is in Vendoo's list but absent from its extension manifest entirely. Should we probe how they do it before assuming API-or-extension is the only fork?
   Recommendation: no probe now; it is one marketplace outside our vertical.

## Unverified

- How Vendoo's mobile apps hold a marketplace session. An embedded webview capturing cookies is inferred from a Play review describing a Depop magic-link page inside the app, and from the fact that the app posts to eight marketplaces with no extension. Vendoo never states the mechanism.
- Vendoo's mobile technology stack (React Native, Flutter or native). No public APK library listing was found, and neither store listing names a framework.
- The exact eBay and Etsy split. The footer disclaims use of both official APIs and neither `.com` domain carries extension host permissions, but Vendoo's help centre still requires the seller to be signed in on Chrome to connect either, which an OAuth flow would not need. The hybrid reading is the best fit but is not stated by Vendoo.
- How Whatnot is integrated. It is absent from the extension manifest and marked BETA in the help centre; no method is documented.
- Whether Vendoo's extension works in Edge, Brave or other Chromium browsers. Technically possible, nowhere claimed or supported by Vendoo.
- TeachShare's bulk import from TPT. A single seller blog dated November 2024 describes it; TeachShare's current site exposes no seller or import surface and the relevant paths 404. It may have been withdrawn or moved behind login.
- Classful's bulk upload. Attributed to Classful by one search summary but contradicted by the primary blog post, which attributes it to TeachShare; Classful's own seller page documents individual upload only.
- Vendoo's funding and headcount. Aggregator figures ($125K-$650K raised, 45-55 staff) disagree with one another and were not confirmed against a primary source.
- Competitor pricing and architecture rows sourced from vendor comparison blogs (Crosslist, Vendoo, Nifty, SellerAider, Voolist, ResaleOS, FlipSail). These are competitor marketing, and the Vendoo rows in them are demonstrably stale.
- The count of marketplaces Vendoo actually supports. Their own surfaces say eight, ten and eleven, and a July 2026 Play reviewer counted seven.
