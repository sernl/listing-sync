# Self-hosted fonts

The console loads Fraunces and Instrument Sans from Google's CDN (`web/src/app.html`).
This site self-hosts the same two faces so the page fetches nothing from a third party at load time.

Both files are the latin subset of the variable font, downloaded from `fonts.gstatic.com` on 2026-09-03:

- `fraunces-latin.woff2` — Fraunces v38, axes `opsz 9..144`, `wght 400..700`.
- `instrument-sans-latin.woff2` — Instrument Sans v4, axis `wght 400..600`.

Both are licensed under the SIL Open Font License 1.1; the licence text for each is beside the file.
To refresh a face, request the same `fonts.googleapis.com/css2` URL with a woff2-capable user agent and take the latin `unicode-range` entry.
