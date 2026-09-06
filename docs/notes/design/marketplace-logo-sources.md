# Marketplace logo sources

Where every logo on the Marketplaces page came from, so a later reader can check a mark against its source rather than trusting the file.

- date: 2026-09-05
- status: twenty marks land and are shown, every one with a source row; two more were fetched and are deliberately not landed; no browser or platform vendor's mark is landed at all, and the closing section records why; amended 2026-09-06, when eighteen of these files were copied to the public marketing site as well, which "On the public marketing page" below records
- decisions it implements: the founder's 2026-09-05 decision to show every marketplace's logo under a disclaimer, recorded in `docs/design/decisions.md`
- research it rests on: `marketplace-catalogue-1.md` and `marketplace-catalogue-2.md` in this directory

This note lives here rather than beside the images because `web/static/` is published unauthenticated by `tam-server`'s `ServeDir`.
A file in that directory is a public document, and this one reasons about brand rules we knowingly accept some risk against, which is not something to serve at a URL.
`web/static/marketplaces/` therefore holds image files and nothing else.

One row per file below: the marketplace, the URL it was retrieved from, and the date it was retrieved.
Retrieval dates are the day the file was written, not the day the catalogue recorded the URL.

The marks belong to their owners and are shown only to identify which marketplace an entry refers to, which is the nominative use catalogue one records the legal basis for.
The page carries a disclaimer beneath the grid saying exactly that, and every tile's mark and name open that marketplace's own front page, so a reader can always reach the owner rather than only read their name.

## The decision these files stand on

Several of these owners publish rules stricter than the law obliges them to allow, and two of them, Etsy and Shopify, require written permission before their logo is used at all.
Etsy's trademark policy: "DON'T use the official Etsy logo without permission."
Shopify's: "Use of our brand assets must be expressly authorized in writing."
The founder read that research and decided on 2026-09-05 to show every marketplace's logo under the disclaimer, accepting the risk and intending to approach each marketplace.
So the default is the owner's own mark, and the one exception below is an exception for a different reason than brand-guideline risk.

Shopify's brand page asks that web use of its assets "should include embedded hyperlinks to our homepage: www.shopify.com."
That condition is met: the Shopify tile's logo and name both link to https://www.shopify.com/, as every other marketplace tile links to its own front page.
The link is on every tile rather than on Shopify's alone, so meeting one owner's condition did not make one card behave unlike the other twenty.

## On the public marketing page, 2026-09-06

These files are no longer shown only behind a login.
Eighteen of them are copied to `apps/landing/public/marks/` and drawn in a band under the hero of the public site, under the founder's decision of 2026-09-06 in `../../design/decisions.md`.
The copies sit under `marks/` rather than under `marketplaces/`, which is what they are called here, because the landing build is answered ahead of the console and a directory sharing a name with a console route would let a later marketing page take a seller's Marketplaces screen.
That is a wider exposure than the 2026-09-05 decision this note implements, which was taken for the Marketplaces page and reasoned about a reader who has already signed up.
A marketing page is the surface a rights holder actually looks at, so it is written down here rather than left as a consequence of a decision about a different page.

Two of the twenty are deliberately not copied.
`etsy.png` and `shopify.png` stay in `web/static/marketplaces/` and reach the login-gated page only; the landing build holds no file for either, so the exclusion is a fact about the bytes that ship rather than only about the markup.
Both owners require written permission for logo use, quoted above, and each becomes a real mark on the public page on the day permission is reported.
Boom Learning is drawn as its initial and name there for the reason it is a wordmark here.

The band draws every mark greyscale at rest and in colour on hover.
The file is the owner's published bytes unaltered and a CSS filter changes the drawing rather than the file, which is the same distinction the render harness draws between changing a page and changing the moment it is photographed.
A rights holder could nonetheless read a greyscale rendering as alteration of their mark, so it is recorded rather than treated as a styling detail.

The copies are copies rather than links for the reason `apps/landing/public/favicon.svg` is a copy of `web/static/email/teachouse-mark.svg`: the two trees build separately, so a path that resolved through the console's `ServeDir` fallthrough under `tam-server` would 404 under `just landing-dev`, and the development render would disagree with the production one on exactly the thing the band adds.

## Retrieved and in use

