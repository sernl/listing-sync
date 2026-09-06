# The product icon

Every icon the product shows, on the web console, on the marketing site, in the desktop client and on Android, is derived from one drawing.

- date: 2026-09-11
- status: the drawing is superseded by the founder's brand kit — the two-colour house on an indigo tile — and every artefact is regenerated from it and checked at every size, the console and landing set, the email set, the desktop and Android set, and the 16 pixel raster the small mark is drawn for; the mark itself is the founder's choice and is not to be redrawn; the console declares the PNG icons alone, on purpose, and the reason is recorded below
- source: `web/static/brand/mark.svg`, copied to `web/static/favicon.svg`
- paths: `web/static/`, `apps/landing/public/`, `apps/desktop/src-tauri/icons/`, `apps/desktop/src-tauri/gen/android/app/src/main/res/`

## Amended 2026-09-11: the brand-kit mark

The founder supplied a brand kit, and the drawing every icon derives from is now its mark: `web/static/brand/mark.svg`, a tile in `#1E2A5A` with `rx="22"` on a 96 unit viewBox, carrying the house of `web/static/brand/house.svg` in `#00B894` at 64 by 68 units, offset 16 and 14.
The mark writes two colours and no more, `#1E2A5A` and `#00B894`, which are the values of `--primary` and `--accent`.
The cream gable, the open book and the jade flower of the Pounamu mark are gone, and with them the two creams that had a named exception in `tokens.test.ts`.
Every section below is rewritten to the kit where it named a colour or a source, and the Pounamu-era readings that survive are the ones about rendering, storage class and the sizes each artefact is drawn for, none of which depends on what the drawing is.

## The drawing and its copies

`web/static/brand/mark.svg` is the source of truth: a tile in `#1E2A5A` with `rx="22"`, carrying the house in `#00B894` at 64 by 68 units, all on a 96 unit viewBox.
`web/static/favicon.svg` is a byte copy of it, and `apps/landing/public/favicon.svg` a byte copy in turn, because the two trees build separately and a path served by the console's fallthrough would 404 under `just landing-dev`; the reasoning is recorded at the head of `apps/landing/src/marketplaces.js`.
`web/static/unreachable.html` embeds the drawing inline, because the desktop app opens that page from its own bundle with no stylesheet and no network.
`web/static/email/teachouse-mark.svg` and `teachouse-mark-small.svg` are the same two drawings again, with explicit `width`, `height` and `role` attributes, which is what a mail client composing without a stylesheet needs.
`tokens.test.ts` holds all of them to the same colours, so a recolour of one without the others fails the web lane.

The mark writes two colours, `#1E2A5A` and `#00B894`, which are the values of `--primary` and `--accent`, and nothing else.
That is what retired the named exception the Pounamu and Kauri marks' two creams held in `tokens.test.ts`, and what let the two email marks join the swept set.
The `email/` exception now names `teachouse-delivery.svg` alone, the illustration beside them, whose skin tones and sky no token names.

`web/static/brand/mark-small.svg` is the 16 pixel companion: the same tile and the same two colours, with the house redrawn heavier so it survives the size.

Two palettes preceded this one.
Kauri was the original, and `web/static/email/teachouse-mark.svg` was its source drawing, with `favicon.svg` produced by copying it.
Pounamu replaced it on 2026-09-06 and reversed that copy step, making `favicon.svg` the original.
The brand kit replaced Pounamu on 2026-09-11 and moved the original again, to `brand/mark.svg`, with `favicon.svg` now a copy of it: nothing under `email/` is a source any more, and every drawing named above is on the kit.

## Regenerating the web console and landing icons

The renderer is resvg, which resolves the 96 unit viewBox to whatever `--width` and `--height` ask for.
It is on the profile path; `nix shell nixpkgs#resvg --command resvg ...` runs it anywhere.
Rendered on 2026-09-11 from the brand-kit mark, with resvg 0.48.1 and ImageMagick 7.1.2-29, having been rendered on 2026-09-06 with resvg 0.47.0 from the Pounamu one.

