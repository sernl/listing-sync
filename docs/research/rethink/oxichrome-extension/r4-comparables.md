# R4: how cross-listing products ship their browser extensions

All pages fetched 2026-09-03.
Every figure below is quoted from the page named beside it; where two sources disagree the disagreement is recorded rather than resolved.
Chrome Web Store detail pages are abbreviated CWS; `chrome-stats.com` is a third-party mirror that publishes manifest data the CWS listing page does not expose.

A note on method, because it bounds what the permissions column is worth.
The CWS listing page does not publish an extension's `permissions` or `host_permissions` array; it publishes only the developer's data-category self-disclosure ("Website content", "Personally identifiable information") on the Privacy tab.
Direct fetches of `chrome-stats.com` returned HTTP 403, so manifest data below was recovered from search-engine reads of those same chrome-stats pages and is marked accordingly.
Mirror data lags: chrome-stats reports Crosslist at v3.11.15 / 2026-07-22 while the CWS page reports v3.11.42 / 2026-09-02, so treat mirror figures as a floor, not a current reading.

## Products

| Product | Extension name and store URL | Users | Last updated | Version | Manifest | Permissions (API) | Host permissions | Firefox (AMO) | Extension vs web app | Extension mandatory? |
|---|---|---|---|---|---|---|---|---|---|---|
| Vendoo | Vendoo Crosslist Extension v3 — `chromewebstore.google.com/detail/vendoo-crosslist-extensio/mnampbajndaipakjhcbbaihllmghlcdf` | 70,000 | 2025-09-26 | 3.1.10 | V3 (inferred: MV3-only permissions) | `declarativeNetRequest`, `declarativeNetRequestWithHostAccess`, `scripting` (chrome-stats) | Poshmark (.com/.ca/.com.au/.co.uk), mercari.com, facebook.com, upload.facebook.com, depop.com, eBay regional domains, vinted (chrome-stats) | None found | Web app holds catalogue, inventory, analytics; extension "allows Vendoo to access and communicate with your marketplaces" (CWS) | Yes — install is step 2 of the 3-step onboarding (help.vendoo.co) |
| List Perfectly | List Perfectly Multi-Channel — `chromewebstore.google.com/detail/flpmljgbaphneikdjhmekdpiamkejfon` | 9,000 | 2026-08-25 | 1.0.181.0 | V3 (chrome-stats) | Not published; chrome-stats flags "access to browser tabs" as critical | Not published | None found | Catalogue, AI listing assistant, images, analytics in the web app; extension does "crosslisting, delist, auto delist, Poshmark tools" | Yes for crosslisting; "You do not need the Chrome extension to create, edit, organize, and manage listings from your phone" |
| Crosslist (crosslist.com) | Crosslist — `chromewebstore.google.com/detail/crosslist/knfhdmkccnbhbgpahakkcmoddgikegjl` | 10,000 | 2026-09-02 | 3.11.42 | V3 (chrome-stats) | `alarms`, `background`, `cookies`, `storage`, `tabs`, `declarativeNetRequest`, `declarativeNetRequestWithHostAccess`, `scripting` (chrome-stats) | 25+ enumerated marketplace domains (full list below) | None found | "API-first whenever a marketplace supports it"; extension only "when an API isn't available for the workflow needed" | Conditional — not needed for API-backed marketplaces |
| PrimeLister | Cross-listing & Poshmark Tool by PrimeLister — `chromewebstore.google.com/detail/cross-listing-poshmark-to/eepbhjeldlodgnndnjofcnnoampciipg` | 9,000 | 2026-08-07 | 2.0.143 | V3 (chrome-stats) | Not published | mercari, poshmark(.com/.ca), eBay (.com/.co.uk/.com.au/.at/.ca), facebook, etsy, tradesy, depop, kidizen, grailed, amazon.ca, primelister.com, shopify, vestiairecollective, vercel.app, sentry.io (chrome-stats) | None found | Crosslisting runs in the extension; Poshmark automation was moved server-side and "operates entirely in the cloud" | Yes for crosslisting; no longer required for Poshmark automation |
| OneShop | OneShop: Sell on marketplaces — `chromewebstore.google.com/detail/oneshop-sell-on-marketpla/pcapaniacmdmabfopeeimmpjkkjpeiok` | 1,000 | 2021-12-12 | 2 | Not stated; see anomaly F7 | Not published | Not published | None found | Extension is only a link monitor: it syncs marketplace accounts and warns "when your account links disconnect" — it is not the posting engine | No — it is a "companion" extension; posting happens elsewhere |
| Closo | Closo — Sell on Every Marketplace / Closo Bot (`aipjhdapgmimfdfcjmlpeoopbdldcfke`) | 1,000 (mirror) | — | 4.0.19 (mirror) | Unknown | Not published | Not published | None found | "the extension uses your existing browser session"; eBay and Shopify use OAuth instead | Yes for Poshmark/Mercari/Depop/Vinted; no for eBay/Shopify |
| Flyp | Crosslister by Flyp (`kbflhgfmfbghhjafnjbgpiopcdjeajio`) | 40,000 (mirror) | — | 1.0.3 (mirror) | V3 (chrome-stats) | Not published | joinflyp.com, poshmark(.com/.ca), mercari, depop, facebook, `*.s3.amazonaws.com` (chrome-stats) | None found | Crosslisting, auto-import, bulk delist/relist, auto-delist on sale | Was yes; site no longer offers it |
| SellerAider | Crosslister - SellerAider — `chromewebstore.google.com/detail/crosslister-selleraider/hoadkegncldcimofoogeljainpjpblpk` | 6,000 | 2026-08-04 | 1.0.8.58 | V3 (chrome-stats, family-level) | Not published | Not published | None found | Extension fills marketplace forms; "no affiliation with any of the supported marketplaces" | Yes |
| Crosslist Tool (listifyx) | Crosslist Tool - Crosslisting Tool — `chromewebstore.google.com/detail/crosslist-tool-crosslisti/enohabfnmaecamadfmihenkkonedakek` | 281 | 2024-07-12 | 1.0.2 | Unknown | Not published | Not published | None found | Injects a crosslist icon onto a marketplace listing page and duplicates it | Yes |

