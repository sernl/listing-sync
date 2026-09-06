# The product icon

Every icon the product shows, on the web console, on the marketing site, in the desktop client and on Android, is derived from one drawing.

- date: 2026-09-06
- status: every artefact is regenerated on the Pounamu palette and checked at every size, the console and landing set, the desktop and Android set, and the 16 pixel raster the small mark is drawn for; the mark itself is the founder's choice and is not to be redrawn; the console declares the PNG icons alone, on purpose, and the reason is recorded below
- source: `web/static/favicon.svg`
- paths: `web/static/`, `apps/landing/public/`, `apps/desktop/src-tauri/icons/`, `apps/desktop/src-tauri/gen/android/app/src/main/res/`

## The drawing and its copies

`web/static/favicon.svg` is the source of truth: the tile in `#1F4A38`, a cream gable and open book, and a jade flower at the apex, all on a 96 unit viewBox.
Two copies of that same drawing exist and are kept in step by hand.
`apps/landing/public/favicon.svg` is a byte copy, because the two trees build separately and a path served by the console's fallthrough would 404 under `just landing-dev`; the reasoning is recorded at the head of `apps/landing/src/marketplaces.js`.
`web/static/unreachable.html` embeds the drawing inline, because the desktop app opens that page from its own bundle with no stylesheet and no network.
`tokens.test.ts` holds `favicon.svg` and `unreachable.html` to the same set of four colours, so a recolour of one without the other fails the web lane.

The mark writes four colours.
Two of them, `#1F4A38` and `#127761`, are the values of `--primary` and `--accent`; the other two, `#F7F2E9` and `#F2E9DA`, are the mark's own light and the shade on the book's far page, and no token names them.
`tokens.test.ts` grants those two creams a named exception and retires the exception the moment a token takes one of the values.

Until 2026-09-06 the source was `web/static/email/teachouse-mark.svg`, with `web/static/email/teachouse-mark-small.svg` as its 16 pixel companion, and `favicon.svg` was produced by copying the first of them.
That copy step is now reversed: `favicon.svg` is the original, and copying `teachouse-mark.svg` over it would revert the console to Kauri.
`teachouse-mark-small.svg` is recoloured to Pounamu, its tile and flower taking `#1F4A38` and `#127761` with the creams and the geometry untouched, because `favicon-16.png` is rendered from it and a raster the console serves may not be in the retired palette.
`teachouse-mark.svg` is still Kauri: a mail client composes from it rather than from `tokens.css`, so recolouring it is its own change and a founder decision that has not been taken.
The `email/` exception in `tokens.test.ts` names the directory rather than either file and still holds, because the sweep retires such an exception only when the directory writes no colour at all, but its recorded reason now describes one drawing of the two.

## Regenerating the web console and landing icons

The renderer is resvg, which resolves the 96 unit viewBox to whatever `--width` and `--height` ask for.
It is on the profile path; `nix shell nixpkgs#resvg --command resvg ...` runs it anywhere.
Rendered with resvg 0.47.0 on 2026-09-06.

```
for n in 32 180 192 512; do
  resvg --width $n --height $n web/static/favicon.svg /tmp/mark-$n.png
done
resvg --width 16 --height 16 web/static/email/teachouse-mark-small.svg /tmp/mark-16.png
magick /tmp/mark-16.png  -depth 8 -strip       web/static/favicon-16.png
magick /tmp/mark-32.png  -depth 8 -strip       web/static/favicon-32.png
magick /tmp/mark-192.png -depth 8 -strip PNG32:web/static/icon-192.png
magick /tmp/mark-512.png -depth 8 -strip PNG32:web/static/icon-512.png
magick /tmp/mark-180.png -background '#1F4A38' -alpha remove -alpha off \
  -depth 8 -strip PNG24:web/static/apple-touch-icon.png
cp web/static/favicon.svg apps/landing/public/favicon.svg
```

The second ImageMagick pass exists only to fix each file's storage class, because resvg always writes RGBA and the set is not uniform: the two favicons are palette PNGs, `icon-192` and `icon-512` are truecolour with alpha, and `apple-touch-icon` is truecolour without, which is what an iOS home-screen icon must be.
`PNG32:` and `PNG24:` are what force the last three, since ImageMagick otherwise palettes them: resvg antialiases to well under 256 colours, so the reduction is lossless and would pass unnoticed.
Flattening the apple-touch icon onto the tile colour fills the corners the `rx="22"` tile leaves open, which is the square opaque icon iOS wants.

`favicon-32.png` is rendered from the full mark, and `favicon-16.png` from `teachouse-mark-small.svg`, which was drawn for that size.
At 16 pixels the full mark's thin roof stroke, curved pages and spine collapse into mush, so the companion thickens the roof, flattens the pages into two blocks and keeps the flower.
It stays under `email/`, and no code references either drawing there by name, so the rendering step above is what binds the file to the console.

The landing declares the SVG alone, in the head of `apps/landing/src/layouts/Base.astro`, and draws it again as an `img` in `SiteHeader.astro` and `SiteFooter.astro`.
It has no raster icons and no manifest, so the loop above writes nothing else for it.

`web/src/app.html` declares the PNG icons alone, and the absence of an SVG link is a decision rather than an omission:

