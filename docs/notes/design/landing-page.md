# The teachouse.io landing page

The public site a teacher-seller reaches before signing up.

- date: 2026-09-03
- status: built and green under `just landing-check`, which runs inside `just pre-push`; rewritten 2026-09-05 to carry the founder's approved prices at `/` and at a new `/pricing`, and re-based on `tam-server` serving the build rather than on a static host of its own; rebuilt 2026-09-11 to the founder's mockup, on the brand kit and the pricing recorded in `brand-kit-and-teacher-ui.md` and `decisions.md` under that date, which supersede the copy, tokens, pricing and the no-marketplace-names rule described below wherever the two disagree; amended 2026-09-12 with the Resource Atelier imagery, two more import rungs, the three-year cap on the Founding discount and the AI "coming soon" placements, recorded under "Amended 2026-09-12" below
- placeholders: the legal text and the support address are what the founder must still replace, and the desktop download URL is still null
- paths: `apps/landing/`, `nix/landing.nix`, and the `landing-check` and `landing-dev` recipes in the justfile

## What it is, and why it is a separate build

D28 names "Astro or a prerendered SvelteKit route" for this page, and the research it rests on names Astro without the disjunction.
`docs/research/rethink/tanstack-and-astro-fit.md` recommends, twice, "a separate Astro 7 site on Cloudflare Pages rather than as a route inside a SPA whose root layout sets `ssr = false`", and it offers the prerendered SvelteKit route only as the smaller answer if the founder is certain there will never be a blog or a help-content programme.
The SvelteKit alternative also lands inside `web/`, which this work does not touch.
The recommendation's "separate Astro 7 site" half is what we took; its Cloudflare Pages half is not, for the reason the deployment section below gives.
Astro 7.2.10 it is, pinned exactly.
The site ships one script of its own and nothing else: no framework island, no analytics, no third-party script, and no request to any host but the one serving it.

The reason for keeping it out of the console is the console's own shape.
`web/src/routes/+layout.ts` turns off both server rendering and prerendering, which SvelteKit's documentation calls a large negative for performance and search.
That is the right trade for a logged-in dashboard and the wrong one for the page that has to rank and convert.

The whole home page is 12 KB of HTML and 9 KB of CSS, plus 360 bytes of script and two self-hosted font files.

## Structure

The site is four pages: `/`, `/pricing`, `/privacy` and `/terms`.

`/` runs in heyretro's order with its two content-marketing blocks dropped, since we have no template library to promote.
A sticky header, a hero, four feature blocks, how it works in three steps, pricing, migrations, questions, a closing call to action, then the footer.
The header carries three anchors into the home page — how it works, migrations, questions — with pricing as a page link, and the two buttons on the right.
Both buttons go to `/login`, which the landing build holds no file for and which therefore falls through to the console.

The hero is a badge, a one-line headline, a two-sentence subhead naming the mechanism, two buttons, and one reassurance line underneath: "Your resource files never pass through us."
An earlier draft said "never leave your computer", which is false: a resource file is uploaded to TPT or TES, and the upload is one of the requests the seller's own machine sends.
The four feature blocks are the four things the product does — cross-list to TPT and TES from one catalogue, change a listing once and have it change everywhere, move a whole shop in one go, and run the marketplace work on the seller's own computer — each a heading, one or two sentences, and a small label beneath.
How it works is three steps, connect, map and sync, followed by the sentence naming the app that TPT and TES work needs.
The closing call to action repeats the hero's button with the free tier's terms under it, and appears on `/` only.

`/pricing` is the pricing, migrations and questions sections and nothing else.
It is the same three components the home page renders, so the two pages cannot disagree about a price.

## Copy decisions