Two listings in the table have no live CWS page as of the fetch date; see F6.

Full Crosslist host-permission list, from chrome-stats via search on 2026-09-03: poshmark.com, poshmark.ca, poshmark.com.au, poshmark.co.uk, mercari.com, facebook.com, upload.facebook.com, depop.com, grailed.com, ebay.com, ebay.co.uk, ebay.ca, ebay.com.au, ebay.ie, crosslist.com, kidizen.com, api.kidizen.com, etsy.com, bonanza.com, vinted.com, vinted.nl, vinted.co.uk, vinted.ca, myshopify.com, instagram.com, vestiairecollective.com, plus S3 storage domains.
Content scripts are declared against depop.com, ebay.ca, ebay.co.uk, ebay.com.au, ebay.com, ebay.ie, facebook.com, vinted.co.uk, vinted.com and poshmark.ca.

Also live in the store on the fetch date, from the CWS search for "crosslist": ListFlow (trylistflow.com, 4.0), Listelf — formerly Crosslist Magic (listelf.com, 4.2), Reselling Tool - Crosslist Tool (3.0), Crosslist: FLUF Connect Utility & Crosslister (fluf.io, 4.5), Ruit (ruit.es, 4.8), CrossList Guard (0.0), plus a "Load more" control indicating the list is not exhausted.
Adjacent listings found by other searches on the same date: Zipsale Crosslisting Plugin, Cross List It, CrossLister (crosslister.co), Fusion Lister, Poshmark Bot | Closet Assistant, and Free Poshmark Bot by Crosslist.

## Findings

F1 — The market has converged on one architecture, and it is the one this repository already committed to.
The server holds the catalogue, the inventory, the analytics and the subscription; the extension is the actuator that touches the marketplace.
Crosslist states the rule explicitly: "Crosslist® is API-first whenever a marketplace supports it, using secure, authorized connections", and "When an API isn't available for the workflow needed, Crosslist® uses a secure browser extension" where "actions happen directly on your computer" (crosslist.com, 2026-09-03).
Closo names the same split per marketplace: "eBay and Shopify connect through their official login (OAuth)", while Poshmark, Mercari, Depop and Vinted "are detected through the Closo extension once you are logged into that marketplace in Chrome" (closo.co get-started guide, 2026-09-03).

