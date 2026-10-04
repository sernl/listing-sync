# The teachouse.io landing page

The public site a seller reaches before signing up.
It is a separate Astro build rather than a console route, because the console's root layout turns off both server rendering and prerendering (D28).

`just landing-check` installs from the lockfile, builds, and runs the style, copy and spacing gates (`scripts/check-spacing.mjs` fails when words run into a link or an interpolation), as part of `just pre-push`; `just landing-dev` serves it locally.
`nix/landing.nix` builds the same site as a store path, exposed as the flake package `teachouse-landing` and as the flake check `landing`.

Deployment is not a static host of its own.
`tam-server --landing-dir <path>` reads the built directory into memory at start-up and answers from it ahead of the console, which is what `services.teachouse.landingPackage` supplies.
A request goes to the API if its first segment is a version, then to this build if it holds a file for the path, and to the console otherwise; a directory route resolves through its own `index.html`, which is how `/pricing`, `/privacy` and `/terms` work.
So this site owns `/`, and the console keeps `/app` and every route below it, along with `/login` and everything else this build holds no file for.
In production the two have separate hosts: with `--landing-host teachouse.io --console-host dash.teachouse.io`, this build is answered only on the apex, every console path there is a 301 to `dash.teachouse.io`, and every link from this site into the console is built from `consoleUrl` in `src/site.js` (`https://dash.teachouse.io`, or `PUBLIC_CONSOLE_URL` at build time for local development).
`robots.txt` speaks for the apex alone, refusing `/v1/`, `/api/`, `/ingest/`, `/downloads/` and the maintenance preview and naming no console route, which `served-artefacts` holds; the console host answers its own.
A page whose first path segment is one the console answers under fails the flake check `served-artefacts`, because tam-server probes this build ahead of the console and such a page would take that path from the seller's board.
The server computes the landing page's Content-Security-Policy from the files it just read, so a page that grew an inline script would be served under a policy carrying that script's hash rather than under a stale one.

Two scripts run here, both from our own origin.
`/app-redirect.js` is loaded on every page: an endpoint (`src/pages/app-redirect.js.js`) so it can carry `consoleUrl`. A desktop window that lands on this site has no use for it, and `window.__TAURI__` is the one signal available before anything loads, so the script sends that window to the console's `/app` on the console host.
The Tes band calculator in `components/Faq.astro` is the other, bundled by Astro from that component and small enough that Astro inlines it into the page; the server hashes every inline script it reads, so the policy it serves already carries this one's `sha256-` token.
It imports `src/tes-bands.js` rather than `src/pricing.js`, which keeps the generated plan table out of the browser bundle, and it is the only behaviour on the site that needs script at all: the FAQ's `<details>` and everything else work with script refused.
Nothing else on the site comes from anywhere but this origin: Poppins and Inter are served from `public/fonts/` rather than from Google's CDN, so the page makes no third-party request at all.
No inline `style` attribute either, for the same reason there is no inline `<style>`: `style-src 'self'` refuses both, so every shape on the page — the hero's blobs and the handwritten line included — is drawn by a class in `src/styles/site.css` or by SVG presentation attributes.
The site must keep working under the policy `landing_policy` in `crates/tam-server/src/serving.rs` builds — `default-src 'self'`, `script-src 'self'` plus a `sha256-` token per inline script found in the build, `style-src 'self'`, `font-src 'self'`, `img-src 'self' data:`, `connect-src 'self' https://cloudflareinsights.com`, `frame-ancestors 'none'` — which is why there is no inline event handler and no inline `<style>` on any page.
The policy admits one third-party origin, on `connect-src` only: the endpoint the Cloudflare Web Analytics beacon the edge injects posts to. Otherwise it admits none: `style-src` carried `'unsafe-inline'` and `fonts.googleapis.com`, and `font-src` carried `fonts.gstatic.com`, until the console's fonts were bundled and those origins were dropped, so a stylesheet or a font fetched from anywhere but this origin is now refused rather than merely unnecessary.

Every call to action goes to the console's signup at `https://dash.teachouse.io/signup`, carrying `next=/settings/billing` and, where the reader picked something, `price=<PriceKey>` — the plan's yearly key (`starter_yearly`, `pro_yearly`, `studio_yearly`) from its card, `pack_100` from the packs band, and no `price` at all from "Start free".
The console's signup is what honours those two: `next` is where to land, and `price` is the checkout to open on arrival.
Both `/` and `/pricing` carry the same price list because both render `components/Pricing.astro` from `src/pricing.js`.
Every price and cap on the site comes from `src/plans.generated.js`, which `cargo run -p tam-typegen` emits from the `tam-limits` plan table the server enforces and `just web-check` diffs; `src/pricing.js` holds only the landing's own phrasing of it, in USD.
Changing a price is an edit in `tam-limits` and a regeneration, never an edit here.

The brand files under `public/brand/`, the marks under `public/marks/` and the faces under `public/fonts/` are byte copies of the console's, not links: the two trees build separately, and `landing-band.test.ts` fails a copy that drifted.
`public/images/og.png` is the social card, 1200 by 630, rendered from `public/brand/logo.svg` with resvg; nothing rebuilds it, so refreshing the logo means re-rendering the card.

Every value the founder must supply is in `src/site.js` and nowhere else: the login path, the support address and the availability sentence.
`supportEmail` is `contact@teachouse.io`, the monitored address the footer, the maintenance page, the privacy policy and the terms all name.
The site links no download: the app card says to download the app once signed in, and the console offers the build.

`/privacy` is the privacy policy, written against how the product is built; a change to what Teachouse collects, where it keeps it or who processes it must move its text and its `updated` date in the same commit.
`/terms` is the terms of service, written against how the product works and bills; a change to plans, refunds, cancellation or what Teachouse keeps must move its text and its `updated` date in the same commit. It names the two marketplaces that connect today, by their full names, in the one sentence that says Teachouse is independent of them.

The structure and the copy decisions are in `docs/notes/design/brand-kit-and-teacher-ui.md`, which supersedes the copy, tokens and pricing of `landing-page.md`.
The founder owes no artwork: since 2026-09-12 the hero and the challenge are inline SVG compositions drawn in tokens, and the solution is a real capture of the console's Resources board under `public/images/`, recorded under "Amended 2026-09-12" in `landing-page.md`.