| Marketplace | Source URL | Retrieved | File |
|---|---|---|---|
| TES | https://www.tes.com/themes/custom/tes_marketing/tes-192x192.png | 2026-09-05 | `tes-mark.png` |
| TPT | https://static1.teacherspayteachers.com/tpt-frontend/releases/production/current/822a28616093afa2ba8d.png | 2026-09-05 | `tpt-mark.png` |
| Etsy | https://www.etsy.com/apple-touch-icon-180x180.png | 2026-09-05 | `etsy.png` |
| Shopify | https://cdn.shopify.com/static/brand-assets/shopify-primary-logo.zip | 2026-09-05 | `shopify.svg`, superseded by `shopify.png` |
| Made By Teachers | https://media.madebyteachers.com/wp-content/uploads/2019/10/08121541/logo.jpg | 2026-09-05 | `made-by-teachers.jpg` |
| Classful | https://classful.com/wp-content/themes/tf/assets/img/brand/classful-logo.svg | 2026-09-05 | `classful.svg` |
| Teach Simple | https://teachsimple.com/images/brand-logo.svg | 2026-09-05 | `teach-simple.svg` |
| Amped Up Learning | https://cdn11.bigcommerce.com/s-wpgom64n7v/images/stencil/original/aul_logo_allblue_1783970958__34958.original.png | 2026-09-05 | `amped-up-learning.png` |
| Teacha! | https://cdn.teacharesources.com/wp-content/themes/marketica-wp-child/img/2022_Teacha-logo-light-blue.png | 2026-09-05 | `teacha.png`, superseded by `teacha.svg` |
| TeachShare | https://www.teachshare.com/icon.svg | 2026-09-05 | `teachshare.svg` |
| eduki | https://eduki.com/assets/eduki-logo-3.png | 2026-09-05 | `eduki.png`, cropped |
| TeachBuySell | https://sharetribe-assets.imgix.net/65567f8a-6664-4037-8e11-81741bb39240/raw/3f/9749f0e7b0576704db0ac797b84df9008db197?auto=format&fit=clip&h=96&w=640&s=b3503eba6dd4c53a5ed3100f76c5a053 | 2026-09-05 | `teachbuysell.png`, superseded: the file now holds the icon below |
| Teach Mzantsi | https://teachmzantsi.com/wp-content/uploads/2025/03/TM-Logo.png | 2026-09-05 | `teach-mzantsi.png` |
| Lesson Planned | https://lessonplanned.co.uk/wp-content/uploads/2020/04/cropped-logo-1-3-300x99.png | 2026-09-05 | `lesson-planned.png`, superseded: the file now holds the icon below |
| School Ninja | https://schoolninja.au/wp-content/uploads/2024/01/SchoolNinja-logo_stacked_small_400x400.png | 2026-09-05 | `school-ninja.png` |
| TPD | https://tpdedu.s3.ap-southeast-2.amazonaws.com/uploads/2021/11/02032046/tpd-logo.png | 2026-09-05 | `tpd.png`, superseded by `tpd.jpg` |
| Gumroad | https://assets.gumroad.com/images/logo-g.svg | 2026-09-05 | `gumroad.svg` |
| Payhip | https://payhip.com/images/designv2/logo/logo-large.svg | 2026-09-05 | `payhip.svg`, superseded by `payhip.png` |
| Sellfy | https://d369wu82uo9y4b.cloudfront.net/assets/images/favicon.svg | 2026-09-05 | `sellfy.svg` |
| Lemon Squeezy | https://cdn.prod.website-files.com/6347244ba8d63489ba51c08e/6347244ba8d63469e851c0d6_footer%20small%20logo.svg | 2026-09-05 | `lemon-squeezy.svg`, superseded by `lemon-squeezy.jpg` |

Every file above is the bytes the owner published, unchanged, with the three exceptions recorded here; a fourth note records why one URL differs from the one the catalogue found.

`shopify.svg` is one file taken out of the bundle the URL serves: it is `01 - Logo/svg/shopify_logo_whitebg.svg` from `shopify-primary-logo.zip`, the light-background primary logo, copied without alteration.
The light-background variant is the right one because the card it sits on is painted `--surface`.

