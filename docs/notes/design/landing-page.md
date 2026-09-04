# The teachouse.io landing page

The public site a teacher-seller reaches before signing up.

- date: 2026-09-03
- status: built and building green under `just landing-check`, which now runs inside `just pre-push`; amended 2026-09-04 to remove the pricing section and stand a waitlist in its place, by founder decision to park payments
- placeholders: the legal text, the support and waitlist addresses, the console origin and the desktop download URL are all still placeholders the founder must replace before the site goes live
- paths: `apps/landing/`, and the `landing-check` and `landing-dev` recipes in the justfile

## What it is, and why it is a separate build

D28 names "Astro or a prerendered SvelteKit route" for this page, and the research it rests on names Astro without the disjunction.
`docs/research/rethink/tanstack-and-astro-fit.md` recommends, twice, "a separate Astro 7 site on Cloudflare Pages rather than as a route inside a SPA whose root layout sets `ssr = false`", and it offers the prerendered SvelteKit route only as the smaller answer if the founder is certain there will never be a blog or a help-content programme.
The SvelteKit alternative also lands inside `web/`, which this work does not touch.
Astro 7.2.10 it is, pinned exactly, and the site ships no JavaScript at all: no framework island, no analytics, no third-party script.

The reason for keeping it out of the console is the console's own shape.
`web/src/routes/+layout.ts` turns off both server rendering and prerendering, which SvelteKit's documentation calls a large negative for performance and search.
That is the right trade for a logged-in dashboard and the wrong one for the page that has to rank and convert.

The whole home page is 10 KB of HTML and 7 KB of CSS, plus two self-hosted font files.

## Structure

The site is three pages.

The home page carries a hero, then five sections that the header and footer link to by anchor: how it works, what you will be looking at, where the work happens, marketplaces, and questions, closing on the waitlist.
The hero states in one sentence what the product does and where the work runs, asks to be told when it opens, and prints the availability sentence underneath so no visitor infers a general release from a marketing page.
"How it works" is three steps — bring the catalogue in, map it once, publish and keep it matching — followed by the sentence naming the Windows app that TeachersPayTeachers and Tes work needs, and the download link beside it.
"What you will be looking at" is three placeholder frames captioned with the screen each will hold; they carry no image, and the caption says the pictures go in when there is a real catalogue to photograph.
"Where the work happens" is the D1 and D30 section, and is described on its own below.
"Marketplaces" is a row per marketplace naming where its work runs and what its status is, driven from the same transport split the registry encodes.
"Questions" is six question-and-answer pairs, rendered open rather than collapsed, because this is the page that has to rank and a collapsed answer is worth less to a search engine than an open one.
The closing section is the waitlist, and it is the only call to action on the page.

`/privacy` and `/terms` are placeholder pages, described below.

## Copy decisions

The one-sentence claim is that Teachouse holds each teaching resource once and keeps the marketplaces matching it, with the marketplace work running from the seller's own computer under their own login.
It leads with the catalogue rather than with cross-listing because the catalogue is the thing the seller does not have today, and the research finds the seller-side inventory shape vacant in the teacher space while the marketplace-side importer shape is occupied.

D30 supplies the heading of the device section verbatim: "your login never leaves your device".
The section then says what our servers do not do — never hold the marketplace password, never open a marketplace session on the seller's behalf — and what they do hold, which is the catalogue, the mapping decisions and the record of what was done.
It closes with the sentence D30 requires: this split is our design choice about where a request should come from, it is not required by any law, and we do not present it as one.
No wording anywhere on the site claims a legal requirement, a compliance obligation, or a marketplace's approval.

Marketplaces are named in words only.
There is no marketplace logo anywhere on the site, because a logo on a marketing page reads as an endorsement, and the footer states plainly that Teachouse is independent, is not affiliated with any of them, and is endorsed by none of them.

There are no testimonials, no seller counts, no time-saved figures and no comparison table.
Nothing on the site is a number that has not been measured, which for a product with no public sellers means no numbers at all.

The availability sentence is the only claim on the site about whether a seller can use it today, and it is one editable string, so the founder can move it from private testing to general release in one place.

There is no pricing on the site and no checkout, because payments are parked.
The site asks to be told rather than asking to be paid, and the waitlist is a `mailto:` rather than a posted form: a form needs a backend that does not exist, and a hosted form service would put a seller's address with a third party and break the page's property of making no third-party request at all.
The waitlist copy says what the seller will hear about and when, and it states that pricing will be known before anything is charged for, which is a promise the parked decision can actually keep.