```
for n in 32 96 180 192 512; do
  resvg --width $n --height $n web/static/favicon.svg /tmp/mark-$n.png
done
resvg --width 16 --height 16 web/static/brand/mark-small.svg /tmp/mark-16.png
magick /tmp/mark-16.png  -depth 8 -strip       web/static/favicon-16.png
magick /tmp/mark-32.png  -depth 8 -strip       web/static/favicon-32.png
magick /tmp/mark-192.png -depth 8 -strip PNG32:web/static/icon-192.png
magick /tmp/mark-512.png -depth 8 -strip PNG32:web/static/icon-512.png
magick /tmp/mark-180.png -background '#1E2A5A' -alpha remove -alpha off \
  -depth 8 -strip PNG24:web/static/apple-touch-icon.png
magick /tmp/mark-96.png -depth 8 -strip PNG32:web/static/email/teachouse-mark.png
resvg --width 1040 --height 520 web/static/email/teachouse-delivery.svg /tmp/delivery.png
magick /tmp/delivery.png -depth 8 -strip PNG32:web/static/email/teachouse-delivery.png
cp web/static/favicon.svg apps/landing/public/favicon.svg
```

The second ImageMagick pass exists only to fix each file's storage class, because resvg always writes RGBA and the set is not uniform: the two favicons are palette PNGs, `icon-192` and `icon-512` are truecolour with alpha, and `apple-touch-icon` is truecolour without, which is what an iOS home-screen icon must be.
`PNG32:` and `PNG24:` are what force the last three, since ImageMagick otherwise palettes them: resvg antialiases to well under 256 colours, so the reduction is lossless and would pass unnoticed.
Flattening the apple-touch icon onto the tile colour fills the corners the `rx="22"` tile leaves open, which is the square opaque icon iOS wants.

`favicon-32.png` is rendered from the full mark, and `favicon-16.png` from `brand/mark-small.svg`, which is drawn for that size.
At 16 pixels the full mark's four window panes fill in and the gaps between them close, so the companion replaces the window with one rounded opening and carries the roof and walls heavier.
Until 2026-09-11 the 16 pixel source was `email/teachouse-mark-small.svg`, and the kit supplies its own small mark, so the render now reads from `brand/`.
The two SVGs under `email/` are copies of the brand drawings rather than sources, and the two PNGs beside them are truecolour with alpha: `teachouse-mark.png` because a mail client is handed a raster with no palette to negotiate, and `teachouse-delivery.png` because the illustration has more colours than a palette holds.

The landing declares the SVG alone, in the head of `apps/landing/src/layouts/Base.astro`, and draws it again as an `img` in `SiteHeader.astro` and `SiteFooter.astro`.
It has no raster icons and no manifest, so the loop above writes nothing else for it.

`web/src/app.html` declares the PNG icons alone, and the absence of an SVG link is a decision rather than an omission:

```html
<link rel="icon" href="/favicon-32.png" sizes="32x32" type="image/png" />
<link rel="icon" href="/favicon-16.png" sizes="16x16" type="image/png" />
<link rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180" />
```

A tab icon is drawn at 16 pixels, and the full mark scaled down that far goes muddy.
`favicon-16.png` is rendered from the simplified small mark of `brand/mark-small.svg` for exactly that reason.
An SVG link would hand that size back to the full mark, because a browser that understands the format prefers it, so the console declares the rasters and no SVG.
`brand/mark.svg` stays the source drawing every other artefact is rendered from, and the console still draws its `favicon.svg` copy directly at the sizes where it holds up: an `img` of 28 pixels in `+layout.svelte` and of 34 in `Console.svelte`.

`icon-192.png` and `icon-512.png` exist for a web manifest that has not been written yet, and nothing references them until it is.
When it is written, its `theme_color` and `background_color` take the values of `--primary` and `--ground` from `web/src/lib/styles/tokens.css`.

## Regenerating the desktop and Android icons

This set was regenerated from the brand-kit mark on 2026-09-11 by the commands below, on tauri-cli 2.11.4 and resvg 0.48.1.
Thirty-three tracked files changed: seventeen under `icons/`, fifteen mipmaps under `res/`, and `ic_launcher_background.xml`, which the run rewrote from `#1F4A38` to the `bg_color` the manifest now carries.
Eighteen more were written under `icons/ios/`, which `.gitignore` excludes.
Nothing outside those two trees was written by the run itself.

