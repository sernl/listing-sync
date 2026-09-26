# Cover rasteriser: dependency and licences

Added 2026-09-26 for Teachouse 0.12, so a PDF's cover is its first page instead of a generated card.

## The crate

`tam-pipeline` depends on `hayro` 0.7.1 (<https://github.com/LaurenzV/hayro>), a PDF rasteriser written in pure Rust.
It is licensed `Apache-2.0 OR MIT`.
It sets `forbid(unsafe_code)` at its own crate root, and nothing in its tree needs a C toolchain, so every client target still builds the way blake3's `pure` feature requires.
It pulls in `hayro-interpret`, `hayro-syntax`, `hayro-cmap`, `hayro-jbig2`, `hayro-jpeg2000`, `hayro-ccitt`, `hayro-postscript`, `vello_cpu`, `vello_common`, `kurbo`, `skrifa`, `read-fonts`, `moxcms` and `pic-scale`.
All of them are `MIT` and/or `Apache-2.0`.

## Embedded assets

The default features stay on, so no font or CMap has to be vendored in this repository.

- `embed-fonts`: stand-ins for the 14 standard PDF fonts, which a PDF may use without embedding them. They are PDFium's Foxit Type 1 fonts (`FoxitSans*.pfb`, `FoxitSerif*.pfb`, `FoxitFixed*.pfb`, `FoxitSymbol.pfb`, `FoxitDingbats.pfb`), shipped inside `hayro-interpret` and about 240 KB in total. Licence: the PDFium BSD-3-Clause licence, "Copyright 2014 PDFium Authors", original code copyright 2014 Foxit Software Inc. A binary distribution must reproduce the copyright notice and disclaimer. The text is in `hayro-interpret-0.7.0/assets/LICENSE_FOXIT`.
- `hayro-interpret` also bundles two colour profiles. `CGATS001Compat-v2-micro.icc` is CC0-1.0 (saucecontrol/Compact-ICC-Profiles). `LAB.icc` was generated with LCMS2 and is CC0-1.0.
- `embed-cmaps`: Adobe's predefined CMaps, used for CJK text, compiled into `hayro-cmap` (about 260 KB).

## Bounds

A PDF is untrusted input.
`render::cover` draws page 1 at the cover frame's own scale, so the pixmap is at most 1600×1200 whatever page size the file declares.
The render runs on its own thread, and the cover gives up on it after `PAGE_RENDER_TIMEOUT` (10 s) and uses the generated card.
The rasteriser cannot be cancelled, so a render that runs past the deadline carries on in the background until it finishes, and its result is thrown away.
