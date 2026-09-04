# The teachouse.io landing page

The public site a seller reaches before signing up.
It is a separate Astro build rather than a console route, because the console's root layout turns off both server rendering and prerendering (D28).

`just landing-check` installs from the lockfile and builds, and runs as part of `just pre-push`; `just landing-dev` serves it locally.
The site ships no JavaScript, loads nothing from a third party, and self-hosts its two fonts from `public/fonts/`.

Payments are parked, so the site carries no pricing and no checkout.
The call to action is a waitlist, and it is a `mailto:` rather than a form, which is what keeps the page free of any third-party request.

Every value the founder must supply is in `src/site.js` and nowhere else: the console origin, the support address, the waitlist address, the desktop download URL and the availability sentence.
`downloadUrl` is `null` and renders as "Download link to come" rather than as a broken link, because no public download page URL exists yet.

`/privacy` and `/terms` are placeholders for counsel, not legal text, and must be replaced in full rather than edited.

The structure, the copy decisions, the full placeholder list and the Cloudflare Pages deployment steps are in `docs/notes/design/landing-page.md`.
