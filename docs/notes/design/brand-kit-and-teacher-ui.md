# The Teachouse brand kit and the teacher-facing form

Redrawing every surface on the founder's brand kit, rebuilding the landing page to the founder's mockup, and rewriting the new-resource form for teachers.

- date: 2026-09-11
- status: design accepted by the founder on 2026-09-11 (six choices, recorded below); implementation in flight across web, landing, backend, desktop and Android
- inputs: `~/downloads/Teachouse/UI Changes.pdf` (seven annotated pages), `Feature Suggestions.pdf`, `Landing Page Mockup 1.png` (the landing mockup and the brand kit panel), `ChatGPT.png` and `ChatGPT-2.png` (logo sheets)
- supersedes: the palette section of `console-redesign-plan.md`, the mark in `product-icon.md`, the copy and tokens of `landing-page.md`, the tile rule in `marketplace-logo-sources.md` for the three authorable marks, and the 2026-09-05 pricing carried in `apps/landing/src/pricing.js`
- paths: `web/`, `apps/landing/`, `apps/desktop/src-tauri/`, `crates/tam-api/`, `crates/tam-taxonomy/`, `crates/tam-authoring/`, `docs/`

## The six founder choices of 2026-09-11

1. Logo: the `UI Changes.pdf` logo everywhere — "TeacHouse" with the house as the H and the "TEACH. CREATE. INSPIRE." tagline; its house alone, on an indigo tile, is the favicon and the app icon.
2. Pricing: the mockup's pricing replaces the 2026-09-05 tiers — a one-off Catalogue Import ladder ($47 up to 20, $77 up to 50, $127 up to 100, $247 up to 250), a Teachouse Subscription (monthly or annual), and a Founding 100 offer (25% off year one, 20% off ongoing, first 20 resources imported free, 100 places).
3. Imagery: the mockup's photo and illustration are cropped out of the mockup and shipped as placeholders in the exact slots, until the founder supplies originals.
4. Testimonials: the band is built and styled but renders nothing while its list is empty; no invented quote ships.
5. Preview generation runs client-side with `pdfjs-dist` and `pdf-lib`, two web dependencies the founder approved for this purpose.
6. Navigation uses the founder's words: Import, Crosslist, Automations, with Marketplaces and Account.

## Brand

Palette, exactly as the kit states it.

| token | hex | kit name | use |
|---|---|---|---|
| `--primary` | `#1E2A5A` | Indigo | headings, primary buttons, rail, tile ground |
| `--accent` | `#00B894` | Teal | secondary actions, ticks, the house |
| `--lavender` | `#C8B4FF` | Lavender | accent surfaces |
| `--peach` | `#FFD6B3` | Peach | accent surfaces |
| `--ground` | `#F8FAF8` | Cream | page background |
| `--text` | `#2D3748` | Charcoal | body text |
| `--muted` | `#64748B` | Slate | secondary text |
| `--ok` | `#10B981` | Success | positive |
| `--warn` | `#F59E0B` | Warning | alerts |
| `--bad` | `#EF4444` | Error | errors |

Type is Poppins for headings (H1 to H3) and Inter for body, both self-hosted because the desktop and Android content-security-policy admits no third-party origin.
Buttons are pills: the primary is solid indigo with white text, the secondary is an indigo outline.
The success banner, the search field and the select in the kit's UI elements panel are the shapes the console's Banner, search and select take.

The kit's Charcoal is the text colour and Indigo the heading colour, which is how the mockup sets them.

### Assets

Traced from the PDF logo at three times its 1377 by 459 raster, one potrace pass per colour, and recoloured to the kit's Indigo and Teal.

| file | what |
|---|---|
| `web/static/brand/logo.svg` | the full logo with tagline |
| `web/static/brand/wordmark.svg` | the wordmark without the tagline, for headers |
| `web/static/brand/house.svg` | the house alone, teal |
| `web/static/brand/mark.svg` | the house on an indigo tile, the favicon and app icon |
| `web/static/brand/mark-small.svg` | the same with the window closed, for 16 and 32 pixel renders |
| `web/static/fonts/poppins-{400,500,600,700}-{latin,latin-ext}.woff2` | Poppins v23 from `fonts.gstatic.com`, 2026-09-11 |
| `web/static/fonts/inter-400-700-{latin,latin-ext}.woff2` | Inter v20 variable, same source and date |

`apps/landing/public/` carries byte copies of the brand files and the fonts it uses.
The three authorable marketplace marks are now SVG, traced from the PNG favicons already landed under `marketplace-logo-sources.md`: `web/static/marketplaces/{tes-mark,tpt-mark,etsy}.svg`.

## Landing page

Rebuilt section for section from the mockup.

1. Header: wordmark, How it works, Pricing, About, Resources, Log in, Join the Founding 100.
2. Hero: eyebrow "THE HOME FOR TEACHER CREATORS", "Create once. Sell everywhere.", the sell line, two buttons, the Founding line, "Built for creators selling on" with the marketplace marks, the photo with the "Your catalogue" card and the handwritten "More reach. More sales. Less admin."
3. The challenge: "You already did the hard part." on the peach band with the illustration.
4. The solution: "You don't need more resources. You need more from your resources." on the mint band, Import, Distribute, Manage, and the console card.
5. Pricing: "Two ways to get started", the import ladder, the subscription, and the Founding 100 card, all from one `pricing.js`.
6. Testimonials, hidden while empty.
7. Footer.