The site says that TeachersPayTeachers and Tes work needs a Windows app installed once.
It is said in "How it works" rather than buried, because a seller who learns it after signing up learns it as a surprise, and because the device story the site tells is not credible without it.
The download link is driven from one value that is null today, so the sentence stands and the link reads "Download link to come" rather than pointing at nothing.

Every answer in "Questions" is checked against the code rather than written from the design notes.
The claim that a change edits the listing already there rather than deleting and recreating it rests on the connector contract's `revise`, which addresses an existing `RemoteListingId`, and on the absence of any delist-and-relist path anywhere in `crates/`; removal is a separate verb that a ledger row has to authorise.

The status pill on a marketplace row is green only for the two marketplaces whose full create, publish and revise path has been proven live.
Etsy reads "Next" rather than "Working", because its connect path is designed and not built.

## Where the code holds each decision

Everything the founder replaces lives in `apps/landing/src/site.js`, and nowhere else.
That file holds the console origin, the sign-in URL, the support address, the waitlist address, the desktop download URL, the availability sentence and the marketplace list with its transport class.
`downloadUrl` is null, and a null renders as "Download link to come" rather than as a link, so no broken download can ship by being forgotten.
`waitlistHref` is the one value to change when the waitlist stops being a `mailto:`; nothing else on the site knows how a waitlist entry travels.
The marketplace transport wording matches `InventoryId::transport_class` in `crates/tam-domain/src/registry/mod.rs`, where TeachersPayTeachers and Tes are `SellerDevice` and Etsy is `OfficialApi`; if a marketplace's class changes there, this file changes with it.

`apps/landing/src/styles/site.css` transcribes the console's design tokens from `web/src/app.css` unchanged — the same ground, ink, accent, line and state colours, and the same Fraunces and Instrument Sans pairing — so the two surfaces read as one product.
The component classes below the tokens are this site's own, because the console's are dashboard furniture and none of it applies to a marketing page.

The two fonts are self-hosted rather than fetched from Google's CDN, which is what the console does.
The latin subsets and both SIL Open Font License texts are in `apps/landing/public/fonts/`, with a note recording where each file came from and how to refresh it.
The page therefore makes no third-party request at all.

## Placeholders the founder must replace

Seven items, all but two of them in `apps/landing/src/site.js`.

1. `waitlistEmail`, which assumes `hello@teachouse.io`. This is where every waitlist entry lands, so it wants an address the founder actually reads. Replacing the `mailto:` with a posted form means changing `waitlistHref` and nothing else.
2. `downloadUrl`, which is null. No public download page URL exists yet: releases go to CrabNebula Cloud on the `beta` channel, and CrabNebula's own documentation says a channelled release is not listed on an application's public page, while the GitHub releases beside them are in a private repository. Either publish a release to the unchannelled production stream, whose public page can then be linked, or host a download page of our own.
3. `consoleOrigin`, which assumes `https://app.teachouse.io`. The console's host has not been settled anywhere in the repository, and this is the only guess on the site.
4. `supportEmail`, which assumes `hello@teachouse.io`, and which appears in the footer and on both legal pages.
5. `availability`, the one sentence about whether a seller can use Teachouse today.
6. The whole of `/privacy`, which is a placeholder for counsel and not a policy.
7. The whole of `/terms`, which is a placeholder for counsel and not terms.

The three placeholder frames under "What you will be looking at" are not in this list because they are not a value to replace: they come out when there are real screenshots to put in, which needs the console running against a seeded catalogue.

Pricing is not in this list either.
It was removed rather than left unset, so there is no draft figure anywhere on the site to forget about.
When payments are decided, the pricing section is written fresh against whatever D4 has become by then.

## The two legal pages

Neither page contains invented legal text, and both say so at the top in a banner, with a "Draft placeholder" pill above the heading.

Each page instead does something useful for the founder's counsel: it sets down, as briefing material, the facts about the product that a drafter would otherwise have to be told, and then lists the questions the real document must answer.
The privacy page records what identity data an account carries, that the no-API marketplace session and the resource files stay on the seller's device, that an official-API token is held server-side, what the catalogue and the analytics series contain, that a third-party merchant of record takes the payment, and that the site itself carries no analytics.
The terms page records the independence from every marketplace, that the seller keeps their own relationship with each one, where each kind of request originates, the D4 metering shape, that a marketplace's own pricing rule is enforced as a publish gate rather than a warning, and that a decision the marketplace makes the seller's — a tax designation, a copyright assertion — is never filled in on the seller's behalf.

These pages must be replaced in full, not edited.