`eduki.png` holds the published file cropped to the mark, at `463x158+266+342`, rather than the published bytes.
eduki publishes a 643 by 208 wordmark centred in a 1018 by 880 white canvas, so a tile fitting the whole canvas drew the wordmark nine pixels tall while every other mark on the grid drew at twenty-five or more.
The crop is a sub-rectangle and nothing else: no resize, no resample, no recolour, and `magick compare -metric AE` against the same rectangle of the original reports zero differing pixels, so every pixel kept is the owner's own.
The bounding box was read at a 2 per cent fuzz and is stable from 2 through 10 per cent, which is what says the crop stops at the mark's own ink rather than inside it; an exact-white trim stops short, because a near-white artefact sits in the canvas's upper left.
The white margin is safe to lose because this console defines one surface colour, `#fffdf9`, and has no dark theme, so the mark is never drawn on a ground its own plate was protecting it from.
The padded original is not on disk: the crop was written over `eduki.png` so the file keeps the name the tile guard requires and no unreferenced copy is served from a public directory.
It is recoverable from version control, and the source URL above refetches it.
`eduki-trimmed.png`, an intermediate copy of the same crop, is still in the directory, is referenced by nothing, and is on the founder's deletion list.

`teach-mzantsi.png` was published at 1253 by 706 pixels and 519 KB, which is over the 200 KB ceiling this set holds to.
It was resized proportionally to 400 pixels wide, 100 KB, and its metadata stripped.
Nothing about the mark itself changed: no crop, no recolour, no distortion.

The Etsy URL is a larger variant of the icon the catalogue recorded.
The catalogue found `apple-touch-icon.png`, which Etsy serves at 57 pixels square; `apple-touch-icon-180x180.png` is the same mark at 180, from the same host and path convention, and is what a 40-pixel tile needs on a high-density screen.

## Retrieved and not landed

| Marketplace | Source URL | Retrieved | File |
|---|---|---|---|
| Boom Learning | https://www.boomlearning.com/hubfs/New%20Website%2026/Boom%20Logo.svg | 2026-09-05 | `boom-learning.svg` |

Boom Learning is the single marketplace on the grid shown as a wordmark rather than as its own logo, and the reason is not the risk the founder accepted above.
Its guidelines, at https://helpcenter.boomlearning.com/using-boom-logos-badges-trademarks-and-service-marks, make a disclaimer mandatory wherever the mark is shown: "Boom™ is the trademark of Boom Learning. Used with permission."
We do not have permission, so printing that sentence would be a false statement about a relationship, which is a different thing from breaching a guideline.
Showing the mark without the sentence breaches the rule that governs it, and showing it with the sentence says something untrue, so the tile renders the name in our own typeface instead.

The file is therefore not landed either.
An earlier draft of this note kept it in `web/static/marketplaces/` as evidence, which contradicted the paragraph two sections above: that directory is public, so keeping the one mark the page refuses to display where anyone can fetch it by name defeats the refusal.
The URL above is the evidence, and it is enough — granted permission makes showing the mark a one-line change either way.
`web/static/marketplaces/tpt-wordmark.svg` is not landed for a plainer reason: nothing in the repository references it.

## TES and TPT

These two are the pair the page leads with, and their marks reached this directory by a different route from every other row.
Neither marketplace was contacted for the research, by instruction, so neither catalogue describes them and neither recorded a logo or an address for them; nothing in either catalogue was read off either site.
The lead placed `tes-mark.png` and `tpt-mark.png` from the two homepages, taking each site's own icon: the 192 by 192 icon `tes.com` links as its icon, and the mark `teacherspayteachers.com` serves from its own asset host.
Both files are byte-identical copies of what those URLs serve, which is checkable without fetching anything: `tes-mark.png` is 2,225 bytes at `e5627ad4e91b…`, `tpt-mark.png` is 2,920 bytes at `840adfd2877c…`, and both match here.
They are used under the same disclaimer as every other mark on the page.

`tpt-wordmark.svg` came from that same asset host and is not landed, because nothing references it.

An earlier version of this note said the three "predate this note and carry their own provenance".
That was wrong — `git ls-tree -r main` shows `web/static/marketplaces/` does not exist on `main`, so all three arrived with this change — and the rows above are what it should have said.

The two tiles link to https://www.tes.com/ and https://www.teacherspayteachers.com/, which are the addresses this repository already holds in its adapters, at `crates/tam-domain/src/registry/listing_url.rs`.