Amended 2026-09-06, by founder reversal: the site's selling copy names no marketplace at all, and the marks of every marketplace the console catalogues appear under the hero.
Both paragraphs below are what the site was until then, and are kept because they record the reasoning the reversal was taken against rather than in ignorance of.
What replaced the first is one sentence: `availability` in `src/site.js` reads "TPT and TES connections work today, with more marketplaces coming.", it is the only sentence on the site that names a marketplace, and `just landing-copy-gate` fails the build if a second one appears.
Every other sentence speaks of the marketplaces a seller sells in, without naming one and without implying a count, so the copy stays true as marketplaces are added rather than needing a rewrite per marketplace.
What replaced the second is the band under the hero, described under "The marketplace band" below.

TPT and TES are written in capitals throughout, and the two marketplaces are the only ones named.
Etsy is not on the site: it is a later branch under D1, and the research advice was that it belongs in a "coming" line at most, never in a feature block or a tier's marketplace list.

Marketplaces are named in words only.
There is no marketplace logo anywhere on the site, because a logo on a marketing page reads as an endorsement, and the footer states plainly that Teachouse is independent and endorsed by neither.

There are no testimonials, no seller counts, no time-saved figures and no comparison table.
Nothing on the site is a number that has not been measured, which for a product with no public sellers means the only numbers are prices.

The device claim is made positively and never as an absolute.
The site says that TPT and TES publish no interface for tools like this one, so every request to them is sent from the seller's own machine under their own login, and that resource files never pass through us — which is D1's no-API branch and D27.
No sentence claims a legal requirement, a compliance obligation, or a marketplace's approval, which is what D30 forbids.
The block says so affirmatively as well: "This split is our design choice about where a request should come from. It is not required by any law, and we do not present it as one."
That wording dropped out with the device section the 2026-09-04 site carried, and was restored on 2026-09-05 into the fourth feature block, which is where the device claim now lives.

Every answer in "Questions" is checked against the code rather than written from the design notes.
The claim that a change edits the listing already there rather than deleting and recreating it rests on the connector contract's `revise`, which addresses an existing `RemoteListingId`, and on the absence of any delist-and-relist path anywhere in `crates/`.
The migration answer promises only what the founder approved: a fixed band price quoted before work starts, and 30 days of Studio to review the mappings.
The re-runs-of-failed-items promise the research proposed is not on the page, because it was not among the terms the founder approved.

The site says that the work for a marketplace publishing no interface needs a small app, installed once.
It is said in "How it works" rather than buried, because a seller who learns it after signing up learns it as a surprise, and because the device story is not credible without it.
The download link is driven from one value that is null today, so the sentence stands and the link reads "Download link to come" rather than pointing at nothing.

## The marketplace band

A row of marks sits under the hero's buttons, one per marketplace the console's `web/src/lib/pages/marketplaces/catalogue.ts` tiles, in that file's order.
`src/marketplaces.js` is a transcription of it rather than an import, and `public/marks/` holds copies of the files rather than links, for the reason `public/favicon.svg` is a copy of the console's mark: the two trees build separately, so a path resolved by the console's `ServeDir` fallthrough under `tam-server` would 404 under `just landing-dev`, and the dev render would disagree with the production one on exactly the thing this band adds.

The directory is `marks/` rather than `marketplaces/`, which is what the same files are called under `web/static/`.
This build is answered ahead of the console and the server matches files rather than directories, so a landing directory sharing a name with a console route is harmless only while this build holds no page at that path.
A later `marketplaces.astro` would then answer `/marketplaces` with a marketing page and take a seller's Marketplaces screen away, with nothing failing to say so.
The rule this site works under is that no top-level entry it writes may match a console route, and the routes are the `href` values in `web/src/lib/nav.ts`.

One of the twenty-one draws an initial and its name rather than a logo, and it is Boom Learning.
Its guidelines make "Boom™ is the trademark of Boom Learning. Used with permission." mandatory wherever its mark appears, and we hold no permission, so drawing the mark means either breaching the rule that governs it or printing a sentence that is untrue.
Etsy and Shopify drew an initial too until 2026-09-06, when the founder reversed that: both owners require written permission, neither has given it, and the founder took the same accepted risk here that was taken for the console page, having read the same policies.
Boom Learning is different in kind rather than a smaller version of it, which is why it stayed.

