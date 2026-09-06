---
title: Where the two typefaces come from
---

The product uses two faces: Fraunces for display and Instrument Sans for text.
Both are self-hosted on every surface, and nothing fetches type from a third party.
This note records where the files came from, why the console stopped using Google's CDN, and what to check when a face is refreshed.

## Why self-hosting, rather than the CDN

The console is packaged as the desktop and Android app, and that build shows the same bundle a browser does.
Its content-security-policy in `apps/desktop/src-tauri/tauri.conf.json` is `default-src 'self'` with `style-src 'self' 'unsafe-inline'` and no `font-src` of its own, so a stylesheet from `fonts.googleapis.com` and a font file from `fonts.gstatic.com` were both refused there.
Nothing reported it: the page drew, and every surface fell back to the stacks in `web/src/lib/styles/tokens.css`.
That is why the app read as a plainer thing than the browser did, and it is the first item a reader of `docs/notes/engineering/console-render-harness.md` would have suspected if the render harness had ever been pointed at the app.

Widening the app's policy was the alternative, and it was refused.
It would have let the app reach two external origins on a product whose whole architecture is an argument about where a request originates, and it would have bought a network round trip on first paint for something that can ship in the bundle.
The founder approved bundling in words on 2026-09-06.

The marketing site reached the same conclusion first, on 2026-09-03, and `apps/landing/public/fonts` has carried both faces since.
The two latin files in `web/static/fonts` are byte-identical to the two there, which is the check that both surfaces are drawing the same face rather than two builds of it.

## The files

Downloaded from `fonts.gstatic.com` on 2026-09-06, at the URLs the two stylesheet links then in `web/src/app.html` resolved to under a woff2-capable user agent.
Requesting those two `fonts.googleapis.com/css2` URLs is how the file URLs are discovered; the stylesheet varies by user agent, and a woff2-capable one is what the console's own browsers are.

- `Fraunces:opsz,wght@9..144,500;9..144,600&display=swap` emitted six `@font-face` blocks across three subsets and two weights, naming three distinct files.
- `Instrument+Sans:wght@400;500;600&display=swap` emitted six blocks across two subsets and three weights, naming two distinct files.

Each subset is one variable font, so a weight block is a declaration rather than a file: every block of a subset named the same URL.
`web/src/app.css` therefore declares one block per family and subset with a weight range, and transcribes each `unicode-range` from the stylesheet unchanged.
The five files, their sizes and their sha256 digests are tabulated in `web/static/fonts/README.md`.

Fraunces is declared `font-weight: 500 600` and Instrument Sans `400 600`, which is what the old links asked for and what the console's rules use — every rule naming `--display` asks for 500 or 600, and none asks for more.
The files themselves carry a wider range: the landing declares Fraunces `400 700` from the byte-identical file.
Widening the console's declaration is therefore an edit to `app.css` and needs no new download.

Fraunces' `opsz` axis is left to the browser, as it was under the CDN: neither the old stylesheet nor these blocks set `font-variation-settings`, and `font-optical-sizing: auto` is the default, so optical size continues to track font size.

All three Fraunces subsets are bundled, including vietnamese, and both Instrument Sans subsets.
The reason to keep `latin-ext` rather than the latin subset alone is te reo Māori: precomposed macron vowels such as `ā` are U+0101 and sit in Latin Extended-A, so a latin-only bundle would render a New Zealand seller's own resource titles in a fallback face beside the real one.
Google emitted no vietnamese subset for Instrument Sans, so Vietnamese body text falls back exactly as it did before this change.

## Licensing

Both families are under the SIL Open Font License 1.1.
The licence text for each is beside the files, as `web/static/fonts/OFL-Fraunces.txt` and `OFL-InstrumentSans.txt`, copied from the pair the landing already carries.
Fraunces is copyright 2018 The Fraunces Project Authors; Instrument Sans is copyright 2022 The Instrument Sans Project Authors.
The OFL requires the licence to travel with the font files, which is why each surface carries its own copy rather than pointing at the other's.

## What the change touched

`web/src/app.html` lost two `preconnect` links and two stylesheet links.
`web/src/app.css` gained five `@font-face` blocks, each `font-display: swap`, immediately after the sheet's imports because a CSS `@import` may not follow another rule.
`web/static/fonts` is new.
`console_policy` in `crates/tam-server/src/main.rs` lost `https://fonts.googleapis.com` from `style-src` and `https://fonts.gstatic.com` from `font-src`, and a test now asserts that no directive names either host, so restoring the grant fails the build rather than quietly reopening the split between what a browser rendered and what the app did.

The fallback stacks in `tokens.css` are unchanged and still do their job: `swap` means the fallback is what a reader sees until the file arrives, and a file that fails to arrive leaves them standing.

## Two stale statements elsewhere, not corrected here

Both predate this change and describe the landing page rather than the console, so they are recorded rather than edited.

`docs/notes/design/landing-page.md:131` says "the server's policy permits `fonts.gstatic.com` without the site needing it", and `:172` describes `landing_policy` as adding `https://fonts.googleapis.com` to `style-src` and `https://fonts.gstatic.com` to `font-src`.
`apps/landing/README.md:17` says the same.
The code disagrees: `landing_policy` in `crates/tam-server/src/serving.rs` is `style-src 'self'; font-src 'self'` and its own doc comment records that the first version carried the pair and that the built site loads neither host.
The code is the later statement, so the three documentation lines are what is out of date.