## Square icons for the tiles

Added 2026-09-05, after the rows above.
The founder decided the marketplace tiles should return toward the reference design's small square tiles, and a small square tile needs a square icon.
Twelve of the marks above are wide wordmarks, which a square tile can only show by shrinking to illegibility or by cropping, and cropping someone's wordmark is an alteration of their mark rather than a use of it.
So each of those twelve was checked for a square icon the marketplace itself publishes, looking in one order: the `apple-touch-icon` link on the home page, a `link rel="icon"` of 128 pixels or more, `/apple-touch-icon.png`, and `/favicon.ico` only as a last resort.
An icon was accepted only if it is square or within ten percent of square, at least 128 pixels on the short side, and visibly that marketplace's own mark.
Nine marketplaces publish one; three do not, and are recorded as such below rather than given a cropped wordmark or an icon from a third-party icon service.

Every file below is the bytes its owner serves at the URL beside it, unaltered: no crop, no resize, no recolour, no metadata strip.

Each was fetched as `<slug>-icon.<ext>` and then became the file its tile shows, because this directory holds one rule: the file a tile shows is named for the tile, and nothing unreferenced is served from it.
Where the icon's extension matched the wordmark's, the wordmark's bytes were overwritten and the name is unchanged; where it differed, the icon was written as `<slug>.<newext>` and the wordmark file is superseded.
Every copy was checked with `magick compare -metric AE` against the file it came from and reports zero differing pixels, so what a tile shows is the owner's bytes whichever name they arrived under.
The nine `-icon` files themselves are now redundant copies and are on the founder's deletion list.
They are shown under the same disclaimer as every other mark on the page, for the same reason: to identify which marketplace a tile refers to.

| Marketplace | Source URL | Retrieved | Size | File | SHA-256 |
|---|---|---|---|---|---|
| Shopify | https://www.shopify.com/favicon.ico | 2026-09-05 | 256x256 | `shopify.png`, unwrapped from the fetched `shopify-icon.ico` | `a7604e40950e90c724e7a4dd3428045f089545400c6473ec4ef667541d27def6` |
| Teacha! | https://cdn.teacharesources.com/wp-content/themes/marketica-wp-child/assets/img/teacha-apple.svg | 2026-09-05 | 330x330 | `teacha.svg`, copied from the fetched `teacha-icon.svg` | `abb596c47613974ca68a263168b785381324ddfdbd8f349e870664417abddd0d` |
| eduki | https://eduki.com/assets/eduki_logo_180x180px.png | 2026-09-05 | 181x180 | `eduki-icon.png`, fetched and not used | `b076c9a103a771d9ba5d88c57c23f2c60911ea9e533c67835a898abbe1acc432` |
| TeachBuySell | https://teachbuysell.com.au/static/icons/apple-touch-icon.png | 2026-09-05 | 512x512 | `teachbuysell.png`, copied from the fetched `teachbuysell-icon.png` | `95f1c53ccba56c8e52a061d5e442e8d42652c9d484531c78f6d71263449b7b8c` |
| Teach Mzantsi | https://teachmzantsi.com/wp-content/uploads/2025/11/cropped-IMG_20250920_180122_402-180x180.webp | 2026-09-05 | 180x180 | `teach-mzantsi-icon.webp`, fetched and not used | `9e70dd1f6a9483e5dc1c1fb70ac57696b525a5d740fc6fabb0d7b8dd33ef98a7` |
| Lesson Planned | https://lessonplanned.co.uk/wp-content/uploads/2020/04/cropped-LOGO-ICON-180x180.png | 2026-09-05 | 180x180 | `lesson-planned.png`, copied from the fetched `lesson-planned-icon.png` | `354d7272cf74c708df0344b36a907341ca5dd8a40ed983287a5329c3f7311ee4` |
| TPD | https://tpdedu.s3.ap-southeast-2.amazonaws.com/uploads/2023/04/02001255/cropped-Favicon-180x180.jpg | 2026-09-05 | 180x180 | `tpd.jpg`, copied from the fetched `tpd-icon.jpg` | `562c9eed24eeb2e75b1d209f1b236afdc57e428bd9b07cae247a1d1a00e6d5e2` |
| Payhip | https://payhip.com/images/designv2/favicon/favicon-196x196.png | 2026-09-05 | 196x196 | `payhip.png`, copied from the fetched `payhip-icon.png` | `c4ad88221dfadf906c081dcf4f601f3dabbe44c7dc933987075238a84bba9488` |
| Lemon Squeezy | https://cdn.prod.website-files.com/6347244ba8d63489ba51c08e/6358e75cbf1bca262b2b2edc_webclip.jpg | 2026-09-05 | 256x256 | `lemon-squeezy.jpg`, copied from the fetched `lemon-squeezy-icon.jpg` | `fed4e7aceb65a6b5c226452bba74cc302f697e479ff55b6f4195e862a6b44c70` |