The marks draw in their owners' own colours at all times, on every page load, by the founder's decision of 2026-09-06.
They were greyscale at rest and coloured on hover until then, on the reasoning that twenty brand palettes at full strength fight the page and each other; a reader on a phone cannot produce a hover state at all, which is how the founder saw a page of grey marks.
The reversal is also the safer reading of the rule the greyscale paragraph had already accepted: a CSS filter changes the drawing rather than the file, and a rights holder could read a greyscale rendering as an alteration of their mark, so drawing the published bytes unaltered removes that reading instead of accepting it.
Nothing keeps this by memory: `landing-band.test.ts` fails if `site.css` contains `grayscale(` at all.
A link still answers a pointer, on the pill rather than on the image — `background: var(--hover)` on `.mark:hover` — so no CSS property reaches the owner's mark.
Each mark links to that marketplace's own front page, as every console tile does.

The strip's classes are `marks-wrap`, `marks` and `marks-note`, and they were `band-*` until 2026-09-06.
`.band` is the migration table's class and had been since the site was built, so the strip's own `.band` was a second top-level declaration of the same selector forty-two lines later in one stylesheet: same specificity, later wins, and every migration row lost its two-column layout to the strip's centring.
`just landing-style-gate`, run by `just landing-check` and so by `just pre-push`, now fails the build on any selector declared twice at the top level of `site.css`; selectors inside an at-rule are ignored, because redeclaring one at a breakpoint is what a media query is for.
It does not run in `nix flake check`, which is the same lane gap wave 4's landing gate recorded for `RESERVED_SLUGS`.

## The platform row

Under the sentence in "How it works" that says some marketplace work needs a small app, a row names the platforms a build is published for: Windows and Android.
The sentence alone leaves a reader on a phone unable to tell whether their phone is one of them, and a row omitting Windows would say we do not support it.
It is not a download page; `site.js` still owns the link.

Android draws Google's own robot, in colour, and its name carries the trademark symbol: `Android™`.
Google's brand guidelines license the robot under Creative Commons 3.0 Attribution and the caption under the row carries the attribution line verbatim, which is the condition of that grant; the same page asks that the name carry the symbol at its first appearance in a creative, and this row is a creative of its own.
Both of Google's name conditions are therefore met here, which they were not when the row first landed.
Windows draws its name and no symbol, because Microsoft requires an express trademark licence for the Windows symbol.
Nothing has been requested from Microsoft: the letter quoting Microsoft's own exception for a product called out next to other platforms is drafted in `marketplace-logo-sources.md` for the founder to send, and on a licence the row changes by one field.
Apple is deliberately absent, because no macOS build is published; `apps/landing/src/platforms.js` records that absence rather than leaving it unsaid, and `landing-band.test.ts` holds the row plus its recorded omissions against the console's own `PLATFORM_ORDER`, so a platform can neither appear here without a build nor vanish from here without a reason.
No store badge appears on the row or anywhere else on the site: all three badge programmes license the badge to link to a store listing, and Teachouse has none.

The caption under the band carries `availability` itself rather than a second wording of it, then says that the others are places teachers told us they sell and none of them is connected yet, then the footer's independence line.
That caption is the only thing standing between a band of twenty-one marks over marketplace-agnostic copy and a claim of twenty-one supported marketplaces, which is a claim no measurement supports and which the standing rule against unmeasured numbers forbids.

## Pricing and migrations

The prices are the founder's, approved 2026-09-05, in USD only.
USD because TPT is a US marketplace and TES is UK-centred, so NZD is neither buyer's currency and quoting it puts an FX conversion in front of a small ticket.

Four tiers: Free, then Solo at $12 a month or $120 a year, Studio at $24 or $240, and Publisher at $48 or $480.
Free is one marketplace, 20 resources and manual sync.
The paid tiers differ on resources kept in sync (100, 400, unlimited), marketplaces (2, all, all), sync frequency (daily, every six hours, hourly), devices (1, 2, 3) and the migration allowance (50, 200, 500 resources migrated a year).
The allowance is worded "50 resources migrated a year" rather than "50 migrations", because the unit is resources and the shorter phrasing reads as a count of jobs.

