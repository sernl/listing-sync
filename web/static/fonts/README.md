# The console's two typefaces

Fraunces and Instrument Sans, served from this directory rather than from Google's CDN.
The console is packaged as the desktop and Android app, whose content-security-policy admits no third-party origin, so a face fetched from `fonts.gstatic.com` never arrived and every surface in the app fell back to the stacks in `lib/styles/tokens.css`.
Self-hosting is what makes the app render the faces the browser already did.
`apps/landing/public/fonts` does the same for the marketing site, and the two latin files here are byte-identical to the two there.

Downloaded from `fonts.gstatic.com` on 2026-09-06, at the URLs the two `fonts.googleapis.com/css2` stylesheets in the old `web/src/app.html` resolved to under a woff2-capable user agent:

| file | family | subset | bytes | sha256 |
|---|---|---|---|---|
| `fraunces-latin.woff2` | Fraunces v38 | latin | 67304 | `7234ed860a9cc83045413c4faee63c960a8f2d1917adcf728119307d56e0d783` |
| `fraunces-latin-ext.woff2` | Fraunces v38 | latin-ext | 59388 | `a2930b27d13a228bd9ab6a49269b5f800237892ad560cb9dd7fab01b1620f88e` |
| `fraunces-vietnamese.woff2` | Fraunces v38 | vietnamese | 19700 | `f18853f63a870ebef013e30e789d8d544f102e4acd94988e57c223d9c796ddf4` |
| `instrument-sans-latin.woff2` | Instrument Sans v4 | latin | 30092 | `2ee17598a98d8a59e4df8152d015bec9ab8e4d5672cc0ab42bef806b568e3971` |
| `instrument-sans-latin-ext.woff2` | Instrument Sans v4 | latin-ext | 11144 | `c4fcfea41f2c1cfeea9211fa43679845454a1d0e0d7e95e069c7e73c4ae302d2` |

Each family is one variable font per subset, not one file per weight: Google emits an `@font-face` block per requested weight and every block of a subset names the same URL.
`web/src/app.css` therefore declares one block per family and subset with a weight range — Fraunces 500 to 600 and Instrument Sans 400 to 600, which is what the old stylesheet links asked for and what the console's rules use.
The files carry a wider range than that (the landing declares Fraunces 400 to 700 from the byte-identical file), so widening is a change to the declaration and needs no new download.

Both families are licensed under the SIL Open Font License 1.1, whose text is beside the files as `OFL-Fraunces.txt` and `OFL-InstrumentSans.txt`.

To refresh a face, request the same `fonts.googleapis.com/css2` URL with a woff2-capable user agent, take one URL per `unicode-range` block, and check the `unicode-range` values in `app.css` against the ones the new stylesheet emits — a subset whose range moved and whose declaration did not will silently stop covering the characters it gained.
Provenance, the exact URLs and the decisions behind the subset choice are in `docs/notes/design/fonts.md`.