About links to the challenge section and Resources to the marketplace band, since neither has a page of its own.
The landing copy gate that allowed one sentence naming a marketplace is widened to the bands the mockup draws them in, and the 2026-09-06 decision that the public copy names no marketplace is superseded by the founder's mockup.

## Console

### Shell

Rail sections and their one-line hints, in the founder's words.

| section | hint |
|---|---|
| Import | Bring your current portfolio to Teachouse from anywhere it is housed. |
| Crosslist | Publish your resources to multiple marketplaces. |
| Automations | Edit tags, descriptions, titles and files across your listings. |
| Marketplaces | Connect the places you sell. |
| Account | Your settings, plan and notifications. |

The logo replaces the old mark in the rail and on the sign-in, sign-up and reset screens.

### The new-resource form, in order

1. Marketplaces, at the top: every authorable marketplace as an evenly spaced icon tile with a checkbox, the full name on hover and for screen readers, no dropdown, and a single Tes tile.
2. Name.
3. Files: one drop zone that takes several files by drag and drop or browse; the ZIP option stays.
4. Preview: its own drop zone for a preview file, and "Make a preview from your file", which opens a page picker over the uploaded PDF, lets the teacher tick pages, optionally writes a diagonal watermark of the seller's name across each page, and stores the result as the preview.
5. Thumbnails, labelled as TPT's layout.
6. Description, with a small formatting bar (bold, italic, lists) that the marketplaces render in their own format, and a counter that says characters.
7. Price, in plain words, with a recommendation under the bundle discount.
8. Categories, with an American / British toggle above the grade grid; the teacher picks in one and Teachouse translates.
9. Education standards, with the licence attributions folded behind a "Sources" disclosure.
10. Details, without the answer-key note.
11. Marketplace-specific options, each panel headed with the marketplace's icon and "TPT only" or "Tes only": copyright, tax code and localization for TPT; licence and curriculum for Tes.
12. Product status: "Draft, visible only to you" or "Make listing active on my selected marketplaces".
13. Errors, headed "These are the errors that need to be fixed. Click one to go to that section.", each a button.
14. Create listing.

The text below the errors ("Nothing is sent to a marketplace yet. 0 files · kept here") is gone.
The video preview box the PDF asks for is deliberately not built: the founder set it aside on 2026-09-11.

### Wording

Every sentence the PDF marks is rewritten, and every hint on the form is rewritten for a teacher.
The replacements are held in the code beside the fields they explain; the rule is one short sentence that says what to do, and no sentence that explains why a rule exists unless the teacher has to act on it.
"Shelf" becomes "category" everywhere; "Localisation" becomes "Localization".

### One Tes

The backend already has one `Marketplace::Tes`; the three inventories `TesGb`, `TesUs` and `TesNz` are the regions Tes runs as separate catalogues with their own currency, age field and taxonomy tree, and the M1 GB-to-NZ duplication is a sync request between two of them.
The founder's rule is that the form shows one Tes.
So the region moves into the Tes-only panel as "Curriculum", the same field Tes's own upload form asks, with England, United States and New Zealand as ticks.
Ticking Tes and one curriculum publishes to that inventory; ticking two publishes to both.
Nothing in the model changes, and the decision record of 2026-08-25 that market targeting is a curriculum field on one account is what this implements.

### British and American grades

TPT's grades are the canonical base, and the crosswalk deliberately never inverts an age band into a year.
The founder's rule of 2026-09-11 is a declared table, not an inference: Reception is Pre-K, Year 1 is Kindergarten, Year N is Grade N minus 1 through Year 13 as 12th Grade, and the bands read Primary School for Elementary, Secondary School for Middle School, College for High School and University for College.
The API serves each grade facet with a British label beside its American one, and the toggle relabels the grid; the selection underneath is one set.

### Preview

The console renders the uploaded PDF's pages with `pdfjs-dist`, the teacher ticks pages, and `pdf-lib` copies those pages into a new document and, when asked, draws the seller's name diagonally across each at low opacity.
The result is uploaded through the existing upload route and attached as the product's preview, within TPT's 30 MB preview cap.
This runs on the seller's device, which is where D27 wants the bytes.
TPT's preview slot has never been exercised by a capture, so the preview is stored and shown in Teachouse today and reaches TPT once that slot is captured; the capture is the named follow-up.

## Desktop and Android

The console reaches both through the webview with no client release.
What does need one: the regenerated icon set from `web/static/brand/mark.svg`, the adaptive-icon ground `#1E2A5A`, `unreachable.html` on the new palette, and the bundle descriptions reworded.

## What this changes in the other notes

- `console-redesign-plan.md`: the palette paragraph and slice S1 are superseded; S6 to S12 build on these tokens.
- `product-icon.md`: the mark is the brand-kit house on an indigo tile; the regeneration procedure stands.
- `landing-page.md`: copy, tokens and pricing are replaced; the placeholders the founder owes are unchanged.
- `marketplace-logo-sources.md`: the three authorable marks are traced to SVG from the landed PNGs; the sources are unchanged.
- `decisions.md`: entries for the brand kit, the one-Tes presentation rule, the Year-to-Grade table, the two web dependencies, the pricing, and the public copy naming marketplaces.
