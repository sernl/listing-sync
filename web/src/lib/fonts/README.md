# The console's two typefaces

Poppins and Inter, the founder's brand-kit faces, served from this directory rather than from Google's CDN.
The console is packaged as the desktop and Android app, whose content-security-policy admits no third-party origin, so a face fetched from `fonts.gstatic.com` never arrived and every surface in the app fell back to the stacks in `lib/styles/tokens.css`.
Self-hosting is what makes the app render the faces the browser already did.
`apps/landing/public/fonts` does the same for the marketing site, and all ten files here are byte-identical to the ten there.

Downloaded from `fonts.gstatic.com` on 2026-09-11, at the URLs the `fonts.googleapis.com/css2` stylesheets for `Poppins:wght@400;500;600;700` and `Inter:wght@400..700` resolved to under a woff2-capable user agent:

| file                            | family                | subset    | bytes | sha256                                                             |
| ------------------------------- | --------------------- | --------- | ----- | ------------------------------------------------------------------ |
| `poppins-400-latin.woff2`       | Poppins v24, 400      | latin     | 7884  | `7d93459d86585bfcdbb7e0376056226adb25821ee54b96236fe2123e9560929f` |
| `poppins-400-latin-ext.woff2`   | Poppins v24, 400      | latin-ext | 5644  | `0b1fcab42c18b69bcfe9ce4799fcbff5af1621c53ffcfdc4723c6f5ec4ee3ffb` |
| `poppins-500-latin.woff2`       | Poppins v24, 500      | latin     | 7748  | `cd36de204aca2d5fa263a731f7c20009b5e3d754ba1f1e03c33e93a48f3e7446` |
| `poppins-500-latin-ext.woff2`   | Poppins v24, 500      | latin-ext | 5484  | `af5fda16a19169e029a132374616728e1bf326d90bef5a552395c5053e21cd0f` |
| `poppins-600-latin.woff2`       | Poppins v24, 600      | latin     | 8000  | `f4e80d9dfd374d02989b87a27b5ed4cb78fbb177c27f1478e9a8b0afb7513149` |
| `poppins-600-latin-ext.woff2`   | Poppins v24, 600      | latin-ext | 5524  | `bb1f2d582e7fba586ab70c91ef062d3becaf78b887654953863521b73665d171` |
| `poppins-700-latin.woff2`       | Poppins v24, 700      | latin     | 7816  | `9338e65fc077355c7a87ae0d64cc101e23b9bf8ad78ae65f0f319c857311b526` |
| `poppins-700-latin-ext.woff2`   | Poppins v24, 700      | latin-ext | 5432  | `ccfd87f69ef00d811da3d06488cec4e79ec99d289cfbcbe4be42031cecae775a` |
| `inter-400-700-latin.woff2`     | Inter v20, 400 to 700 | latin     | 48256 | `3100e775e8616cd2611beecfa23a4263d7037586789b43f035236a2e6fbd4c62` |
| `inter-400-700-latin-ext.woff2` | Inter v20, 400 to 700 | latin-ext | 85068 | `34b9c504cab7a73e37b746343a449132e56cf7b5481af2cb81dc74dcff25c956` |

The two families are packaged differently, and the file counts follow that rather than a choice.
Inter is a variable font, so one file per subset carries the whole 400-to-700 range and `web/src/app.css` declares it with a weight range.
Poppins is not, so Google serves one file per weight and subset, and the stylesheet declares eight blocks for the four weights the kit names.
The console draws headings at 600 and 700; 400 and 500 are held because the kit's own type specimen uses them and a heading weight change should not need a download.

Only the latin and latin-ext subsets are taken.
Both stylesheets also emit devanagari for Poppins and the cyrillic, greek and vietnamese sets for Inter, and none of them is downloaded or declared: a subset that is not declared is a subset the browser never asks for.

Both families are licensed under the SIL Open Font License 1.1, whose text is beside the files as `OFL-Poppins.txt` and `OFL-Inter.txt`, both taken from `google/fonts` because these are Google Fonts' builds and that is the licence copy travelling with the bytes.

To refresh a face, request the same `fonts.googleapis.com/css2` URL with a woff2-capable user agent, take one URL per `unicode-range` block, and check the `unicode-range` values in `app.css` against the ones the new stylesheet emits — a subset whose range moved and whose declaration did not will silently stop covering the characters it gained.
The brand decision these faces come from is `docs/notes/design/brand-kit-and-teacher-ui.md`; the subset reasoning this directory inherited is in `docs/notes/design/fonts.md`.