F2 — Not one vendor holds a marketplace credential, and every one of them says so on the record.
Crosslist: "We do not ask for your marketplace password" (crosslist.com, 2026-09-03).
PrimeLister: "PrimeLister will never ask you for any of your marketplace login information", and after the seller logs in normally "the PrimeLister Extension will get connected to your accounts automatically" (docs.primelister.com, 2026-09-03).
List Perfectly: "List Perfectly does not require the marketplace email or password used to log into selling accounts to crosslist", and "Sellers log into their own marketplace accounts in their own browser" (listperfectly.com, 2026-09-03).
Closo: "Closo never stores your marketplace passwords — the extension uses your existing browser session" (closo.co, 2026-09-03).
Vendoo: "If you're already logged into your marketplace accounts, Vendoo will automatically connect to them" (help.vendoo.co, 2026-09-03).
The consistency is the finding: the ambient-session model is not one vendor's preference, it is the category's only shipped pattern.

F3 — Host permissions are an enumerated marketplace allowlist, never a wildcard.
Crosslist declares roughly 25 named domains; PrimeLister declares a comparable named set including its own `primelister.com` and its error sink `sentry.io`; Vendoo and Flyp declare short marketplace lists (chrome-stats via search, 2026-09-03).
No extension examined declares `<all_urls>`.
That matters for review posture: a reviewer sees a bounded, purpose-legible set of hosts, and every added marketplace is a visible permission delta rather than a silent capability.

F4 — Google accepts this category; the store's own trust signals say so.
The CWS listing pages for both Crosslist BV and PrimeLister carry the badge "The publisher has a good record with no history of violations" (CWS, 2026-09-03).
So does "Free Poshmark Bot by Crosslist", an extension that names marketplace automation in its title (CWS search result, 2026-09-03).
A single store search for "crosslist" returns ten distinct live cross-listers with more behind a "Load more" control (CWS, 2026-09-03).
The category is a decade old, well populated, and unmarked by Google.

F5 — No public record of a cross-lister being removed for policy violation, and none of a marketplace forcing one out.
Searches for Chrome Web Store removals or suspensions of cross-listers, and for Poshmark cease-and-desist or litigation against Vendoo, List Perfectly, PrimeLister, Closet Tools, Simple Posher or PosherVA, returned nothing on 2026-09-03.
Poshmark's guidelines forbid "unauthorized programs or other forms of automation", but multiple 2026 vendor and review sources describe enforcement as effectively nonexistent, and the reported consequence is per-seller "share jail" rather than action against the tool.
The eBay community thread "Third Party Crosslisters Like Vendoo, FLYP can't transfer listings" turns out to describe a long-standing eBay account-isolation boundary, not a change eBay made; no eBay staff replied (community.ebay.com, thread posts dated ~1 year to ~1 month before 2026-09-03).
Marketplace-side pressure is real but takes the shape of surface churn, not enforcement: Mercari dropped tags and colour fields, which Vendoo absorbed as a listing-form change in its April 2026 product update; Poshmark introduced an Excessive Listing Removal Policy in May 2025 that alarmed cross-posters and scrapped it in July 2026; and a November 2025 Poshmark incident deleted listings for sellers "who use third-party services to access Poshmark" (valueaddedresource.net and blog.vendoo.co, read 2026-09-03).

F6 — Two of the named products have no live Chrome Web Store listing as of 2026-09-03, cause unestablished.
Closo (`aipjhdapgmimfdfcjmlpeoopbdldcfke`) returns an empty store shell across five URL variants — both slugs, with and without `hl`, by bare id, and via the `chrome.google.com` redirect — and the store search for "closo" returns no results at all.
A second id carrying the same "Closo Bot" title, `mbepjmhihojingbamiffjnjfnnmfnkpm`, is likewise empty.
Closo's own get-started page still hands users the now-dead store URL.
Crosslister by Flyp (`kbflhgfmfbghhjafnjbgpiopcdjeajio`) behaves identically and appears in neither the "crosslist" nor the "flyp" store search, though a small unrelated "Flyp Orders Auto Refresh" does; joinflyp.com no longer mentions the Chrome Web Store, routing "Get it for Free" to a Typeform and login to `tools.joinflyp.com`.
The chrome-stats mirror still lists both as installable, which is what a mirror does when its crawl predates a delisting.
Nine other listings fetched with the same method on the same day rendered fully, so this is not a fetch artefact — but absence from the store is not by itself evidence of enforcement, and no removal notice was found for either.

F7 — An unresolved anomaly worth one sentence of caution.
Google removed every remaining Manifest V2 extension from the Chrome Web Store on 2026-08-31 (chromeunboxed.com, ghacks.net, androidauthority.com, read 2026-09-03).
The OneShop listing is still live three days later while reporting "Last updated December 12, 2021" and version "2", which predates the MV3 requirement for new submissions; either the store's displayed update date is not the manifest date, or the purge did not reach every listing.
Do not lean on OneShop's manifest version either way.

