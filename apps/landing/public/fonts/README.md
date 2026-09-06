# Self-hosted fonts

Poppins for headings and Inter for body text, the two faces of the brand kit, served from this directory rather than from Google's CDN.
The site's Content-Security-Policy admits no third-party origin, so a face fetched from `fonts.gstatic.com` would be refused rather than merely unnecessary.

Every file below is a byte copy of the one under `web/static/fonts/`, which is where the console loads the same faces from.
That is not tidiness: `crates/tam-server/src/serving.rs` probes this build ahead of the console's own static directory, so a name both builds carry is answered from this copy whichever tier asked for it, and the flake check `served-artefacts` fails a build where the two differ.

Downloaded from `fonts.gstatic.com` on 2026-09-11 by requesting the `fonts.googleapis.com/css2` URL for each family with a woff2-capable user agent and taking one file per `unicode-range` block.

| file | family | subset | bytes | sha256 | source |
|---|---|---|---|---|---|
| `poppins-400-latin.woff2` | Poppins v24, weight 400 | latin | 7884 | `7d93459d86585bfcdbb7e0376056226adb25821ee54b96236fe2123e9560929f` | `/s/poppins/v24/pxiEyp8kv8JHgFVrJJfecg.woff2` |
| `poppins-400-latin-ext.woff2` | Poppins v24, weight 400 | latin-ext | 5644 | `0b1fcab42c18b69bcfe9ce4799fcbff5af1621c53ffcfdc4723c6f5ec4ee3ffb` | `/s/poppins/v24/pxiEyp8kv8JHgFVrJJnecmNE.woff2` |
| `poppins-500-latin.woff2` | Poppins v24, weight 500 | latin | 7748 | `cd36de204aca2d5fa263a731f7c20009b5e3d754ba1f1e03c33e93a48f3e7446` | `/s/poppins/v24/pxiByp8kv8JHgFVrLGT9Z1xlFQ.woff2` |
| `poppins-500-latin-ext.woff2` | Poppins v24, weight 500 | latin-ext | 5484 | `af5fda16a19169e029a132374616728e1bf326d90bef5a552395c5053e21cd0f` | `/s/poppins/v24/pxiByp8kv8JHgFVrLGT9Z1JlFc-K.woff2` |
| `poppins-600-latin.woff2` | Poppins v24, weight 600 | latin | 8000 | `f4e80d9dfd374d02989b87a27b5ed4cb78fbb177c27f1478e9a8b0afb7513149` | `/s/poppins/v24/pxiByp8kv8JHgFVrLEj6Z1xlFQ.woff2` |
| `poppins-600-latin-ext.woff2` | Poppins v24, weight 600 | latin-ext | 5524 | `bb1f2d582e7fba586ab70c91ef062d3becaf78b887654953863521b73665d171` | `/s/poppins/v24/pxiByp8kv8JHgFVrLEj6Z1JlFc-K.woff2` |
| `poppins-700-latin.woff2` | Poppins v24, weight 700 | latin | 7816 | `9338e65fc077355c7a87ae0d64cc101e23b9bf8ad78ae65f0f319c857311b526` | `/s/poppins/v24/pxiByp8kv8JHgFVrLCz7Z1xlFQ.woff2` |
| `poppins-700-latin-ext.woff2` | Poppins v24, weight 700 | latin-ext | 5432 | `ccfd87f69ef00d811da3d06488cec4e79ec99d289cfbcbe4be42031cecae775a` | `/s/poppins/v24/pxiByp8kv8JHgFVrLCz7Z1JlFc-K.woff2` |
| `inter-400-700-latin.woff2` | Inter v20, weights 400 to 700 | latin | 48256 | `3100e775e8616cd2611beecfa23a4263d7037586789b43f035236a2e6fbd4c62` | `/s/inter/v20/UcC73FwrK3iLTeHuS_nVMrMxCp50SjIa1ZL7.woff2` |
| `inter-400-700-latin-ext.woff2` | Inter v20, weights 400 to 700 | latin-ext | 85068 | `34b9c504cab7a73e37b746343a449132e56cf7b5481af2cb81dc74dcff25c956` | `/s/inter/v20/UcC73FwrK3iLTeHuS_nVMrMxCp50SjIa25L7SUc.woff2` |

Poppins ships one static file per weight, so each weight is its own `@font-face`; Inter is one variable font per subset, so its two files carry the whole 400 to 700 range in one declaration each.

`latin-ext` is bundled rather than the latin subset alone because of te reo Māori: precomposed macron vowels such as `ā` are U+0101 and sit in Latin Extended-A, so a latin-only bundle would draw a New Zealand teacher's own words in a fallback face beside the real one.

Both families are licensed under the SIL Open Font License 1.1, whose text is beside the files as `OFL-Poppins.txt` and `OFL-Inter.txt`.
Poppins is copyright 2020 The Poppins Project Authors; Inter is copyright 2020 The Inter Project Authors.

To refresh a face, request the same `css2` URL again, take one file per `unicode-range` block, update the `unicode-range` values in `src/styles/site.css` against the ones the new stylesheet emits, and copy the same bytes to `web/static/fonts/`.