The previous run, on 2026-09-06 and on resvg 0.47.0, wrote the same file set from the Pounamu mark.

The Tauri CLI writes it from a manifest, given an opaque square source and a foreground already inset into the Android adaptive-icon safe zone.
The three inputs are written outside the repository and the manifest passed by absolute path, since the CLI resolves the manifest's own entries relative to itself and none of the three belongs in the tree.

```
resvg --width 1024 --height 1024 web/static/brand/mark.svg /tmp/mark-1024.png
magick /tmp/mark-1024.png -background '#1E2A5A' -alpha remove -alpha off \
  -depth 8 -strip PNG32:app-icon.png
resvg --width 1024 --height 1024 app-icon-fg.svg android-fg.png
nix develop . --command sh -c 'cd apps/desktop/src-tauri && cargo tauri icon /abs/path/icon-manifest.json'
```

`icon-manifest.json`, whose paths are relative to itself:

```json
{
  "default": "app-icon.png",
  "bg_color": "#1E2A5A",
  "android_fg": "android-fg.png",
  "android_fg_scale": 100
}
```

`app-icon-fg.svg`, the house of `web/static/brand/house.svg` with the mark's tile removed, placed inside the adaptive-icon safe zone:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="108" height="108" viewBox="0 0 108 108">
  <g transform="translate(24.84,24.02) scale(0.0826)"><!-- the single path of house.svg, verbatim --></g>
</svg>
```

`house.svg` draws on a 720 by 760 viewBox and leaves padding in it: the inked content is 638 by 680 at an offset of 34 and 23, measured by trimming a render of the file.
The scale and the translate are derived from those numbers rather than from the viewBox, so it is the ink and not the padding that is centred and sized.

`bg_color` becomes the `ic_launcher_background` colour resource, so the adaptive icon's ground is the tile colour rather than the CLI's default white.
`android_fg` matters more: without it the CLI writes a full-bleed foreground that every Android 8 launcher crops to its middle two thirds, cutting the roof and the eaves off the house.
The foreground above therefore carries the house alone, sized so its taller dimension spans 52 percent of the 108 unit canvas and centred on it, which leaves it well inside the 72 of 108 mask.
A trim of the rendered `android-fg.png` reads 500 by 534 at an offset of 262 and 245 on 1024 pixels, which is that 52 percent, centred to the pixel.

`PNG32:` on the source matters for the reason it does in the web loop: ImageMagick palettes a 1024 pixel render, because resvg antialiases to about a hundred colours.
The 1024 pixel iOS icon is written at the source's own size and follows its storage class, so a palette source changes that one file's encoding, while every resized output is unaffected either way.
With the truecolour source, all thirty tracked rasters keep the dimensions, bit depth and PNG colour type they had, and `icon.ico` keeps its six PNG-compressed frames.
Read the colour type from the IHDR rather than from `magick identify`: `%[type]` reports how ImageMagick classified the decoded pixels, so the two-colour mark reads back as `PaletteAlpha` from a file whose IHDR byte is 6, and a whole set looks to have changed encoding when none of it has.
`od -An -tu1 -j24 -N2 file.png` prints the bit depth and colour type the file actually stores, and across the thirty that pair is identical before and after.

The one thing a regeneration does not reproduce is `icon.icns`, whose twelve OSType entries the CLI emits in a different order each run.
The entry set and the images in it are stable, so an icns diff of that shape is expected and carries no information on its own.

## What consumes them

`apps/desktop/src-tauri/tauri.conf.json` names `icons/32x32.png`, `icons/128x128.png` and `icons/icon.ico`, the bundlers also read `icon.icns` and the `Square*Logo.png` set, and `AndroidManifest.xml` reads `@mipmap/ic_launcher`.
The `icons/ios/` set is written on every run although no iOS project exists, and `apps/desktop/src-tauri/.gitignore` excludes it, so it never reaches a diff.
One limitation stands: the 16 pixel layer inside `icons/icon.ico` is the full mark downscaled by the CLI and reads muddy in the Windows title bar, and replacing it needs an ICO writer that keeps PNG-compressed frames, which ImageMagick is not.
