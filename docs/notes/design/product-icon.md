# The product icon

Every icon the product shows, on the web console, in the desktop client and on Android, is derived from one drawing.

- date: 2026-09-05
- status: every artefact generated and checked at every size, and the mark itself is the founder's choice and is not to be redrawn; the four link elements below are still owed to `web/src/app.html`
- source: `web/static/email/teachouse-mark.svg`, with `web/static/email/teachouse-mark-small.svg` as its 16 pixel companion
- paths: `web/static/`, `apps/desktop/src-tauri/icons/`, `apps/desktop/src-tauri/gen/android/app/src/main/res/`

## The two drawings

`teachouse-mark.svg` is the source of truth: the rimu tile in `#6B4423`, a cream gable and open book, and a pōhutukawa flower at the apex, all on a 96 unit viewBox.
`teachouse-mark-small.svg` is a simplified companion used only at 16 pixels, where the full mark's thin roof stroke, curved pages and spine collapse into mush; it thickens the roof, flattens the pages into two blocks separated by a brown gap, and keeps the flower.
Both render through ImageMagick's librsvg delegate, where `-density N` against that 96 unit viewBox yields exactly N by N pixels.

## Regenerating the web console icons

```
cp web/static/email/teachouse-mark.svg web/static/favicon.svg
magick -background none -density 512 web/static/email/teachouse-mark.svg -depth 8 -strip web/static/icon-512.png
magick -background none -density 192 web/static/email/teachouse-mark.svg -depth 8 -strip web/static/icon-192.png
magick -background none -density 32  web/static/email/teachouse-mark.svg -depth 8 -strip web/static/favicon-32.png
magick -background none -density 16  web/static/email/teachouse-mark-small.svg -depth 8 -strip web/static/favicon-16.png
magick -background none -density 180 web/static/email/teachouse-mark.svg \
  -background '#6B4423' -alpha remove -alpha off -depth 8 -strip web/static/apple-touch-icon.png
```

The apple-touch icon alone is flattened onto the tile colour and carries no alpha channel, which is what an iOS home-screen icon must be.
`favicon.svg`, `favicon-32.png`, `favicon-16.png` and `apple-touch-icon.png` are declared by four link elements in the head of `web/src/app.html`:

```html
<link rel="icon" href="/favicon.svg" type="image/svg+xml" />
<link rel="icon" href="/favicon-32.png" sizes="32x32" type="image/png" />
<link rel="icon" href="/favicon-16.png" sizes="16x16" type="image/png" />
<link rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180" />
```

`icon-192.png` and `icon-512.png` exist for a web manifest that has not been written yet, and nothing references them until it is.

## Regenerating the desktop and Android icons

The Tauri CLI writes that whole set from a manifest, given an opaque square source and a foreground already inset into the Android adaptive-icon safe zone.

```
magick -background none -density 1024 web/static/email/teachouse-mark.svg \
  -background '#6B4423' -alpha remove -alpha off -depth 8 -strip app-icon.png
magick -background none -density 2048 app-icon-fg.svg -resize 1024x1024 -depth 8 -strip android-fg.png
nix develop . --command sh -c 'cd apps/desktop/src-tauri && cargo tauri icon /abs/path/icon-manifest.json'
```

`icon-manifest.json`, whose paths are relative to itself:

```json
{
  "default": "app-icon.png",
  "bg_color": "#6B4423",
  "android_fg": "android-fg.png",
  "android_fg_scale": 100
}
```

`app-icon-fg.svg`, the artwork of the mark with its tile removed, placed inside the adaptive-icon safe zone:

```svg
<svg xmlns="http://www.w3.org/2000/svg" width="108" height="108" viewBox="0 0 108 108">
  <g transform="translate(18,18) scale(0.75)">
    <path d="M15 46 L48 19 L81 46" fill="none" stroke="#F7F2E9" stroke-width="7" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="48" cy="19" r="5.5" fill="#C2543A" />
    <path d="M45 58 C39 54 30 52.5 22 54 L22 74 C30 72.5 39 74 45 78 Z" fill="#F7F2E9" />
    <path d="M51 58 C57 54 66 52.5 74 54 L74 74 C66 72.5 57 74 51 78 Z" fill="#F2E9DA" />
    <path d="M48 58 L48 78" fill="none" stroke="#C2543A" stroke-width="3" stroke-linecap="round" />
  </g>
</svg>
```

`bg_color` becomes the `ic_launcher_background` colour resource, so the adaptive icon's ground is the tile brown rather than the CLI's default white.
`android_fg` matters more: without it the CLI writes a full-bleed foreground that every Android 8 launcher crops to its middle two thirds, cutting off the roof ends and the outer pages.
The foreground above therefore carries the artwork alone, scaled by 0.75 and offset by 18 on a 108 unit canvas, which leaves the drawn content spanning 52 percent of the foreground and well inside the 72 of 108 mask.

## What consumes them

`apps/desktop/src-tauri/tauri.conf.json` names `icons/32x32.png`, `icons/128x128.png` and `icons/icon.ico`, the bundlers also read `icon.icns` and the `Square*Logo.png` set, and `AndroidManifest.xml` reads `@mipmap/ic_launcher`.
The `icons/ios/` set is written on every run and kept, so that a regeneration leaves no spurious diff, although no iOS project exists.
One limitation stands: the 16 pixel layer inside `icons/icon.ico` is the full mark downscaled by the CLI and reads muddy in the Windows title bar, and replacing it needs an ICO writer that keeps PNG-compressed frames, which ImageMagick is not.