## Deployment to Cloudflare Pages

Nothing has been deployed, and no Cloudflare account has been touched.
The steps below are the whole of it, and the free plan covers this site.

1. In the Cloudflare dashboard, open Workers and Pages, create an application, choose Pages, and connect to the repository's Git host.
2. Set the production branch to `main`.
3. Set the framework preset to Astro, or leave it as none; the preset only fills the next two fields.
4. Set the build command to `npm ci --no-audit --no-fund && npm run build`.
5. Set the build output directory to `dist`.
6. Set the root directory to `apps/landing`, which is what makes the two fields above resolve against this app rather than the repository root.
7. Set the environment variable `NODE_VERSION` to `22`, matching `pkgs.nodejs_22` in the devShell.
8. Deploy, and confirm the preview URL renders the home page with both fonts and no console error.
9. In the project's custom domains, add `teachouse.io` and `www.teachouse.io`, and follow the dashboard's instruction to point the domain's nameservers or records at Cloudflare.
10. Confirm that `https://teachouse.io/privacy/` and `https://teachouse.io/terms/` both resolve, since the build emits directory-style routes.

There is no server-side runtime, no Worker, no binding and no secret, so nothing in `wrangler.toml` is needed and none is committed.
A pull-request preview deployment is on by default and is worth keeping, because it makes a copy change reviewable before it is public.

The `site` value in `apps/landing/astro.config.mjs` is `https://teachouse.io`, and it is what the canonical link and the Open Graph URL are built from; if the domain changes, that one value changes with it.

## Open items, each a founder decision

The site states where the marketplace work runs as what Teachouse is built to do, not as an absolute.
No sentence claims that our servers never hold a marketplace login or never open a marketplace session, and no sentence says the desktop app is the only thing that reaches a marketplace, because both would be false today.
Two paths run at once, for different traffic.
On the device, `apps/desktop/src-tauri/src/work.rs` composes and issues marketplace requests on the seller's machine, under the session the login webview filed in the operating system's keychain.
On our side, `crates/tam-sync-worker` reaches Tes through the broker's gateway with the seller cookie injected server-side, for the source reads that canonicalisation needs, taking a broker lease per request under `LeasePurpose::Drain`.
So the positive claim is true, and neither the absolute nor an exclusivity claim is.
The absolute wording, including D30's "your login never leaves your device" as a heading, returns only when no server-side path reaches a marketplace under a seller's session, and this paragraph is the reminder so nobody has to hold it in their head.
Check the claim against the tree rather than against this paragraph: the question is which processes still construct a marketplace adapter over a broker gateway.

Body text set in `--muted` does not clear the WCAG AA contrast minimum for normal text.
Measured on this palette, `--muted` is 3.90:1 on `--card` and 3.65:1 on `--ground` against a 4.5:1 requirement, and it is used for the section ledes (`.section .lede`), the hero's opening sell line (`.hero p.sell`), the install aside under "How it works" (`.aside`), the step copy (`.step p`), the marketplace transport column (`.row .how`), the FAQ answers (`.faq dd`), the screenshot captions (`.shot figcaption`), the footer and its navigation (`.site-foot`, `.site-nav a`) and the muted status pill (`.pill.mut`).
It is left alone here because these tokens are transcribed unchanged from `web/src/app.css` and diverging on the marketing site would break the one-product reading the transcription buys; the fix belongs in the console's palette, where a token between `--muted` and `--ink` would serve both surfaces.
The `--faint` uses were not left alone, because at 2.28:1 they were carrying the availability sentence, the marketplace-independence disclaimer and the D30 design-choice line, and a disclosure nobody can read is not a disclosure; those are now `--ink`.

The `/terms` placeholder still records the D4 metering shape as briefing material for counsel, and that is deliberate: the metering axis is a design decision that survives payments being parked, and the page is a brief rather than a public offer.
It is named here so that a later reader does not mistake it for pricing that escaped the removal.

There is no type-check step, only the build.
`astro check` would require `@astrojs/check` and `typescript` as dependencies, and the standing instruction is to report a dependency beyond the framework itself rather than add it; the build already fails on a template or import error, and the site has no application logic for a type checker to find a fault in.

D18 requires the NGSS disclaimer in the footer wherever the mark is used nominatively.
The landing page names no standards framework at all, so the disclaimer is not triggered here, and the obligation stays with the console.
If a future page describes standards alignment by name, the disclaimer comes with it.

There is no blog and no help centre yet.
Astro's content collections are the reason this site is Astro rather than a prerendered route, and adding the first one is a day's work when the founder wants it.