Migrations are five fixed bands — $49, $79, $129, $199, and $299 for the first 500 resources plus $0.25 for each resource beyond 500 — quoted before work starts.
The founder ruled on that last band on 2026-09-05, because "501+ $299 plus $0.25 per resource" also reads as $0.25 on every resource, and the two readings differ by $125 at 600 resources.
The first band is labelled "50 resources or fewer" rather than the approved "0-50", because nobody orders a migration of no resources.
Three rules combine them with the tiers: every migration includes 30 days of Studio, a subscriber's yearly allowance is consumed first and anything past it is half the band price, and an annual plan bought within 30 days credits the migration price in full.

The block follows heyretro's structure, which is where the shape comes from and not the numbers.
The heading names the metering unit, the trial is stated once above the cards rather than repeated inside each, exactly one card is badged, and the top tier gets a full-width panel of its own below the row as well as its card.
The trial line reads "14-day Studio trial." and no more: which paid plans it attaches to is not something the founder has said, and a Studio trial sold beside Publisher would be a downgrade rather than a trial.
heyretro's monthly-and-yearly toggle is not built, because it needs JavaScript; each card carries its annual price as the caption under its monthly one instead.

## Amended 2026-09-12: the imagery, the ladder, the Founding cap and AI

Four changes, all from the design of record at `2026-09-12-one-marketplace-per-site-and-the-seller-workflows.md`, sections 11 and 12, and the decisions of that date.

**The imagery is the Resource Atelier, and it is drawn rather than photographed.**
The three `-placeholder.webp` files the founder owed are gone: the hero and the challenge are inline SVG compositions, and the solution is a real screenshot of the console's own Resources board.
The hero draws a cream "Your catalogue" card carrying the house mark and three resource covers — fraction circles, ruled reading lines, checklist squares — with a peach card behind one and a lavender behind another, and four thin teal lines routing out to tiles carrying the four featured marks, then "+ more".
There is no person, no laptop and no script face anywhere on the page; the handwritten line and the system cursive fallback it was set in are both gone.
The challenge draws one finished cover with a teal tick, then the same title typed again on a listing sheet per marketplace, gathered under a peach bracket reading "Same resource. Repeated admin.".
`ResourceCover.astro` draws a cover, `MarkTile.astro` draws a tile, and `src/art.js` holds the arithmetic both share, so a box and the mark inside it are computed once.

Two things make the compositions safe to keep.
Every colour in them is a token or `currentColor`, which is what lets them read on the dark ground the same sheet now carries, and `tokens.test.ts` fails the lane on a colour literal in any `.astro` file here.
The four marks are `<image>` references to the owners' own published bytes under `public/marks/`, never a redrawing, which is the rule the strip under the hero already worked under; the compositions sit inside `data-marketplace-band` wrappers, so `just landing-copy-gate` strips them whole.

The hero ships as two drawings rather than one that reflows.
Under 720px the wide composition's three covers and four-high tile column have no room, so `.atelier-phone` — one cover, the card, a 2x2 grid of tiles — replaces it, and the compositions are inline so this site's own `@font-face` faces set their display text and no glyph is outlined by hand.

**The screenshot is real, and it is reproducible.**
`public/images/console-resources.webp` and its 390-wide crop are captures of `/resources` running against the ephemeral Postgres with three seeded resources named Fractions practice, Reading response and Classroom routines, which are the three the covers above draw.
They are framed in an indigo-outlined panel under the house mark, captioned "Your resources, together.", rather than bled into the band, because a screenshot with no edge reads as part of the page rather than as the product.

**The import ladder gains two rungs and says what it counts.**
`importLadder` is now a list of `{ upTo, price }` rather than two display strings per rung, so a page phrases a rung its own way and no page can quote a band the file does not hold.
`$397` up to 500 joins the four the founder set, because the measured dual-lister holds about 764 listings, and a last rung carries no figure at all: it reads "Talk to us", its action reads "Ask us", and it links the support address where `site.supportEmail` is set and `/pricing/#founding` where it is null, which is the same rule the footer's missing address already follows.
Under the ladder, in both pages, "Counted as resources added to your catalogue after duplicates are merged."