F8 — The platform, not the marketplaces, is what has actually forced this category to change.
PrimeLister rebuilt its extension against a Chrome deadline and used the occasion to move work off the device: "Google Chrome recently introduced a new extension development infrastructure and set a deadline of June 1", after which "Poshmark cloud automation operates entirely in the cloud, eliminating the need to keep your PC on", while "Crosslisting has become more stable" and stayed in the extension (docs.primelister.com, 2026-09-03).
That is the one durable split observed anywhere in the set: automation that is merely scheduled moves server-side, and the act of composing a marketplace write stays on the seller's device.

F9 — Firefox is empty, and the vendors say so themselves.
The addons.mozilla.org searches for "crosslist" and "poshmark" on 2026-09-03 return no product from Vendoo, List Perfectly, Crosslist, PrimeLister, Flyp, SellerAider, Closo or Nifty; the closest hits are hobby add-ons with single-digit user counts (PicFlip, 5 users; Poshmark Share OneClick, 5 users; SellyGenie, 8 users).
List Perfectly documents the boundary precisely: "a browser extension that works with Google Chrome and Microsoft Edge on desktop", where "Microsoft Edge Tablet is the only tablet browser that supports installing the List Perfectly Chrome extension" and it "cannot be installed on a mobile phone, including iPhone or Android" (listperfectly.com, 2026-09-03).
PrimeLister's and Vendoo's documentation reference Chrome only.

F10 — The extension is mandatory exactly where the marketplace is unsanctioned, and optional everywhere else.
Catalogue creation, editing, inventory and analytics work without it in every product that separates the two — List Perfectly states outright that the extension is not needed "to create, edit, organize, and manage listings from your phone".
Crosslist needs it only "when an API isn't available"; Closo needs it for Poshmark, Mercari, Depop and Vinted but not for OAuth-connected eBay and Shopify; OneShop's extension is a connection monitor and posts nothing.
The mandatory-optional line falls on precisely the sanctioned/unsanctioned boundary this repository's two-branch non-negotiable already draws.

F11 — One outlier is worth noting for contrast on disclosure.
Crosslist Tool (listifyx, 281 users) discloses collecting "Personally identifiable information", "Financial and payment information", "Authentication information", "Personal communications" and "User activity" (CWS Privacy tab, 2026-09-03).
Every established vendor discloses far less — Vendoo and OneShop declare they "will not collect or use your data", PrimeLister declares only "Website content", List Perfectly "Web history" and "Website content".
A narrow disclosure is the norm in this category, and a broad one is a small-vendor tell rather than a category requirement.

F12 — A correction to a premise in the brief.
OneShop is not "formerly Nifty".
OneShop's CWS listing gives its developer contact as `rlucia@invsys.co` (CWS, 2026-09-03), while Nifty is described by multiple 2026 sources as "formerly known as Auto Posher" and operates at nifty.ai.
Relatedly, one chrome-stats snapshot attributed "Crosslister by Flyp" to "Vendoo, Inc."; searches of PitchBook, Tracxn and CB Insights profiles on 2026-09-03 surfaced no Vendoo/Flyp acquisition, and 2026 comparison articles still treat them as rivals, so treat that attribution as an unverified mirror artefact.

## Education market

No tool was found, on the Chrome Web Store or elsewhere, that posts to TeachersPayTeachers or Tes on a seller's behalf.
This is a positive search result rather than an absence of searching, and the marketplaces themselves were not contacted.

The Chrome Web Store search for "teachers pay teachers" on 2026-09-03 returns ten extensions, and every one of them reads, scores or scrapes — none creates, uploads, publishes or cross-lists.
They are Grow TPT (2.2), TpT Informer (4.0), TPT Pro — Seller Analytics & SEO Tools (4.2), SEO Analyzer for TPT (seomantis.com, 3.7), Radar for TPT — Product & Niche Research (5.0), TPT Niche Keyword Finder (0.0), Scopetpt TPT Extractor (5.0), SellerSpy (sellerspy.co, 4.6), plus the unrelated Read&Write and TeacherTab.
The most established of them, TpT Informer (`chromewebstore.google.com/detail/tpt-informer/dlpnhinaaofkbonofnmagbkfgjnkpbok`), reports 1,000 users, 4.0 from 7 ratings, last updated 2025-04-27, version 3.2, 425KiB, developer `frgoe003`, and discloses that it "will not collect or use your data"; its own description is confined to sales tracking, notifications, product analysis and seller ranking (CWS, 2026-09-03).

