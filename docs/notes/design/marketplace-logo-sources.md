# Marketplace logo sources

Where every logo on the Marketplaces page came from, so a later reader can check a mark against its source rather than trusting the file.

- date: 2026-09-05
- status: twenty marks land and are shown, every one with a source row; two more were fetched and are deliberately not landed
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

## Retrieved and in use

| Marketplace | Source URL | Retrieved | File |
|---|---|---|---|
| TES | https://www.tes.com/themes/custom/tes_marketing/tes-192x192.png | 2026-09-05 | `tes-mark.png` |
| TPT | https://static1.teacherspayteachers.com/tpt-frontend/releases/production/current/822a28616093afa2ba8d.png | 2026-09-05 | `tpt-mark.png` |
| Etsy | https://www.etsy.com/apple-touch-icon-180x180.png | 2026-09-05 | `etsy.png` |
| Shopify | https://cdn.shopify.com/static/brand-assets/shopify-primary-logo.zip | 2026-09-05 | `shopify.svg` |
| Made By Teachers | https://media.madebyteachers.com/wp-content/uploads/2019/10/08121541/logo.jpg | 2026-09-05 | `made-by-teachers.jpg` |
| Classful | https://classful.com/wp-content/themes/tf/assets/img/brand/classful-logo.svg | 2026-09-05 | `classful.svg` |
| Teach Simple | https://teachsimple.com/images/brand-logo.svg | 2026-09-05 | `teach-simple.svg` |
| Amped Up Learning | https://cdn11.bigcommerce.com/s-wpgom64n7v/images/stencil/original/aul_logo_allblue_1783970958__34958.original.png | 2026-09-05 | `amped-up-learning.png` |
| Teacha! | https://cdn.teacharesources.com/wp-content/themes/marketica-wp-child/img/2022_Teacha-logo-light-blue.png | 2026-09-05 | `teacha.png` |
| TeachShare | https://www.teachshare.com/icon.svg | 2026-09-05 | `teachshare.svg` |
| eduki | https://eduki.com/assets/eduki-logo-3.png | 2026-09-05 | `eduki.png` |
| TeachBuySell | https://sharetribe-assets.imgix.net/65567f8a-6664-4037-8e11-81741bb39240/raw/3f/9749f0e7b0576704db0ac797b84df9008db197?auto=format&fit=clip&h=96&w=640&s=b3503eba6dd4c53a5ed3100f76c5a053 | 2026-09-05 | `teachbuysell.png` |
| Teach Mzantsi | https://teachmzantsi.com/wp-content/uploads/2025/03/TM-Logo.png | 2026-09-05 | `teach-mzantsi.png` |
| Lesson Planned | https://lessonplanned.co.uk/wp-content/uploads/2020/04/cropped-logo-1-3-300x99.png | 2026-09-05 | `lesson-planned.png` |
| School Ninja | https://schoolninja.au/wp-content/uploads/2024/01/SchoolNinja-logo_stacked_small_400x400.png | 2026-09-05 | `school-ninja.png` |
| TPD | https://tpdedu.s3.ap-southeast-2.amazonaws.com/uploads/2021/11/02032046/tpd-logo.png | 2026-09-05 | `tpd.png` |
| Gumroad | https://assets.gumroad.com/images/logo-g.svg | 2026-09-05 | `gumroad.svg` |
| Payhip | https://payhip.com/images/designv2/logo/logo-large.svg | 2026-09-05 | `payhip.svg` |
| Sellfy | https://d369wu82uo9y4b.cloudfront.net/assets/images/favicon.svg | 2026-09-05 | `sellfy.svg` |
| Lemon Squeezy | https://cdn.prod.website-files.com/6347244ba8d63489ba51c08e/6347244ba8d63469e851c0d6_footer%20small%20logo.svg | 2026-09-05 | `lemon-squeezy.svg` |

Every file above is the bytes the owner published, unchanged, with the two exceptions recorded here; a third note records why one URL differs from the one the catalogue found.

`shopify.svg` is one file taken out of the bundle the URL serves: it is `01 - Logo/svg/shopify_logo_whitebg.svg` from `shopify-primary-logo.zip`, the light-background primary logo, copied without alteration.
The light-background variant is the right one because the card it sits on is painted `--surface`.

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