**Import is included, the Founding discount is capped, and AI is sold as coming soon.**
`subscription.includesImport` is true and the card lists "Import included".
`founding.ongoingYears` is 3 and the card reads "20% off for 3 years after" in place of "20% off ongoing".
`subscription.ai` carries the packaging the founder adopted — `{ status: 'coming-soon', includedFills: 200, addOn: { fills: 100, price: 5 } }` — and the card's line reads "AI fill — coming soon (200 a month)" against a hollow grey mark rather than a tick, because a tick beside a line that is not built yet claims it is.
Two questions join the FAQ: "Do I pay for import?", which says the subscription includes it and the one-off price is for a seller who does not subscribe, and "What does AI fill do?", which promises that it fills the form from the seller's own file for them to check, that it never writes to a marketplace on its own, and no date and no accuracy figure.

## Amended 2026-09-26: the founder's landing feedback and the charcoal theme

The site opens light whatever the visitor's machine prefers, as the console does; only a stored `system` follows the machine, and `web/src/lib/theme.test.ts` runs the script in `Base.astro` against the same stub store as the console's own.
Dark is re-grounded on neutral charcoal greys (page `#141414`, card `#1C1C1E`) instead of Indigo, which read as blue everywhere; the Challenge band is graphite rather than brown, and the kit's Peach survives on dark as a muted `#E3A878` in the quote's rule, the bracket and the cover backing.
The drawn covers' cream becomes the hover grey on dark, because any Peach mixed into charcoal reads brown.

Import, Distribute and Manage are a five-column grid: three centred columns with 72px icon tiles and 36px arrows centred on the tiles' middle line, one row down to a phone.
The platform row's licence lines stay, because the Android robot's Creative Commons grant makes its attribution line a condition (`marketplace-logo-sources.md`), but as 11px fine print in `--faint`.
"Three ways to start" is followed by a definition of a move; it names Tes and TPT as the founder's example does, so the line carries `data-marketplace-band`.
The plan cards read "(Trial)", "(Subscription)" and "Move Packs (One-Off)", and their lines are the founder's wording, matched line for line by the console's plan page: "Import all your resources from wherever you sell" for "Import included", "5 moves onto a marketplace of your choice" for the per-shop free moves, "Statistics on every shop" for "Figures on every shop", and a new Sync line, "Edit resources in Teachouse and sync the edits across all platforms"; "No limit on resources" and "Pulls every 6 hours" are gone. Every FAQ answer was rewritten in a teacher's words.

The screenshot was retaken from production `/resources` at 1440×900 and 390×844, light theme, at 2x. Both captures stop above the first resource card, because production still showed the placeholder covers the pure-Rust page rasteriser replaces; retake the full list once real covers are live.

## Where the code holds each decision

Amended 2026-09-12, phase 1: the prices are no longer the landing's own.
`apps/landing/src/plans.generated.js` is emitted by `cargo run -p tam-typegen` from the `tam-limits` plan table, the same table the server enforces a quota against and `GET /v1/plans` serves, and it carries the four plans with their `capabilities`, the import ladder in cents, the Founding overlay and the AI packaging.
`apps/landing/src/pricing.js` now holds only what the landing alone owns — the phrasing of a rung, dollar formatting from cents, the sentence under the ladder, the chips the subscription card derives from `capabilities`, and the questions — and `/` and `/pricing` read the figures through it, so the two pages still cannot disagree with each other and can no longer disagree with the server either.
`just web-check` diffs the emitted file against the tree and fails when a number moved in Rust without the landing being regenerated, which is the gate; the landing lane has no price check of its own, and changing a price is an edit in `tam-limits` and nowhere else.
`studio` is in the table with `sold: false` and is rendered nowhere on this site.