Four of those rows need a word about where the file came from or what it shows.

Shopify's home page links `apple-touch-icon` at 120 pixels, eight short of the bar, so this one came from the last resort in the order.
`https://www.shopify.com/favicon.ico` is an ICO holding a single PNG-encoded frame at 256 by 256, which is the green shopping-bag glyph rather than the wordmark already landed as `shopify.svg`.
The `.ico` extension is the format the file actually is; it is served as `image/x-icon`.

Teacha! links its own 48 by 49 favicon as its `apple-touch-icon`, which is under the bar.
`teacha-apple.svg` is the same apple-with-a-heart shape as that favicon, drawn as vector in the brand green rather than the favicon's blue, and it sits in Teacha!'s own child-theme asset directory.
The shape was compared against their favicon before accepting it, because the page also uses this file as a generic store icon and the filename alone would not have settled whether it is their mark.

Teach Mzantsi's square icon is a promotional image cropped to square by its owner, carrying the words "Teach Mzantsi" and the site address, rather than a clean glyph.
It is what they publish as their own `apple-touch-icon`, and it is their bytes untouched, but at tile size the address beneath the name will not be readable.

eduki's square file sets the eduki wordmark inside a square canvas rather than reducing the brand to a glyph, because that is the icon eduki publishes.

Seven of the nine square files are shown. Two are not, and the page states why beside each tile.

`eduki-icon.png` is not used because its ink measures 131 by 46, an aspect ratio of 2.85: it is the eduki wordmark on a square canvas rather than a glyph, so a square tile would draw the word nine pixels tall, which is the defect the eduki crop above exists to remove.
eduki therefore keeps its wordmark in the wide tile, where its 463 by 158 draws 78 by 26.6 in a 78 by 38 box.
`teach-mzantsi-icon.webp` is not used by the founder's design call: it is a cropped promotional photograph carrying an address line that cannot be read at tile size, so Teach Mzantsi keeps its wordmark too.

Shopify's icon took one extra step. `shopify.png` is `shopify-icon.ico` unwrapped: the ICO holds a single PNG-encoded frame at 256 by 256, so extracting it changes no pixel, and it keeps this directory to the three formats its guard allows rather than admitting a fourth for one file.
The hash in its row is `shopify.png`'s own, which is the one row in this table where the container fetched and the file shipped genuinely differ in bytes: the ICO hashes to `a0d9f7b8adf1a699a94c7d8381d7a47a7a7fa810faebe8a67e7b9aab525546d3`, and checking a shipped file against that would fail for a file that is correct.

### The three with no usable square icon

| Marketplace | Largest square icon it publishes | Checked |
|---|---|---|
| Classful | 48x48, in `https://classful.com/favicon.ico` and the theme copy at `https://classful.com/wp-content/themes/tf/assets/img/favicon.ico`; `/apple-touch-icon.png` is 404 | 2026-09-05 |
| Teach Simple | 16x16, at `https://teachsimple.com/favicon.png`; `/apple-touch-icon.png` and `/favicon.ico` are both 404 | 2026-09-05 |
| Amped Up Learning | 48x48, at `https://cdn11.bigcommerce.com/s-wpgom64n7v/product_images/AUL_Bolt_Square_48px.png`, linked as `rel="shortcut icon"`; `/apple-touch-icon.png` and `/favicon.ico` are both 404 | 2026-09-05 |

