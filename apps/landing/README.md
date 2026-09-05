# The teachouse.io landing page

The public site a seller reaches before signing up.
It is a separate Astro build rather than a console route, because the console's root layout turns off both server rendering and prerendering (D28).

`just landing-check` installs from the lockfile and builds, and runs as part of `just pre-push`; `just landing-dev` serves it locally.
`nix/landing.nix` builds the same site as a store path, exposed as the flake package `teachouse-landing` and as the flake check `landing`.

Deployment is not a static host of its own.
`tam-server --landing-dir <path>` reads the built directory into memory at start-up and answers from it ahead of the console, which is what `services.teachouse.landingPackage` supplies.
A request goes to the API if its first segment is a version, then to this build if it holds a file for the path, and to the console otherwise; a directory route resolves through its own `index.html`, which is how `/pricing`, `/privacy` and `/terms` work.
So this site owns `/`, and the console keeps `/app` and every route below it, along with `/login` and everything else this build holds no file for.
The server computes the landing page's Content-Security-Policy from the files it just read, so a page that grew an inline script would be served under a policy carrying that script's hash rather than under a stale one.

The only script on the site is `public/app-redirect.js`, loaded from our own origin on every page: the desktop app opens this origin too, and `window.__TAURI__` is the one signal available before the console loads, so it sends that window to `/app`.
Nothing else on the site comes from anywhere but this origin: Fraunces and Instrument Sans are served from `public/fonts/` rather than from Google's CDN, so the page makes no third-party request at all.
The site must keep working under the policy `landing_policy` in `crates/tam-server/src/serving.rs` builds — `default-src 'self'`, `style-src` adding `'unsafe-inline'` and `fonts.googleapis.com`, `font-src` adding `fonts.gstatic.com`, `img-src` adding `data:`, `frame-ancestors 'none'` — which is why there is no inline event handler on any page.

Every call to action goes to `/login`, and both `/` and `/pricing` carry the same price list because both read `src/pricing.js`.
Every price on the site is in that one file, in USD, as approved on 2026-09-05.

Every value the founder must supply is in `src/site.js` and nowhere else: the login path, the support address, the desktop download URL and the availability sentence.
`supportEmail` is `null` and renders no address anywhere rather than a `mailto:` that reaches nobody.
`downloadUrl` is `null` and renders as "Download link to come" rather than as a broken link, because no public download page URL exists yet.

`/privacy` and `/terms` are placeholders for counsel, not legal text, and must be replaced in full rather than edited.

The structure, the copy decisions and the full placeholder list are in `docs/notes/design/landing-page.md`.