`apps/landing/src/site.js` holds what the founder must supply and nothing else: the login path, the support address, the desktop download URL and the availability sentence.
`downloadUrl` is null, and a null renders as "Download link to come" rather than as a link, so no broken download can ship by being forgotten.
The login path is `/login` rather than an origin, because the console is served from this same origin, which is also what keeps the site working under `default-src 'self'`.

`apps/landing/public/app-redirect.js` is the site's only script, loaded from our own origin on every page.
The desktop app opens this origin too and has no use for a marketing page, and `window.__TAURI__` is the one signal available before the console loads, so the script sends that window to `/app`.
It is an external file rather than an inline block so that the policy needs no hash for it.

`apps/landing/src/styles/site.css` carries "Pounamu", from the console design spec, under the same token names and values the console uses in `web/src/lib/styles/tokens.css`, so the two surfaces read as one product.
It replaced "Kauri" on 2026-09-06, and the realignment that change carried is recorded rather than the drift it fixed: `--accent-deep`, `--hover`, `--muted`, `--ok` and `--warn` had each drifted by a shade under Kauri and all five now take the console's value, `#0f6b54`, `#eef1ee`, `#5a6560`, `#1f6a45` and `#7a5410`.
`--muted-strong` is gone with the same change, because it existed only to carry text on `--rail` where `--muted` failed AA at 4.24, and `--muted` measures 5.17 there under Pounamu.
Nothing keeps the two sheets in step by hand any longer: `web/src/lib/styles/tokens.test.ts` fails the web lane if a token both sheets declare stops agreeing, and the one deliberate difference, `--r-card`, is named there with its reason.
The shape language is heyretro's: fully round buttons and badges, 2rem section cards, one very diffuse shadow with a hairline inset ring instead of a border, and headings semibold with tightened tracking and a balanced wrap.
The component classes are this site's own, because the console's are dashboard furniture.

The two fonts are self-hosted rather than fetched from Google's CDN, which is what the console does.
The latin subsets and both SIL Open Font License texts are in `apps/landing/public/fonts/`, with a note recording where each file came from and how to refresh it.
The page therefore makes no third-party request at all, and the server's policy no longer permits one: `style-src` and `font-src` are both `'self'` since the console's fonts were bundled and the Google origins were dropped, so self-hosting is now what the policy requires rather than a precaution ahead of it.

The favicon is `apps/landing/public/favicon.svg`, a copy of the product mark at `web/static/email/teachouse-mark.svg`.
It is a copy rather than a reference because the two trees build separately; if the mark changes, this copy changes with it.
The header and footer wordmarks draw the same file as an `<img>` beside the word "Teachouse", with an empty `alt` because the word beside it already names the product.

## Placeholders the founder must replace

Three items.

1. `supportEmail` in `src/site.js`, which is null. Null renders no address at all rather than a `mailto:` that reaches nobody, so the footer drops the link and both legal pages say a contact address is still to come. `hello@teachouse.io` stood here until 2026-09-05 and was never monitored.
2. The whole of `/privacy`, which is a placeholder for counsel and not a policy.
3. The whole of `/terms`, which is a placeholder for counsel and not terms.

`downloadUrl` is not in this list, because null is a working state rather than a wrong value: the sentence stands without it.
No public download page URL exists yet, since releases go to CrabNebula Cloud on the `beta` channel and a channelled release is not listed on an application's public page, while the GitHub releases beside them are in a private repository.

## The two legal pages

Neither page contains invented legal text, and both say so at the top in a banner, with a "Draft placeholder" pill above the heading.

Each page instead does something useful for the founder's counsel: it sets down, as briefing material, the facts about the product that a drafter would otherwise have to be told, and then lists the questions the real document must answer.
The privacy page records what identity data an account carries, that the no-API marketplace session and the resource files stay on the seller's device, that an official-API token is held server-side, what the catalogue and the analytics series contain, that a third-party merchant of record takes the payment, and that the site itself carries no analytics.
The terms page records the independence from every marketplace, that the seller keeps their own relationship with each one, where each kind of request originates, the metering shape the published prices use, that a marketplace's own pricing rule is enforced as a publish gate rather than a warning, and that a decision the marketplace makes the seller's — a tax designation, a copyright assertion — is never filled in on the seller's behalf.