Each of these three publishes a square mark, and each publishes it only far below the 128-pixel bar; Amped Up Learning's own filename records the size it was uploaded at.
Upscaling one would invent detail its owner never published, and cropping the wide wordmark already landed would alter the mark, so all three keep the wordmark they have.
Whichever way the tiles handle a marketplace with no square icon, it is these three that meet it.

## Browser and platform marks, sought and not taken

Added 2026-09-06, after everything above.
The founder asked for two more things on this page: a store badge on each of the three download tiles, and the vendor's own logo on the Chrome and Firefox tiles.
Neither landed, no file was added to `web/static/`, and this section is the record of why, so the question is not reopened from memory.

Nothing in this section was fetched from a marketplace.
The only hosts contacted were the two browser vendors' own brand addresses named below.

### The three store badges

Google Play, the App Store and the Microsoft Store each publish a badge and each license it for one purpose: to link to that product's listing on that store.
None of the three download tiles links to a listing.
The Windows and Android cards offer a file the release manifest names, installed by hand, and the Apple card offers nothing at all, because no macOS build is published.
A "Get it on Google Play" badge over a sideloaded `.apk` is outside the licence that grants the badge, and it tells the reader something untrue about where the file came from.
That is a different thing from the brand-guideline risk the founder knowingly accepted on 2026-09-05 for the marketplace logos: there the mark identifies a marketplace correctly and breaches a guideline, here the mark would state a fact that is not so.

So the three tiles draw a neutral glyph of our own instead, from the Lucide set already approved as a dependency: `monitor`, `smartphone` and `laptop`.
A glyph is a drawing rather than a logo, claims no relationship, and needs no provenance row, which is why this section has no table.
`PLATFORM_MARK` in `web/src/lib/pages/marketplaces/downloads.ts` is typed as a `Mark` rather than as a glyph name, so the day a store listing exists the badge lands as a change to that table and to nothing else, and `downloads.test.ts` fails on a change from a glyph to an image, which is the point at which somebody has to have read the badge licence.

### The two browser logos

Neither vendor publishes its browser logo at an address that can be fetched, which is a fact about the sources rather than a decision of ours.

`https://about.google/brand-resource-center/` redirects to `partnermarketinghub.withgoogle.com/brands/google/overview/`, whose own navigation links `/brands/chrome/overview/brand-introduction/`.
That address, and `/brands/chrome/` with it, redirect to a login, so Chrome's logo and its published usage terms are both behind a Google partner account.
`https://www.google.com/chrome/branding/`, the other address on record for Chrome brand assets, is a 404.
All three were checked on 2026-09-06.

`https://mozilla.design/firefox/` redirects to `https://brand.mozilla.com/all-brands`, a Frontify portal that serves the same JavaScript shell at every path under it, `/firefox` and `/sitemap.xml` included, with no asset address anywhere in the markup.
The Firefox logo is reachable only by driving that application in a browser.
Checked the same day.

Mozilla's trademark policy expressly permits nominative use of its marks to refer to Mozilla products without prior permission, which makes Firefox the least exposed of the five marks in this section and the only one whose owner publishes such an allowance.
That is a fact about permission rather than about availability, and it does not make the file any easier to obtain.

So both browser tiles keep the name set in our own typeface, which is what they already drew.
A mark whose licence terms cannot be read cannot be landed under the rule the rest of this note holds every file to, and a partner portal is what makes them unreadable.

A glyph of ours was the alternative, as the download tiles take, and it is not the right answer here.
Lucide carries no brand marks at all, so the nearest it offers either browser is a generic drawing, and a circular one resembles Chrome's own logo more closely than the word "Chrome" does.
A mark adopted to avoid using a logo must not be the closer imitation of it, and that asymmetry is the whole reason the glyph is right on the download tiles and wrong on these two: a monitor, a phone and a laptop resemble no store badge.

### What this leaves unchanged, deliberately

`web/static/` gains no file, so nothing new is published unauthenticated by `ServeDir` and the rule at the top of this note still holds.
The guard in `catalogue.test.ts` requiring every mark to be served from `/marketplaces/` on our own origin is untouched and stays exactly as strong as it was; there is no second directory to admit.
The disclaimer beneath the grid still says "marketplace names and logos", and still says only that, because no browser or platform vendor's mark appears under it.
Every mark on this page that is not a marketplace's is a drawing of ours, so extending that sentence to browser and platform vendors would have claimed we show something we do not.