The Chrome Web Store search for "tes teaching resources" on 2026-09-03 returns literally nothing: "It looks like there aren't any search results for your search."
Tes's own author documentation describes only its native five-step manual uploader and mentions no API, bulk path or third-party tool (tes.com author academy, read 2026-09-03).

The nearest thing to a cross-lister in this market is not an extension.
A "TPT Seller SEO Listing Optimizer App" sold on TPT itself advertises a bulk product importer, but it imports a seller's own TPT catalogue inward for SEO rewriting rather than publishing outward to another marketplace (teacherspayteachers.com product page, read 2026-09-03).
Sellers who do sell across TPT, Etsy, Made By Teachers and Classful describe manual re-upload to each platform, with Classful offering a one-time bulk import at signup and nothing after that (seller blogs and comparison posts, read 2026-09-03).
Bulk-upload extensions in this adjacent space are built for Etsy, not for any education marketplace.

So the education market has analytics extensions and no actuator extensions, which is the opposite of the reseller market's shape.
No incumbent occupies the position this project is building toward, and no incumbent has therefore established either a precedent or a hostile reaction to defend against.

## Sources

Chrome Web Store listing and search pages, all fetched 2026-09-03:
`chromewebstore.google.com/detail/vendoo-crosslist-extensio/mnampbajndaipakjhcbbaihllmghlcdf`,
`chromewebstore.google.com/detail/flpmljgbaphneikdjhmekdpiamkejfon`,
`chromewebstore.google.com/detail/crosslist/knfhdmkccnbhbgpahakkcmoddgikegjl`,
`chromewebstore.google.com/detail/cross-listing-poshmark-to/eepbhjeldlodgnndnjofcnnoampciipg`,
`chromewebstore.google.com/detail/oneshop-sell-on-marketpla/pcapaniacmdmabfopeeimmpjkkjpeiok`,
`chromewebstore.google.com/detail/crosslister-selleraider/hoadkegncldcimofoogeljainpjpblpk`,
`chromewebstore.google.com/detail/crosslist-tool-crosslisti/enohabfnmaecamadfmihenkkonedakek`,
`chromewebstore.google.com/detail/tpt-informer/dlpnhinaaofkbonofnmagbkfgjnkpbok`,
`chromewebstore.google.com/search/crosslist`, `/search/closo`, `/search/flyp`, `/search/teachers%20pay%20teachers`, `/search/tes%20teaching%20resources`,
and the empty-shell responses for `aipjhdapgmimfdfcjmlpeoopbdldcfke`, `mbepjmhihojingbamiffjnjfnnmfnkpm` and `kbflhgfmfbghhjafnjbgpiopcdjeajio`.

Vendor documentation, all fetched 2026-09-03:
`help.vendoo.co/en/articles/6260267-...`,
`listperfectly.com/selling/what-is-a-google-chrome-extension/`,
`crosslist.com/`,
`docs.primelister.com/` and `docs.primelister.com/faq/primelister-extension-v2-upgrade`,
`closo.co/blogs/begin-here/get-started-with-closo-...`,
`joinflyp.com/`,
`tes.com/author-academy/...`.

Third-party and press, all read 2026-09-03:
`chrome-stats.com/d/{mnampbajndaipakjhcbbaihllmghlcdf, knfhdmkccnbhbgpahakkcmoddgikegjl, flpmljgbaphneikdjhmekdpiamkejfon, eepbhjeldlodgnndnjofcnnoampciipg, kbflhgfmfbghhjafnjbgpiopcdjeajio, aipjhdapgmimfdfcjmlpeoopbdldcfke}` (via search; direct fetch 403),
`extpose.com/ext/mnampbajndaipakjhcbbaihllmghlcdf`,
`addons.mozilla.org/en-US/firefox/search/?q=crosslist` and `?q=poshmark`,
`community.ebay.com/t5/Selling/Third-Party-Crosslisters-Like-Vendoo-FLYP-cant-tranfer-listings/td-p/35034415`,
`valueaddedresource.net` (Poshmark listing-removal policy and reversal, November 2025 deletion incident),
`blog.vendoo.co/vendoo-product-update-for-april-2026`,
and the Manifest V2 purge coverage at `chromeunboxed.com`, `ghacks.net` and `androidauthority.com`.