These pages must be replaced in full, not edited.

## Deployment

The site is not deployed to a static host of its own, and no Cloudflare account has been touched.
`tam-server` serves it, from the directory named by `--landing-dir`.

`nix/landing.nix` builds `apps/landing` into a store path holding the `dist` tree, exposed as the flake package `teachouse-landing` and as the flake check `landing`.
The NixOS module passes that package as `services.teachouse.landingPackage`, and the unit passes its path to `tam-server --landing-dir`.
`serving::Landing::load` reads the whole directory into memory once at start-up and refuses a directory with no `index.html`, so a broken or empty build fails the process rather than serving a 404 at the root.

Three tiers answer a request, in order, and `serving::route` decides between them.
A path whose first segment parses as an API version goes to the API.
A path the landing build holds a file for is answered from memory, resolving a directory route through its own `index.html`, which is what makes `/pricing`, `/privacy` and `/terms` work without the Astro build emitting extensionless files.
Everything else is the console's, including `/app` and every route below it, and including `/login`, which is where both of the landing page's buttons go.

The landing page's Content-Security-Policy is computed by `landing_policy` from the files just read rather than written down twice: `default-src 'self'`, `script-src 'self'` plus a `sha256-` token for each inline script found in the build, `style-src 'self'`, `font-src 'self'`, `img-src 'self' data:`, `connect-src 'self'` and `frame-ancestors 'none'`.
It admitted `'unsafe-inline'` and `https://fonts.googleapis.com` on `style-src` and `https://fonts.gstatic.com` on `font-src` until the console's fonts were bundled and those origins were dropped; every origin the policy names is now this one.
The site carries no inline script, so no hash is emitted today.

The `site` value in `apps/landing/astro.config.mjs` is `https://teachouse.io`; the canonical link and Open Graph URL are built from it.

## Open items, each a founder decision

The site states where the marketplace work runs as what Teachouse is built to do, not as an absolute.
No sentence claims that our servers never hold a marketplace login or never open a marketplace session, and no sentence says the desktop app is the only thing that reaches a marketplace, because both would be false today.
Two paths run at once, for different traffic.
On the device, `apps/desktop/src-tauri/src/work.rs` composes and issues marketplace requests on the seller's machine, under the session the login webview filed in the operating system's keychain.
On our side, two binaries still reach a no-API marketplace under a seller's session: `crates/tam-canary` builds a Tes adapter over a direct transport from a cookie-jar path, and `crates/tam-import` builds a Tpt one over `TAM_TPT_COOKIE_JAR` for the operator manifest drain.
Corrected 2026-09-04: `crates/tam-sync-worker` is no longer one of them and the gateway is no longer the shape, because its Tes read leg moved to the seller's device under D1, leaving the crate the enqueue half with no marketplace edge and no poll loop.
So the positive claim is true, and neither the absolute nor an exclusivity claim is.
The absolute wording, including D30's "your login never leaves your device", returns only when no server-side path reaches a marketplace under a seller's session, and this paragraph is the reminder so nobody has to hold it in their head.
Check the claim against the tree rather than against this paragraph, and check it wider than the gateway: the question is which processes construct a marketplace adapter for a no-API marketplace at all, whether the session arrives over a broker lease or out of a cookie jar on our own disk.

The published price list meters differently from D4.
D4 says meter connected marketplaces with a catalogue cap on the entry tier; the prices the founder approved on 2026-09-05 meter resources kept in sync at every tier, name that unit in the pricing heading, and carry marketplaces as a second axis.
The later decision governs what shipped, and D4's row in `docs/notes/design/vendoo-for-teachers-rethink.md` has not been amended to match.