```html
<link rel="icon" href="/favicon-32.png" sizes="32x32" type="image/png" />
<link rel="icon" href="/favicon-16.png" sizes="16x16" type="image/png" />
<link rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180" />
```

A tab icon is drawn at 16 pixels, and the full mark scaled down that far goes muddy.
`favicon-16.png` is rendered from the simplified small mark for exactly that reason.
An SVG link would hand that size back to the full mark, because a browser that understands the format prefers it, so the console declares the rasters and no SVG.
`favicon.svg` stays the source drawing every other artefact is rendered from, and the console still draws it directly at the sizes where it holds up: an `img` of 28 pixels in `+layout.svelte` and of 34 in `Console.svelte`.

`icon-192.png` and `icon-512.png` exist for a web manifest that has not been written yet, and nothing references them until it is.
When it is written, its `theme_color` and `background_color` take the values of `--primary` and `--ground` from `web/src/lib/styles/tokens.css`.

## Regenerating the desktop and Android icons

This set was regenerated from the current mark on 2026-09-06 by the commands below, on tauri-cli 2.11.4 and resvg 0.47.0.
`ic_launcher_background.xml` had already been edited directly to `#1F4A38`, and the run wrote that same value from `bg_color`, so the colour resource is produced by the pipeline now rather than held by hand.
Fifty files changed: thirty-five under `icons/`, seventeen of them tracked and eighteen the ignored iOS set, and fifteen mipmaps under `res/`.
`ic_launcher_background.xml` was rewritten as well, with the value it already held.
Nothing outside those two trees was written, and `tauri.conf.json` was not touched.

The Tauri CLI writes it from a manifest, given an opaque square source and a foreground already inset into the Android adaptive-icon safe zone.
The three inputs are written outside the repository and the manifest passed by absolute path, since the CLI resolves the manifest's own entries relative to itself and none of the three belongs in the tree.

```
resvg --width 1024 --height 1024 web/static/favicon.svg /tmp/mark-1024.png
magick /tmp/mark-1024.png -background '#1F4A38' -alpha remove -alpha off \
  -depth 8 -strip PNG32:app-icon.png
resvg --width 1024 --height 1024 app-icon-fg.svg android-fg.png
nix develop . --command sh -c 'cd apps/desktop/src-tauri && cargo tauri icon /abs/path/icon-manifest.json'
```

`icon-manifest.json`, whose paths are relative to itself:

```json
{
  "default": "app-icon.png",
  "bg_color": "#1F4A38",
  "android_fg": "android-fg.png",
  "android_fg_scale": 100
}
```

`app-icon-fg.svg`, the artwork of the mark with its tile removed, placed inside the adaptive-icon safe zone:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="108" height="108" viewBox="0 0 108 108">
  <g transform="translate(18,18) scale(0.75)">
    <path d="M15 46 L48 19 L81 46" fill="none" stroke="#F7F2E9" stroke-width="7" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="48" cy="19" r="5.5" fill="#127761" />
    <path d="M45 58 C39 54 30 52.5 22 54 L22 74 C30 72.5 39 74 45 78 Z" fill="#F7F2E9" />
    <path d="M51 58 C57 54 66 52.5 74 54 L74 74 C66 72.5 57 74 51 78 Z" fill="#F2E9DA" />
    <path d="M48 58 L48 78" fill="none" stroke="#127761" stroke-width="3" stroke-linecap="round" />
  </g>
</svg>
```

`bg_color` becomes the `ic_launcher_background` colour resource, so the adaptive icon's ground is the tile colour rather than the CLI's default white.
`android_fg` matters more: without it the CLI writes a full-bleed foreground that every Android 8 launcher crops to its middle two thirds, cutting off the roof ends and the outer pages.
The foreground above therefore carries the artwork alone, scaled by 0.75 and offset by 18 on a 108 unit canvas, which leaves the drawn content spanning 52 percent of the foreground and well inside the 72 of 108 mask.

`PNG32:` on the source matters for the reason it does in the web loop: ImageMagick palettes a 1024 pixel render, because resvg antialiases to about a hundred colours.
The 1024 pixel iOS icon is written at the source's own size and follows its storage class, so a palette source changes that one file's encoding, while every resized output is unaffected either way.
With the truecolour source, all thirty tracked rasters keep the dimensions, bit depth and PNG colour type they had, and `icon.ico` keeps its six PNG-compressed frames.

The one thing a regeneration does not reproduce is `icon.icns`, whose twelve OSType entries the CLI emits in a different order each run.
The entry set and the images in it are stable, so an icns diff of that shape is expected and carries no information on its own.

## What consumes them

`apps/desktop/src-tauri/tauri.conf.json` names `icons/32x32.png`, `icons/128x128.png` and `icons/icon.ico`, the bundlers also read `icon.icns` and the `Square*Logo.png` set, and `AndroidManifest.xml` reads `@mipmap/ic_launcher`.
The `icons/ios/` set is written on every run although no iOS project exists, and `apps/desktop/src-tauri/.gitignore` excludes it, so it never reaches a diff.
One limitation stands: the 16 pixel layer inside `icons/icon.ico` is the full mark downscaled by the CLI and reads muddy in the Windows title bar, and replacing it needs an ICO writer that keeps PNG-compressed frames, which ImageMagick is not.
