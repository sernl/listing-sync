# The teachouse.io landing page

The public site a seller reaches before signing up.
It is a separate Astro build rather than a console route, because the console's root layout turns off both server rendering and prerendering (D28).

`just landing-check` installs from the lockfile and builds; `just landing-dev` serves it locally.
The site ships no JavaScript, loads nothing from a third party, and self-hosts its two fonts from `public/fonts/`.

Every value the founder must supply is in `src/site.js` and nowhere else: the console origin, the support address, the availability sentence, the pricing tiers and the marketplace list.
A tier whose `amount` is `null` renders as an unset price with a "Price not set" pill, so a draft figure cannot ship by being forgotten.

`/privacy` and `/terms` are placeholders for counsel, not legal text, and must be replaced in full rather than edited.

The structure, the copy decisions, the full placeholder list and the Cloudflare Pages deployment steps are in `docs/notes/design/landing-page.md`.