The `/terms` placeholder briefly contradicted `/pricing` about metering, and no longer does.
It described the D4 shape to counsel, as "subscription is metered by connected marketplaces, with a cap on catalogue size at the entry tier", while the price list on the same site meters resources; the sentence was corrected on 2026-09-05 to name resources kept in sync with marketplaces as a second axis per tier.
That is the one edit made to a page whose rule is replacement rather than editing, because two live pages disagreeing about what a seller is billed for is worse than the exception; counsel's replacement must not reintroduce the old shape.

The site publishes no way to reach us.
`supportEmail` is null, which is the right state while no monitored address exists, but a page that takes money with no contact route is not a state to launch in; one founder value closes it.

There is no monthly-and-yearly toggle.
heyretro has one and it needs JavaScript, and the site's one script is the desktop redirect; each card carries its annual price as a caption instead, which loses the comparison heyretro's toggle gives but costs no script.
A toggle becomes reasonable if the site ever takes a second script for another reason.

No marketplace logo appears on the site, and none should be added.
A logo on a marketing page reads as an endorsement, our own footer says we have none, and the trade-name line beside it is doing the work a logo would undo.
Reversed 2026-09-06 by founder decision: the band described under "The marketplace band" ships, with the disclaimer caption and the per-mark link to the owner as the mitigation, which is the console's arrangement moved onto a public page.
The reasoning above is not weakened by anything found since, so what remains open is the exposure rather than the decision: twenty-one marks on a marketing page is the most trademark-exposed surface on the site, and approaching Etsy and Shopify sits at the top of the founder's list.

Body text set in `--muted` does not clear the WCAG AA contrast minimum for normal text.
Corrected 2026-09-05: an earlier version of this paragraph said `--muted` was 3.90:1 on `--card` and 3.65:1 on `--ground` and therefore failed, and both figures were wrong.
Measured under WCAG 2.x relative luminance, Kauri's `#7c6b5b` was 5.03:1 on `#fffdf9` and 4.58:1 on `#f7f2e9`, so every place `--muted` sat on a card or on the ground passed AA for normal text; under Pounamu the same three tokens are `#5a6560`, `#fdfdfc` and `#f6f4f1`, measuring 5.95:1 and 5.52:1.
The arithmetic was checked against the two published reference pairs, `#767676` on white at 4.54:1 and `#595959` on white at 7.00:1, and reproduces both exactly.

Two combinations did fail, and the wrong paragraph above hid them; both were fixed, and both fixes were then retired by the palette change.
`--muted` on `--rail` was 4.24:1, which is the Publisher panel's lead line and body copy, the largest block of selling copy in the pricing section and present on both pages; `--muted-strong` was added to carry that text and is gone as of 2026-09-06, because `--muted` measures 5.17:1 on `--rail` under Pounamu and a token whose only reason was a failure that no longer happens is a second source of truth for nothing.
`--warn` on `--warn-soft` was 3.37:1, which was the "Draft placeholder" chip on both legal pages, the one element there whose whole job is to be noticed; it took a `color-mix` toward the ink to reach 4.63:1, and takes the plain `--warn` again as of 2026-09-06, which measures 5.86:1 on `--warn-soft`.
The `--faint` token is gone from this stylesheet: at 2.28:1 it was carrying the disclaimer lines, and a disclosure nobody can read is not a disclosure.

Neither of those failures could recur unnoticed now.
`web/src/lib/styles/tokens.test.ts` measures every ink against every ground either sheet paints, and fails the web lane below 4.5:1, so the hand measurement that found these two is no longer the thing standing between the site and an unreadable line.

There is no type-check step, only the build.
`astro check` would require `@astrojs/check` and `typescript` as dependencies, and the standing instruction is to report a dependency beyond the framework itself rather than add it; the build already fails on a template or import error, and the site has no application logic for a type checker to find a fault in.

D18 requires the NGSS disclaimer in the footer wherever the mark is used nominatively.
The landing page names no standards framework at all, so the disclaimer is not triggered here, and the obligation stays with the console.
If a future page describes standards alignment by name, the disclaimer comes with it.

There is no blog and no help centre yet.
Astro's content collections are the reason this site is Astro rather than a prerendered route, and adding the first one is a day's work when the founder wants it.
