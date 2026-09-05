# Vendoo for teachers: the rethink

- date: 2026-09-02, decisions recorded 2026-09-03
- status: decisions taken. The founder answered D1 to D27 across four rounds on 2026-09-02 and 2026-09-03, and D28 to D30 are recorded here; nothing is built, no marketplace has been contacted, and the build awaits an explicit founder go on the plan
- inputs: the six research notes of 2026-09-02 and the six of 2026-09-03 under `docs/research/rethink/`, the last of them `tpt-create-form-dom.md`, and the landed notes `docs/notes/design/client-side-architecture.md`, `docs/notes/legal/marketplace-terms-assessment.md`, `docs/notes/design/creation-flow.md`, `platform-linking.md`, `analytics-console.md`, `admin-backoffice.md`, `billing-vendor-memo.md`

This memo turns six research notes into one plan, one recommendation, and a numbered list of decisions only the founder can take.
It answers the brief in the order the brief asks: what "Vendoo for teachers" means once physical resale is stripped out, what the canonical product model is with TPT as its base, how that model projects onto every other marketplace, what it costs to deliver on web, desktop, phone and browser extension, what in the current build survives, what the legal and app-store posture becomes, how the work sequences, and what must be decided before any of it starts.
Every claim that carries weight names the research file and section it came from, so the evidence is one click away.
Where two sources disagree, both appear with their provenance rather than one being chosen silently.

## Executive summary

Vendoo is not a hard product to copy: it is a thin cloud catalogue plus a fat Chrome extension, and every marketplace write happens inside the seller's own browser (`docs/research/rethink/vendoo-architecture-and-market.md` §1).
Two thirds of what Vendoo does assumes a physical object that sells once, and that two thirds evaporates for a digital file: quantity, variations, condition, size, colour, brand, shipping, background removal, sharing, offers, and auto-delist (`vendoo-workflows-and-ux.md` §10).
What replaces it has no Vendoo analogue at all: file versions, licence tiers, education standards, grade and subject taxonomies, bundles composed of other products, previews generated from the payload, and free-versus-paid gating (same section).
Vendoo's best idea transfers unchanged: a shared form, per-marketplace tabs, and inline "Update all" and "Reset" controls that appear on a field only where that field has diverged (`vendoo-workflows-and-ux.md` §3 and §11).
Vendoo's worst idea must be refused: it updates a live listing by delisting, editing and relisting, which for a teacher destroys the reviews, ratings, question thread, follower notifications and URL that are the accumulated asset (`vendoo-workflows-and-ux.md` §3).
TPT works as the base model and the wire proves it: one form, 48 client-writable field paths named by TPT's own server, and four visually distinct pickers that all serialise into a single flat tag array (`tpt-product-model.md`, "The field catalogue").
Every other marketplace is a projection of that model with named losses, and TeachBuySell's own import-from-TPT is external proof of the thesis: the metadata projects, and the sellable file never does (`cross-marketplace-mapping-tpt-base.md`, "Two targets found during this research").
The delivery surfaces are not equal and the inequality is structural: a web page cannot originate a marketplace request at all, so an extension or a native client must supply the data plane, and neither iOS nor Android can run a deterministic schedule (`client-surfaces-and-cross-compile.md` §5).
Moving execution to the seller's machine is the same change the legal memo already ranked as the structural fix rather than a mitigation, so the amendment to the architecture non-negotiable is the first decision here rather than a consequence of the others (`docs/notes/legal/marketplace-terms-assessment.md`, "The architecture finding").
The founder narrowed that move rather than taking it whole: it applies where no official API exists, while a marketplace that publishes an official API and issues a token for the purpose stays server-side under that token (D1).
Recommendation, now the decision taken: amend the non-negotiable to the two-branch rule, ship a Tauri v2 desktop client for Windows as the first surface with Android and then iOS behind it, defer the browser extension, keep SvelteKit and Paddle, and hold the programme at roughly seven to thirteen calendar weeks of build on this repository's own cadence rather than the research's 33 to 49 engineer-weeks, with the first shippable product being Phases 0 to 2 plus 5 and every uncompressible wall-clock floor stated separately.

## Product definition: what "Vendoo for teachers" means

### What transfers unchanged

One item that fans out to many marketplaces, and every screen a view over that fan-out, is the whole product shape and it transfers intact (`vendoo-workflows-and-ux.md` §1).
The per-field divergence affordance transfers verbatim and is the single best mechanism in Vendoo: data flows one way from the shared form to every marketplace form, and once a marketplace value has been edited so it differs, an "Update all" control appears on the shared form and a "Reset" control appears on the marketplace form, with neither firing automatically (`vendoo-workflows-and-ux.md` §3).
Coloured custom labels transfer as-is: created in a label manager, applied singly or in bulk, matched by the same search box that matches titles, and usable as a dimension in analytics (`vendoo-workflows-and-ux.md` §7).
Advanced filters, sorting and the page-size selector transfer as-is, as does the bulk-progress pattern: a floating status box that can be minimised, does not block navigation, and offers an opt-in completion sound (`vendoo-workflows-and-ux.md` §11).
Templates and per-field "save as default" transfer and are worth more to a teacher than to a reseller, because a teacher's boilerplate is terms of use, credits, a "follow my store" block and a standard preview page (`vendoo-workflows-and-ux.md` §10).
The split listing editor transfers: a rail of destinations on one side, the form for the selected destination on the other, the canonical form first in the rail, required fields marked, and an inline notification naming what is missing rather than a disabled button (`vendoo-workflows-and-ux.md` §11).
The connections page shape transfers: one row per marketplace carrying the brand mark, the connection state, a connect or disconnect control, and the per-marketplace feature toggles beside the connection that grants them (same section).
The automation rule editor shape transfers even though its content does not: a named rule with tiered conditions, exclusions, an optional schedule and an Active toggle, plus an activity log recording item, marketplace, date, time and status (`vendoo-workflows-and-ux.md` §5 and §11).

### What adapts, and how

The inventory board's three status columns adapt rather than transfer: drafted and listed carry over, but sold must not be a column, because selling does not remove a digital product from sale, and the third column becomes "needs attention" for out-of-date versions, drifted metadata and failed publishes (`vendoo-workflows-and-ux.md` §10).
The per-marketplace glyph strip adapts by widening its state set from Vendoo's three (not listed, listed, sold) to five: not published, published and in sync, published but out of date, publishing, and failed (same section).
Stale inventory adapts: the signal survives, because a product that has not sold in N days deserves attention, but the remedy does not, since Vendoo's remedy is the destructive delist-and-relist refresh; the offered actions become update the preview, revise the description or tags, or schedule a discount (same section).
Import from a marketplace adapts and should be improved: Vendoo's manual URL-paste "Mark as Listed" dedupe is documented as the root of its most common failure, and teacher catalogues have strong natural keys in the payload file hash, the title and the marketplace product id, so automatic matching replaces the paste and the paste stays as the manual fallback (`vendoo-workflows-and-ux.md` §2 and §10).
Photos adapt: per-marketplace photo sets with preserved ordering are needed because TPT thumbnail conventions differ from Tes cover conventions, but background removal drops entirely and preview generation replaces it, rendering page thumbnails from the payload PDF and assembling the marketplace's required preview file (`vendoo-workflows-and-ux.md` §10).
Cost of goods adapts and is worth keeping: teachers do incur per-product cost in commercial-use clipart and font licences and track it badly, so it becomes production cost with a licences-used field beside it, which also serves the attribution obligation those clipart licences carry (same section).
Marketplace fees adapt from Vendoo's per-sale typed entry to a per-marketplace commission rate modelled once and computed, because TPT and Tes take a commission whose rate depends on the seller's own plan tier (same section).
Sale detection adapts by keeping the detection and deleting the delist: the polling cadence, the per-marketplace enable toggle, the detected-sales inbox with a pending-details state and the blue dot all transfer, while auto-delist must not exist and the red sold-but-still-listed warning has no meaning (same section).
Analytics adapts by keeping the layout and changing the metrics: revenue, profit, average sale price, per-marketplace breakdown with prior-period comparison, top categories and label filtering all transfer, while inventory value drops because there is no inventory to value, and per-product lifetime sales and sell-through by grade and subject are added (same section).
Multi-account per marketplace adapts: Vendoo's one-account-per-marketplace limit with a manual disconnect-and-reconnect dance is a documented irritant, and teachers with a personal and a school-district account will hit it, so multiple credentials per marketplace should be designed in from the start (same section).

### What evaporates

Quantity, the multi-quantity manager and the multi-variation workarounds go entirely, and this is a large amount of Vendoo surface that simply disappears (`vendoo-workflows-and-ux.md` §10).
Condition, size, colour and brand go, and the form real estate they occupy is exactly where grade level, subject, resource type, standards alignment, page count and file format belong (same section).
Shipping goes, along with the marketplace-side shipping profile and business-policy machinery that Vendoo pulls in at connect time, though the *mechanism* of that machinery is reused below for licences (same section).
Auto-delist goes, and with it the hardest and most complaint-generating part of Vendoo's product: its Trustpilot score is 4.2 with 23 percent one-star, and the two recurring complaint classes are missed sale detection and silent connection drops (`vendoo-architecture-and-market.md` §5).
Send Offers and Marketplace Sharing go as built, because neither TPT nor Tes has liker offers or feed bumping, while the Automations shape they live in is kept for scheduled promotions and price changes around back-to-school, TPT sitewide sales and end-of-term (`vendoo-workflows-and-ux.md` §10).
Bulk delist, bulk relist and bulk mark-as-not-listed go, replaced by bulk publish, bulk update-to-latest-version, bulk price change and bulk label edit; Vendoo's "Save and relist" versus "Save Only" split becomes "Save and publish" versus "Save only" and is genuinely useful (same section).
Vendoo's item counter as the metering unit goes, because metering new items per month punishes a teacher who publishes twelve products a year and updates them monthly, and rewards a reseller adding hundreds (same section).

### What is new, with no Vendoo analogue

File versions are the central new entity: nothing in Vendoo tracks that the artefact behind a listing changed, and this one entity drives the out-of-date state, the update-everywhere action and the buyer-notification question (`vendoo-workflows-and-ux.md` §10).
Licence tiers are new as a concept and old as a mechanism: single, multi-user and school licences are a pricing dimension Vendoo has no field for, but Vendoo's own handling of eBay business policies and Etsy shipping profiles is the right shape, a named object fetched from the marketplace at connect time, selectable per listing and settable as a default (same section).
Standards alignment, grade and subject taxonomies are new and are a harder mapping problem than anything Vendoo solves, being multi-select, hierarchical, jurisdiction-specific and partially absent on some marketplaces (same section).
Bundles are new and are a different thing from Vendoo's bundle: Vendoo's is a transaction containing several items and its own documentation admits the shortcut corrupts analytics, whereas a TPT bundle is a product composed of other products, priced at a discount, whose members update when their sources update (same section, and `tpt-product-model.md`, "Bundles").
Free versus paid is new: Vendoo has a price field and no concept of a deliberately free product, whereas free products are a core acquisition tactic for teachers and are gated differently on every marketplace (`vendoo-workflows-and-ux.md` §10).
Previews generated from the payload are new, replacing background removal as the image work the product does for the seller (same section).

### The source-of-truth rule

Vendoo's rule is that the item is the source of truth for authoring and the marketplace is the source of truth for the live listing, reconciled by the seller destroying and recreating the listing; Vendoo states outright that "changes you make in Vendoo will NOT automatically be reflected on your published listings" and recommends delist, edit, relist on the grounds that brand-new listings rank best (`vendoo-workflows-and-ux.md` §3).
That rule cannot be adopted, because delisting a TPT or Tes product discards its reviews, ratings, sales history, question thread, follower notifications and its URL, which sellers have embedded in bundles, blog posts, Pinterest pins and their own newsletters (`vendoo-workflows-and-ux.md` §10).
The rule this product needs instead: the product record owns the canonical fields and the payload files, each marketplace mapping owns its override diff plus the version last published there, and the reconciliation verb is revise-in-place, never delete-and-recreate (same section).
The concrete behaviour when a teacher updates a PDF that is live on three marketplaces is that the upload creates a new product version, every mapping is marked out of date against that version, the console offers one "update everywhere" action lowering to a revise on each mapping, per-marketplace results are reported individually, and a mapping that cannot be revised stays flagged rather than silently succeeding (same section).
Drift the other way, a teacher editing the title on TPT directly, is detected by read-back and surfaced as a banner offering adopt-into-canonical or overwrite-from-canonical, which is strictly better than Vendoo's instruction to remember to type the change into Vendoo as well (same section).
Sale detection inverts in purpose: Vendoo scans every ten minutes and auto-delists to stop one physical unit being sold twice, whereas here the purpose is consolidating royalty and sales data across marketplaces, which is the reporting teachers cannot get today (`vendoo-workflows-and-ux.md` §3 and §10).

## The canonical product model, with TPT as the base

### How the TPT form actually works

TPT's create form is one CakePHP form posting `multipart/form-data` to `/My-Products/New/Digital-Next`, and the edit form posts a near-identical body to `/itemsDigital/editNext/{id}`, so create and edit are one model rather than two (`tpt-product-model.md`, executive summary).
The form's own `data[_Token][unlocked]` hidden input enumerates exactly 48 client-writable field paths, which is a field inventory authored by TPT's own server and the most authoritative list we hold (same file, "The server's own field inventory").
Four visually distinct pickers, Grade Level, Subject Area, Tag and Format, all serialise into one flat `data[TaxonomyTags][]` array of slugs, discriminated only by each slug's own `category` metadata, so there is no separate subject field and no separate grade field on the wire (same file, "Where each picker's values come from").
Product type is chosen on a type-selection page before the form renders and is read-only thereafter, with `itemType` one of `DIGITAL_PRODUCT`, `BUNDLE`, `ONLINE_RESOURCE`, `EASEL`, `VIDEO` (same file, executive summary).
Draft versus live is a field, `data[Item][status_user]` set to 0 or 1, not a route, so publishing is an ordinary edit (same file).

The DOM snapshot corrects three of those readings and adds one provenance rule (`tpt-create-form-dom.md`, "Closing the seventeen gaps" and "Control-by-control inventory").
`data[Item][generate_thumbnail]` is a three-way radio rather than an unenumerated integer: `1` auto-generates from the product file and is the default, `2` uploads thumbnails now and reveals the four slots as its conditional body, and `3` defers them, so the captured edit posting `3` means the seller deferred rather than that a fourth mode exists.
There is no Type-of-Resource picker anywhere on the create route: the only "Resource type" string in the page sits in the site header's browse dropdown, outside `#ItemAddForm`, and the form offers Grade Level, Subject Area, Tag, Format and Custom Category and nothing else.
There is no Primary Audience control either, so `data[ItemsProperty][audience]` is unlocked by the server's inventory and unreachable from this route.
The provenance rule follows from what the markup does not carry: five wire fields, the `data[TaxonomyTags][]` array, the standards ids, the tax code, the teaching duration and the answer key, have no submittable input in the DOM at all, because each is a client-rendered widget that serialises through JavaScript at submit.
The HAR therefore remains the wire authority for those five and the DOM is the label and layout authority, and a form-scrape-and-replay strategy that reads inputs out of this HTML would silently omit all five, one of them required (same file, "What is static markup and what is a client-rendered mount").
The three custom listboxes, Tax Code, Teaching Duration and Answer Key, carry their full option lists in the DOM as label-only `<li role="option">` elements with no value attribute, so their wire ids come from the vocabulary file and the captured wire and never from the markup.
Answer Key sets the trap: its six menu labels match ours exactly, but the display order is not the id order, so ordering our own control by id and rendering the label removes a class of mapping bug that would otherwise be invisible to a seller and wrong on the wire (same file, "Vocabulary reconciliation" and "Implications for our own form").
`data[_Token][unlocked]` is byte-identical to the 48 paths decoded from the 2026-08-30 capture, in the same order, so the server's own field inventory has not moved (same file, executive summary).

### The field catalogue

| Group | Field | Type | Cardinality | Required | Conditional rule | Value source |
|---|---|---|---|---|---|---|
| Name | Title | plain text | one | yes | screened by `CheckResourceTitle` before submit | free text, 80-char client constant |
| Files | File to Upload | opaque handle | one | yes | handle from the S3 then `process_file` then `queue/results` chain | 4 GiB cap, 49 extensions |
| Files | Product Preview | opaque handle | one | no | same upload chain | 30 MiB cap |
| Files | Video preview | opaque handle | one | no | behind the `VideoPreviewForDigitalProduct` A/B flag | 1 GiB cap, 11 extensions |
| Files | Auto-generated thumbnail | enum | one | yes | three-way radio closed by the DOM snapshot: 1 auto-generate (default), 2 upload now, 3 upload later | `data[Item][generate_thumbnail]` |
| Files | Thumbnail images | opaque handle | 4 fixed slots | no | 4 MiB each | image extensions, including TPT's own `ipeg` typo |
| Price | Free Resource | boolean | one | no | when set, no price and no tax code are collected | checkbox |
| Price | Price | decimal | one | yes when not free | floor 0.95 | bare decimal, no currency on the wire |
| Price | Multiple Licenses | decimal | one | yes when priced | form pre-fills 90 percent of price, seller may overwrite | any amount, no documented floor or cap |
| Price | Bundle Discount Price | decimal | one | no | posted 0 in both captures; meaning on a non-bundle listing unverified | unmeasured |
| Price | Tax Code | integer row id | one | yes when priced | not needed for free; a free product made paid must gain one | five values |
| Categories | Grade Level | slug | up to 4 | yes | "Not Grade Specific" is the all-grades escape | 17 checkboxes, 20 facets |
| Categories | Subject Area | slug | up to 3 stated | yes | the capture posted 4, so the cap is unresolved | 140 facets, 7 hidden |
| Categories | Tag (theme, audience, language) | slug | up to 6 | yes | controlled vocabulary, not free text | 52 facets |
| Categories | Format | slug | up to 3 | no | | 26 facets |
| Categories | Custom Category | integer id | multi, cap unknown | no | seller-scoped shelves, not a platform vocabulary | the seller's own |
| Standards | CCSS / NGSS / TEKS / VA SOL | integer node id | multi | no | TEKS shown only to Texas buyers, VA SOL only to Virginia buyers | leaves under jurisdictions 3054, 3055, 3326, 5785 |
| Standards | standards count | integer | one | yes when standards posted | must equal the number of id parts | derived |
| Details | Teaching duration | integer id | one | no | | 23 values |
| Details | Number of pages or slides | integer | one | no | read back as `filePreview.pageCount` | positive integer |
| Details | Answer Key | integer id | one | no | | 6 values |
| Details | Primary Audience | integer id | one | no | `OTHER` pairs with free text | 8 values |
| Details | Appropriate for New Zealand | boolean | one | no | label appears to be seller-country-derived | 0 or 1 |
| Copyright | Copyright declaration | integer id | one | yes | a legal attestation the adapter refuses to synthesise | 1 ORIGINAL_WORK, 2 USED_COPYRIGHTED_MATERIALS |
| Status | Making Listing Active | integer | one | yes | publishing is an edit, not a route | 0 draft, 1 live |
| Type | Product type | enum | one | chosen before the form | read-only after creation | five values |

All of the above is from `tpt-product-model.md`, "The field catalogue", which sources each row to the HAR capture, the form screenshot, `docs/design/data/tpt-vocabulary.json` or a named help article.

### Cardinality caps, product types, and two corrections to our adapter

The cardinality caps are stated on the form and were absent from our vocabulary file: up to four grades, three subject areas, six tags, three formats and four thumbnails (`tpt-product-model.md`, executive summary).
One cap is unresolved: the form says three subject areas and the captured create posted four, with a client-side advisory cap and a parent-plus-leaf picker expansion as the two candidate explanations (same file, "Where each picker's values come from").
Product types are five, and two of them need a modelling decision rather than a field: a bundle is composed of other resources rather than uploaded files, holds at least 2 and up to 500 members, may nest, and updates automatically when its members change; Easel is both a standalone type and an attachment that can hang off an ordinary digital product, so modelling it as a type alone would miss the attachment case entirely (`tpt-product-model.md`, "Multiple licences, bundles, Easel and video").
Correction one, the additional-licence price: our adapter derives `license_price` as a fixed 90 percent of price with a documented rounding choice, but TPT's own help centre states "Although the default discount is 10% off, you can choose whatever discount seems right to you", so the 90 percent is a pre-fill and not a rule, and treating the derived value as the only possible one would silently overwrite the pricing of any seller who set a custom additional-licence price on every sync (same section).
Correction two, the tax code: TPT's Terms of Service state "You acknowledge and agree that you are responsible for designating the appropriate tax codes for your Resources", so a tool that defaults a tax code is making a tax determination the seller is contractually answerable for, and the same reasoning our adapter already applies to the copyright attestation applies here (`tpt-product-model.md`, "Tax codes").
The DOM snapshot adds two constraints no earlier source held (`tpt-create-form-dom.md`, executive summary).
A tooltip on the Free Resource checkbox states "Free resources should be 10 pages or fewer.", which is guidance on the form rather than a validated bound, and it belongs beside our own free-resource control because a seller who breaches it learns so from TPT rather than from us.
The Custom Category tooltip defines the concept: "A custom category is any word or phrase you'd like to use to categorize your resources. You can manage your custom categories on your “My Product Listings” page." That confirms custom categories are the seller's own shelves rather than a platform vocabulary, which is why they render separately from the three platform pickers, and the tooltip states no cap, so the custom-category cardinality stays open.

### The copyright control, closed, and the title pre-check

The exact wording of the copyright assertions was the largest single gap in the TPT model, and the DOM snapshot closes it (`tpt-create-form-dom.md`, "Closing the seventeen gaps", items 1 and 2).
It is a single radio group rather than two checkboxes: one `name="data[ItemsProperty][copyright_declaration]"` group at values `1` and `2`, mutually exclusive, so the earlier two-boxes-to-tick reading is wrong.
Value `1` reads "I attest that this product I am about to post is an original work and it does not infringe upon the Intellectual Property rights of others."
Value `2` reads "I attest that I have used copyrighted and/or trademarked materials in my product and it does not infringe upon the Intellectual Property rights of others. I have either received express permission to use such materials, or I hereby certify that the use of such materials is otherwise non-infringing, for example as a fair use."
Value `1` arrives pre-selected on a blank form, which is the consequence that matters: a naive scrape-and-replay would post an attestation the seller never made.
Our adapter's handling is therefore vindicated rather than merely prudent, and should not be relaxed: `AuthorshipDeclaration` has no default and no `from_bool`, and its only constructor names the attesting party and the instant, so nothing in the crate can put a declaration on the wire without a seller having actually attested (`tpt-product-model.md`, "Copyright").
Our own form must invert TPT's default and pre-select nothing, refusing submission until the seller chooses, because a pre-selected attestation is ours rather than theirs (`tpt-create-form-dom.md`, "Implications for our own form").
Separately from the declaration, TPT screens the title: `CheckResourceTitle` fires as the seller types and returns a rightsholder identity when the title matches a trademark, so any tool that writes titles to TPT should call it before submitting rather than discovering the problem as a rejection (same file, "Title screening").

### Education standards

The four frameworks TPT's form offers are Common Core (jurisdiction 3054), NGSS (3055), Texas Essential Knowledge and Skills (3326) and Virginia Standards of Learning (5785), out of 166 jurisdictions TPT holds, so the four offered are a deliberate product choice rather than the whole of TPT's data (`tpt-product-model.md`, "Education standards").
Only one framework has an official machine-readable feed from its own owner: TEKS, published by the Texas Education Agency as a CASE-certified REST API that answered unauthenticated on 2026-09-02 (`education-standards-sources.md`, executive summary).
Common Core's official XML endpoint is dead, `corestandards.org` is now a four-PDF stub, and the browsable standards sit behind Cloudflare bot protection on a second domain whose own footer links its branding guidelines to the Wayback Machine (same file, "Common Core").
NGSS publishes no machine-readable form at all, only two PDFs and a paginated search UI, and Virginia publishes PDF and Word only while blocking every non-browser client with HTTP 403 (same file).
The practical primary source for three of the four is therefore a mirror rather than the owner: the Common Standards Project's open, keyless, CC BY API carries current-cycle CCSS, NGSS and Virginia data, and TEA's own API carries TEKS (same file, "Best primary source per framework").
Counts, all measured on 2026-09-02: roughly 1,540 addressable Common Core leaf codes out of about 3,600 nodes; 208 NGSS performance expectations out of about 3,350 nodes; roughly 5,000 TEKS student expectations across the four core chapters; and roughly 4,200 Virginia codes across three core subjects, with Science not aggregated (same file, "Expected table sizes").
That is order 11,000 addressable codes and 19,000 nodes under 10 MB of text, so the cost is correctness and currency rather than storage (same section).
Three licence obligations must appear in the product rather than in a footnote: the Common Core copyright notice verbatim wherever a standard is displayed, because the grant names copy, publish, distribute and display and never names modification; the NGSS asterisk-and-disclaimer footnote at the bottom of the home page and of every internal page that prominently uses the mark, with the mark visually subordinate to ours and no logo, no registered symbol; and attribution for whichever mirror we ingest from, under its CC BY terms (same file, "Licence obligations that must appear in the UI").
A fourth obligation follows from the first: no paraphrase of any standard's statement text anywhere, including in generated listing copy, because the Common Core grant does not extend to modification (same section).
TPT binds standards by its own opaque numeric node id, read back under the alias `sphinxId`, not by the published code, so carrying only the published code would leave us unable to post a selection and the code-to-node-id mapping table is the real deliverable (`education-standards-sources.md`, "What identifier a reseller tool must carry").
The named risk: a search-index identifier is exactly the kind that gets rebuilt, and if TPT reindexes, every projection edge we hold becomes silently wrong, so the mitigation is to store the code, the id and the statement text together, re-verify the triple on a schedule, treat a mismatch as a reconciliation item rather than an automatic repair, and never post an id not verified within the current crawl window (same section).
No source in this ecosystem publishes a cross-framework crosswalk, TEA's CASE packages carry only parent-child associations, and TPT's own help centre says it cannot translate Common Core into TEKS or VA SOL because they are not structured similarly, so building one is a separate proposition and should be recorded as a non-goal (same file, "Cross-walks").
Ingestion is estimated at 9.5 to 14.5 engineering days for the four frameworks, plus 3 to 5 days for the TPT standards-vocabulary crawl and the code-to-node-id table, which is what actually makes the feature post (same file, "Effort").

## Projecting the model onto other marketplaces

### The rule that sorts the targets

The rule is simple and it comes from the legal memo rather than from engineering: where a marketplace issues an official token for the purpose, a server holding that token is doing what the token was minted for, and server-side execution is legitimate; where no API exists, any automation drives the seller's own authenticated session against a form, which is precisely the pattern the legal memo identifies as the exposed one (`cross-marketplace-mapping-tpt-base.md`, "Server-side, client-side, and why the mapping does not care").
Only two targets fall on the server-side of that line, Etsy and Shopify, and only one of them is a teacher marketplace (same file, executive summary).
Everything else sits client-side: Tes, Classful, Made By Teachers, Teach Simple, Amped Up Learning, Teacha!, TeachBuySell, Gumroad, Payhip, Sellfy and Lemon Squeezy, with Boom outside the line entirely because there is nothing to automate that is not authoring (same section).

### Per-target verdicts

Tes: no API, five-step uploader, 58 file extensions at 200 MB, a required licence gated by price, exactly one main type and one primary category, and a three-level curriculum cascade; the connector is built, and the target stays client-side.
Etsy: the only sanctioned write API in the set, four calls to create a digital listing, but it forbids a free price outright, caps digital files at five of twenty megabytes, and holds no concept of grade, subject or standard.
Classful: no API and the one clause across the assessed platforms that plausibly reaches automated writes, but the cheapest crosswalk in the set, because its sixteen grade values are near-identical to TPT's list.
Made By Teachers: unmeasured on every field, because the site returns 403 to this network on every path, so every fact about it is second-hand.
Teach Simple: not a projection target as things stand, because it has no seller-set price, forbids descriptions copied from another marketplace, and requires an original description of at least 150 words.
TeachBuySell: not a target but the best external evidence for the thesis, since its own import-from-TPT works and its one stated limit is the one that matters.
Amped Up Learning: no self-service seller signup at all, an email to set up a store, and no listing-field detail public.
Boom Learning: a category error as a target, because the product is an interactive deck authored in Boom Studio rather than a file, only one copy of a deck may exist, and republishing a near-identical deck earns an account strike.
Storefronts (Shopify, Gumroad, Payhip, Sellfy, Lemon Squeezy): they hold the commerce fields and none of the discovery vocabulary, and of the five only Shopify can create a product through its API, with digital delivery delegated to a separate app.
All from `cross-marketplace-mapping-tpt-base.md`, "Target profiles".

### The proof that the metadata projects and the file does not

TeachBuySell ships a first-party "Import from TPT" tool: the seller pastes their public TPT store URL, the platform confirms the store name and product count, fetches in under a minute, auto-maps TPT categories onto its own subjects, year levels and resource types, converts USD to AUD at an adjustable rate defaulting to a 45 percent markup, rewrites in-description links pointing at other TPT products so they point at the seller's imported listings, removes links to products that were not imported, deduplicates against prior imports and identical titles, and creates draft listings only (`cross-marketplace-mapping-tpt-base.md`, "Two targets found during this research").
The one thing it cannot do is the thing that matters most: "we can't transfer files you sell there", so the seller must still download each resource from TPT and drag it in before publishing (same section).
That is the shape of the whole projection problem stated by a third party who built it: everything except the sellable bytes crosses automatically, and the bytes need the seller.

### The projection table in summary

| TPT field | Tes | Etsy | Classful | Boom | Storefronts |
|---|---|---|---|---|---|
| Title | adopt | adjust: 140 cap and a character class | unmeasured | adjust: 48-60 chars | adopt |
| Description | adjust: strip external URLs | adjust: HTML flattened | unmeasured | adjust: strip store links, fold standards into prose | adopt |
| File | adjust: 58 extensions, 200 MB | adjust: 5 files, 20 MB each | unmeasured | unsupported: deck authored in Studio | adopt |
| Free resource | adjust: forces a CC licence election | unsupported: price must be positive | adopt | adjust: capped free decks | adopt |
| Price | adjust: minor units, floor and ceiling | adjust: positive non-zero | adopt | adopt | adopt |
| Multiple licences | adjust: only the school tier | unsupported | unmeasured | unsupported | unsupported |
| Tax code | unsupported: platform handles tax | adjust: five codes collapse to one boolean | unsupported | unsupported | adjust |
| Grade level | adjust: 15-row exact table, GB by age band | drop: tags only | adjust: 16-value crosswalk | adjust: preschool to university | drop |
| Subject area | adjust: 43-root tree, one primary | drop: commerce taxonomy, tags only | adjust: 18-root crosswalk | adjust: credential-gated subjects | drop |
| Tag | unsupported: no free tag field | adjust: 13 tags of 20 chars | adjust: keywords | adjust: keywords | adopt |
| Education standards | adjust: coarse framework only, leaf ids dropped | drop | adjust: 6 framework names | drop: prose only | drop |
| Copyright declaration | adjust: compliance checkbox | adjust: `who_made` and `when_made`, non-delegable | unmeasured | adjust: acknowledgments | unsupported |
| Making listing active | adjust: publish then review | adopt: draft to active | adopt | adjust: publish or private publish | adopt |

The full table with every field and both extra targets is in `cross-marketplace-mapping-tpt-base.md`, "The projection table"; the five verdicts are adopt, adjust, drop, unsupported and unmeasured, and unmeasured means the projection blocks rather than that the field is absent.
Standards deserve the precise statement: the leaf ids stay in the canonical product and the projection emits a loss record naming the axis, so the seller sees "these 91 standards are not carried to this marketplace" in the field diff before publish, and the data is recoverable the day a target grows a field, because nothing was deleted (same file, "What happens to the standards data").

### What changes in the mapping engine

The canonical vocabulary is currently minted from Tes `yearGroups` because that was the richest set on file, and under a TPT-base model that direction inverts, which is a re-seed rather than a rewrite because the projection functions decide on the edge set alone and the authored artefacts are already expressed as derivations over the captures rather than transcriptions (`cross-marketplace-mapping-tpt-base.md`, "The canonical model as a TPT superset").
The superset claim needs one honest qualification: TPT is not a superset everywhere, because Tes requires a licence TPT has no field for, Etsy requires `who_made`, `when_made` and `quantity` that TPT does not hold, and Boom requires a deck, so the canonical model is TPT plus a per-target set of natives that the registry names rather than the adapter invents (same section).
Per-seller overrides cannot live in the global edge table, because a seller wanting "my TPT Math tag always becomes Tes Mathematics / Number" is not asserting a global equivalence; the shape that fits is a per-organisation override slice consulted before the global relation and carried into the listing context, with the field diff able to say "you set this" rather than "the relation says this" (same file, "Where per-seller overrides live").
One target makes that layer non-optional: Etsy's shop section, Boom's store folders and Shopify's collections are per-seller identifiers that must exist on the target before use, so TPT's custom categories map through a per-seller table populated by reading the target (same section).
Effort for the engine extension, from `cross-marketplace-mapping-tpt-base.md`, "Effort": re-base plus re-derive 3-5 days, per-seller override layer 4-6 days, route the unbound axes 5-8 days, best-fit suggestion layer 3-5 days, drift job 3-4 days, total 18-28 days, which that file states as four to six weeks.
Per new target on top of that: an API target such as Etsy is 1-2 days of vocabulary capture plus 8-12 days of adapter, and a form-driven target is 1-2 days plus 10-18 days, with the largest single cost driver being that seven of the nine platforms have no API (same section).

## Delivery architecture

### The surfaces are structurally unequal

A web page cannot originate a marketplace request at all, because a cross-origin fetch to TPT or Tes carries no session cookie and is refused by CORS, so the web surface can only ever be a console unless a browser extension supplies its data plane (`client-surfaces-and-cross-compile.md` §1).
Desktop, iOS and Android can each originate the request, and session capture works on both mobile platforms: WebKit's `getAllCookies` applies no HttpOnly filter and Chromium's Android WebView cookie manager reads with all cookies included, so a marketplace session cookie is reachable from the app after a webview login (same file §5.1).
Neither phone can be the scheduler: iOS grants a background refresh task up to 30 seconds at a time the system chooses, and Android's Doze mode stops JobScheduler and therefore WorkManager entirely, with the battery-optimisation exemption barred by Play policy (same file §5.2).
So the deterministic scheduler lives on the desktop client, or in an extension that only runs while the browser is open, and the honest product statement is that unattended sync requires the desktop agent (same file §8).
The S1 safe point from the prior note is preserved by every one of these placements: the server sends declarative intent, the client's local adapter composes and issues the request, and the seller owns the schedule (`docs/notes/design/client-side-architecture.md` §6).

| Capability | Web console alone | Web plus extension | Desktop app | iOS app | Android app |
|---|---|---|---|---|---|
| Browse, edit copy, see analytics | yes | yes | yes | yes | yes |
| Connect a marketplace account | no | yes | yes | yes | yes, via a JNI plugin we write |
| Originate a marketplace read | no, CORS | yes | yes | yes | yes |
| Originate a write or publish | no, CORS | yes | yes | yes, foreground only | yes, foreground only |
| Large multipart upload | no | yes, while the tab is open | yes | fragile | fragile |
| Unattended scheduled sync | no | no | yes | no | no |
| Photo capture into a listing | no | no | no | yes | yes |

From `client-surfaces-and-cross-compile.md` §5.4.

### Vendoo's own architecture, as evidence rather than as a model

Vendoo's extension is Manifest V3 with a service worker, `cookies` and `webRequest` permissions, host permissions on nine marketplace domains, and sixteen `declarativeNetRequest` rules that rewrite `Origin`, `Referer` and `Access-Control-Allow-Origin` so extension-issued fetches are accepted by each marketplace as if they came from that marketplace's own listing page (`vendoo-architecture-and-market.md` §1).
That is not DOM form-filling: it is the same plain-HTTP adapter approach we already have in Rust, relocated into a browser and given CORS cover by the browser's own rule engine, which means our adapters transpose rather than being rewritten (same section).
The marketplace-specific logic is served rather than shipped: a single 552 KB generic page-script executor runs on every marketplace host and `externally_connectable` is limited to Vendoo's own web origins, so selector and endpoint drift is fixed without a Chrome Web Store review cycle (same section).
Vendoo is Chrome only, with no Firefox, Edge or Safari listing found anywhere, which is a competitive weakness rather than a model (same section).
The mobile apps are real and crosslist to eight marketplaces with no extension, but cannot import, cannot bulk delist or relist, and cannot reach Facebook Marketplace or Shopify, and Vendoo states on its own marketing page that "your computer must be on and connected to the marketplaces for sale detection and auto delist to work in the mobile app" (`vendoo-architecture-and-market.md` §2).
The support record shows what that costs: users report a marketplace suspension caused by a missed sync, and support advising a seller to "leave the computer open over the night so that the software can recognise the sales" (same file §5).

### The candidate architectures

| | Surfaces | Rust reuse | UI reuse | Effort (weeks) | Store surfaces | Deterministic cron |
|---|---|---|---|---|---|---|
| A1 | web console, 3 desktop OSes | 100% | 100% | 10-16 | 0 | yes, desktop |
| A2 | web console, 3 desktop OSes, iOS, Android | 100% | 100% | 18-28 | 2 | yes, desktop only |
| A3 | A1 plus extension, mobile as a Capacitor companion | 100% desktop, partial wasm, none mobile | 100% | 18-27 | 5 | yes, desktop |
| A4 | web console plus extension only | core 100%, transport replaced | 100% | 10-14 | 2 | no |

From `client-surfaces-and-cross-compile.md` §9.
What a teacher can do differs sharply: on A1, everything from a desktop and nothing from a phone; on A2, full parity for anything they start themselves on every surface, with unattended sync only where the desktop agent runs; on A3, publish from any desktop browser without installing anything, and from the phone manage inventory and dispatch work but not originate a write; on A4, everything except unattended sync, and only while a browser is open on a computer they are at (same section).
A4 is the cheaper different approach the brief asked to see, and it deserves a fair hearing: it abandons mobile and desktop entirely, is roughly half the cost of A2, has no gatekeeper in the payment path, needs no code signing, no notarisation, no macOS runner and no Apple or Google developer fee, and it is the architecture both extension-based incumbents chose (same section).
It fails the brief on three counts: no iOS, no Android, and no deterministic schedule, plus a store takedown lever the prior note called disqualifying (same section, and `docs/notes/design/client-side-architecture.md` §7).
Tauri v2 is the only framework that reuses both the Rust core and the existing SvelteKit console across all five surfaces, and the console is already a static build with an `index.html` fallback, which is exactly the shape a Tauri or Capacitor shell consumes without modification (`client-surfaces-and-cross-compile.md` §4.1 and §2).

### The cross-compilation verdict

The pure core is 16,169 lines across six crates whose entire dependency closure is 48 crates, every one of them pure Rust, with no `libc`, no `ring`, no crate ending in `-sys` and no `getrandom`, verified locally by `cargo tree` and a clean `cargo check` of that set alone (`client-surfaces-and-cross-compile.md` §2).
That layer compiles to every Rust target including WebAssembly with no feature flags, no shim and no C toolchain (same section).
Transport-coupled production code is 873 lines out of 28,151 that would move to a client, a ratio of about 32 to 1 (same section).
Step zero is a Cargo feature gate that does not exist yet: `reqwest` is an unconditional dependency of both adapters with no features table at all, so a non-native build fails at the dependency rather than at the code (same section).
Every target the plan needs is Rust Tier 2 or better, and the only compiled-from-C component in the adapter path is `ring`, whose own CI tests iOS and both Android ARM targets (same file §3.1 and §3.2).
One fixed cost cannot be engineered away: iOS builds require a macOS host, no cross-compilation tool bridges it, and macOS CI runners cost roughly six times Windows and ten times Linux per minute (same file §3.3 and §3.4).
The verdict the founder should hold: our code is not the obstacle on any target, but the claim is not yet proven locally, because this machine has only the Linux target installed and no cross-target check was attempted, so adding the targets to the toolchain file and proving them in CI is the first action and it converts the largest unverified claim into a green check (same file §3.1 and §10).

## Keep, rewrite, delete

| Component | Lines | Verdict | Why |
|---|---|---|---|
| `tam-types`, `tam-limits`, `tam-marketplace`, `tam-domain`, `tam-taxonomy` | 15,026 | keep unchanged | pure core, compiles to every target with no feature flags |
| `tam-pipeline` | 1,143 | keep, one separation | portable except the local object store, which is separable |
| `tam-marketplace-tpt`, `tam-marketplace-tes` non-transport code | 11,982 | keep unchanged | generic over the seam; mentions `reqwest` exactly once, in a comment |
| `tam-marketplace-tpt/src/live.rs`, `tam-marketplace-tes/src/live.rs` | 873 | rewrite, gated | move behind a `live` Cargo feature; a wasm build replaces them with a fetch transport and reworks redirect classification |
| `tam-engine/src/driver.rs` | 1,581 | rewrite | split from concrete storage repositories behind a repository trait so the interpreter runs against a remote ledger |
| `tam-engine` remainder | 1,053 | keep | scheduling policy moves to the client, the effect vocabulary does not |
| `tam-storage` | 10,031 | keep, minus the vault tables | control plane; `connection_secret` and its migration go |
| `tam-api` | 8,205 | keep, extend | gains the authoring endpoints the creation-flow note designs, and the declarative-intent surface the client pulls from |
| `tam-session-broker` (`gateway.rs` 1,575, `vault.rs` 1,478, `main.rs` 406, `service.rs` 399, `jar.rs` 276, `protocol.rs` 192) | 4,326 | delete | the session lives on the seller's machine; custody dissolves |
| `tam-engine/src/broker_client.rs` | 249 | delete | the engine's side of a lease that no longer exists |
| Key-encryption key, escrow and rotation drills, `LeasePurpose` | n/a | delete | nothing left to encrypt centrally |
| Gateway route allow-lists (`TES_PREFIXES`, `TPT_PREFIXES`) | in `gateway.rs` | delete, with a recorded downgrade | the rule survives in the signed client, but moves from infrastructure-enforced to client-enforced |
| Four database roles | n/a | collapse to two or three | `tam_broker` goes with the vault; `tam_engine`'s cross-tenant justification narrows |
| Auto-delist analogue | never built | delete from the plan | no sale-detection code exists anywhere, and digital resources are infinite-inventory |

The 4,575-line deletion total and the per-file counts are from `docs/notes/design/client-side-architecture.md` §5; the crate line counts are from `client-surfaces-and-cross-compile.md` §2.

| Web route | Verdict | Why |
|---|---|---|
| `library`, `listings`, `templates`, `queue`, `jobs`, `sync`, `status`, `notifications` | keep | already the console shape the rethink needs |
| `connections` | rewrite | today it renders list and revoke only, with the sole sealing path an operator command; it becomes the per-marketplace row with connect, state and feature toggles |
| `analytics` | keep, feed it | the layout survives; nothing persists a metric row today, so there is no durable series to read |
| `admin` | keep | operator surface, unaffected by where the request originates |
| creation form (designed, not built) | rewrite before building | re-base it on the canonical TPT model rather than on the current six canonical field keys |
| `purchases`, `settings`, `help`, `login`, `signup`, `reset` | keep | identity and account surfaces, unaffected |

The connections and creation-form verdicts follow `docs/notes/design/platform-linking.md` §1 and `docs/notes/design/creation-flow.md` §3.

## Legal, store and licence posture

Request origin is the whole question, and moving it is the only change that alters which question gets litigated rather than improving the answer to the existing one (`docs/notes/legal/marketplace-terms-assessment.md`, "Ranked mitigations").
The Ninth Circuit's Perplexity decision turned on an architectural fact, that Perplexity's servers never directly accessed Amazon's servers, so it is the user who accesses with the help of the tool; our servers indisputably access TPT's and Tes's, which concedes the prong Perplexity won on and leaves only the authorization question that is lost the day a cease-and-desist arrives (same file, "The access-versus-authorization distinction").
The S1 point on the control spectrum is the safe one and the recommendation stands: the server sends declarative intent, the client's local adapter composes and issues, and the user set the schedule; relocating only the gateway would leave our server composing every request, which is the configuration the court expressly reserved (`docs/notes/design/client-side-architecture.md` §6).
Nothing in this architecture improves the tortious-interference position, which turns on knowledge of the terms and intent to induce a breach and was expressly left open, so the mitigations that bite there are conduct-based: do not induce the breach, do not market against the terms (`marketplace-terms-assessment.md`, "The legal framework").
The cease-and-desist is the pivot and the kill switch is the answer: one short-lived Ed25519 entitlement artifact carries subscription state and a per-marketplace grant set, so a lapsed subscription and a marketplace revocation are the same code path, and disabling a marketplace across the whole installed fleet is a control-plane flag flip propagating within one revalidation window (`docs/notes/design/client-side-architecture.md` §8).
The app stores are a second gatekeeper and this is the sharpest new risk in the rethink: Apple guideline 5.2.2 requires that you be "specifically permitted" under a third-party service's terms and that "Authorization must be provided upon request", which hands Apple the same lever a cease-and-desist gives the marketplace, and Google Play's Device and Network Abuse policy says the same thing in different words (`client-surfaces-and-cross-compile.md` §6.1 and §6.2).
Vendoo and Crosslist both ship automating apps on the App Store today, and PrimeLister ships one literally named "Poshmark Bot", which is evidence of non-enforcement rather than of permission: none of those marketplaces has complained yet, and 5.2.2 exists precisely so the store can act the day one does (same file §6.3).
Both stores also ban downloading executable code that changes app functionality, which converges neatly with the legal line: ship selectors and route maps as signed data and never as code, and one discipline satisfies Apple, Google and the S1 position at once (same file §6.1, and `client-side-architecture.md` §6).
Common Core: display is permitted, modification is not, and the copyright notice must appear wherever a standard is published or publicly displayed, which forecloses paraphrasing a standard's text anywhere including generated listing copy (`education-standards-sources.md`, "The public licence").
NGSS: the permissive public licence is addressed to states, districts, schools, teachers and non-profit education entities, which we are not; commercial use falls under a trademark regime requiring the prescribed disclaimer footnote on the home page and every internal page that prominently uses the mark, no logo, no registered symbol, and sample submission to WestEd with a stated 4 to 6 week turnaround (same file, "Licence terms").
D18 takes the disclaimer and the no-logo rule and declines the sample submission, so nothing in the plan waits on WestEd.
TEKS: the standards are Chapters 110 to 128 of Title 19 of the Texas Administrative Code and therefore state law, while TEA's own documentation site restricts registered users to personal, noncommercial use and forbids bulk download, and which of those controls a commercial tool caching TEKS codes is a legal question rather than an engineering one (same file, "What we may do with each framework").
Etsy adds a second non-delegable field class: `who_made` is required on every create, takes `i_did`, `someone_else` or `collective`, and is an attestation about authorship not derivable from any TPT field, with `when_made` riding along because Etsy requires the pair (`cross-marketplace-mapping-tpt-base.md`, "Resolution modes, with licence as the worked exemplar").
One correction to the legal memo, which is applied at the memo itself as a dated paragraph: PrimeLister's cloud Poshmark automation asks the seller to enter their Poshmark username and password, stores those credentials encrypted server-side, and documents that connecting without saving them is not currently possible, so server-side credential-holding does exist in the comparable set (`vendoo-architecture-and-market.md` §6).
That correction does not change the verdict: it removes "nobody does this" from the risk argument, and it leaves the memo's exposure analysis untouched, because the analysis never rested on the practice being unique, and TPT is a materially more litigious counterparty than Poshmark (same section).

## Methodology and sequence

### How the research becomes a build

The ordering principle is that the data plane rather than the surface is the unit of work, and that every step is independently useful if the next is deferred (`client-surfaces-and-cross-compile.md` §10).

The estimates are re-baselined, because the research priced one senior human engineer working serially and that is the wrong unit for this repository.
The founder has said so, and the log agrees.
Forty-eight changes landed on `main` between 2026-08-30 and 2026-08-31, delivering the `tam-auth` better-auth identity service with its Postgres role, schema and append-only event log, the console redesign, analytics capture and its page, the operator backoffice with impersonation and its audit trail, the Paddle billing foundation, the server-side creation flow with its authoring UI and vocabulary reads, and the Tes one-seller-per-account rule.
That is seven work items the research's own conversion would have priced in engineer-weeks each, and they landed inside two calendar days; the whole repository is 256 changes deep since 2026-08-25.

The table therefore carries three columns.
The first is the research's figure for one human engineer, kept as provenance rather than as a plan.
The second is the orchestrator's re-baselined estimate for this repository's agent-driven cadence.
The third names the wall-clock floors that do not compress whatever the cadence is, because each one is somebody else's queue.

| Phase | Research, one human engineer | Re-baselined build | Wall-clock floors |
|---|---|---|---|
| 0. Seam gate and cross-target CI | 1 week | 1 day | the cross-target CI proof itself |
| 1. Engine driver split | 4 to 6 weeks | 2 to 4 days | none |
| 2. Tauri desktop, Windows first | 6 to 10 weeks | 1 to 2 weeks | Windows code-signing certificate issuance; the founder's live login-page probe on their own Windows machine |
| 3. Mapping re-base on TPT, per-seller overrides | 4.5 to 7 weeks | 3 to 5 days | none |
| 4. Standards ingestion and the TPT node-id crawl | 3 to 5 weeks | 2 to 4 days | none |
| 5. The canonical creation form on the DOM snapshot | not separately priced | 3 to 5 days | none |
| 6. Etsy connector | 2 to 3.5 weeks | 3 to 5 days | Etsy Commercial Access review, no published service level |
| 7. Android | 4 to 6 weeks | 1 to 2 weeks | Google Play organisation account setup, then store review |
| 8. iOS | 4 to 6 weeks | 1 to 2 weeks | Apple developer account, a macOS runner, then App Store review |
| The WASM core in the browser | 5 to 10 engineer-days | 3 to 5 days | none |
| The device registry and "Your devices" | not priced | 3 to 5 days | none |

Summing the second column gives 35 to 64 working days of build, roughly seven to thirteen calendar weeks, against the research's 33 to 49 weeks.
The first shippable product is Phases 0 to 2 plus 5: eleven to twenty working days of build, roughly two to four calendar weeks, plus the code-signing certificate and the probe.
Phase 5 is in that set because a desktop client that cannot create a product on the canonical form is a synchroniser rather than the product the brief describes.

Three assumptions sit behind the second column, stated so the estimate can be falsified rather than argued about.
The unit is this repository's agent-driven cadence, with the founder answering a gating question the same day and the working copy staying green under `just check`.
The figures exclude every floor in the third column, and those floors run in parallel with build work wherever a phase does not depend on them.
Confidence is lowest on Phases 2, 7 and 8, because each crosses a store or a webview this tree has never driven, which is why each carries a floor rather than a tighter number.
The two exclusions the research already declared still hold and are restated under "Reconciling the two estimates" below.

Phase 0: add a `live` Cargo feature to both adapter manifests, gate the transport files behind it, add the send-bound shim to the transport trait, add the iOS, Android and wasm targets to the toolchain file, and prove `cargo check --target` for the pure core in CI.
Proves the whole portability argument, converting the largest unverified claim into a green check.
Kill gate: a target that does not build.
Verification: the CI job itself, which fails under any incorrect implementation because it compiles the real crates against the real targets.
Founder acts gating it: none.

Phase 1: split the engine driver from concrete storage repositories behind a repository trait, so the effect interpreter runs against a remote ledger.
Proves every later surface can host the engine, and none of the work is wasted if the surface choice changes.
Kill gate: the split cannot be made without changing engine semantics.
Verification: the existing engine tests pass unchanged against an in-memory ledger, which would fail under a split that leaked storage assumptions.

Phase 2, the first surface: a Tauri v2 desktop client for Windows, with the webview used for login only, the existing transport unchanged, a client-side scheduler, OS-keychain session storage, entitlement verification, a signed installer, and an updater served from CrabNebula Cloud.
macOS leaves the first release under D2 and D29, because the founder has no Mac and no macOS virtual machine is permissible; Linux stays best-effort.
Proves the client-side thesis against real marketplaces, and it is a complete product for a seller with a computer.
Kill gate: the login-page bot-protection probe, which decides whether an embedded webview can clear the challenge at all.
Verification: a live create and publish on the founder's own TPT and Tes stores from the desktop client, read back through the authoritative API rather than trusting the write's own status code.
Founder acts gating it: the probe on their own Windows machine, and the Windows code-signing certificate.

Phase 3: re-base the canonical vocabulary on TPT, add the per-seller override layer, route the axes not yet bound, add the drift job.
Proves the base-product decision is true in the data rather than only in the prose.
Kill gate: a residue report showing the TPT-to-Tes re-derivation loses a mapping the current direction holds.
Verification: the seed residue report, plus the existing projection tests re-run in the inverted direction.
Founder acts gating it: none remaining, because the TPT create-form DOM snapshot that gated it is in hand (D13).

Phase 4: standards ingestion for the four frameworks plus the TPT node-id crawl.
Proves a teacher can tag standards and have them post.
Kill gate: TPT's node ids prove unstable across a re-crawl.
Verification: a hand-checked sample per framework against the owner's PDF, and a diff of Virginia's two independent mirrors against each other.
Founder acts gating it: none.
The first draft put a WestEd sample submission in Phase 0 on a four-to-six-week turnaround; D18 removes it, so NGSS compliance is the prescribed disclaimer in the footer and no logo, with nothing submitted and nothing to wait for.
TEKS comes from the Common Standards Project mirror under CC BY, and no question goes to Texas (D19).

Phase 5: rebuild the canonical creation form on the TPT DOM snapshot, control by control, against `docs/research/rethink/tpt-create-form-dom.md`.
Proves the canonical model is the form a seller actually fills in rather than a projection target only, which is what makes the desktop client a product instead of a synchroniser.
Kill gate: a control the snapshot shows that the canonical model cannot express.
Verification: every one of TPT's 48 client-writable field paths is either bound to a canonical field or recorded as a deliberate omission with its reason.

Phase 6: the Etsy connector, which is the only sanctioned API in the set and therefore the first marketplace on D1's server-side branch.
Founder act gating it: the Etsy app registration, filed early because Commercial Access is a manual review with no published service level.

Phase 7: Android, reusing the same project and the same console build, with foreground-only publishing for the no-API marketplaces, full parity for Etsy because the server schedules it, camera capture, and a "run now" dispatch to the desktop agent.
Founder act gating it: the Google Play account registered as an organisation now, because a personal account faces a twelve-tester, fourteen-consecutive-day production gate that is wall-clock and cannot be compressed.
The first Play upload is manual, because Tauri does not automate an Android release.

Phase 8: iOS, on the same project and the same console build.
Founder acts gating it: an Apple developer account at 99 dollars a year, and a macOS host, which is a GitHub or Codemagic runner for the build and a physical Mac for the Keychain certificate export and on-device work.

The browser extension is deferred rather than sequenced.
The research priced it at six to eight weeks as the web surface's data plane; the founder would take it only as a one-day build, and the one-day version is the server-composes relay that D1 rules out, so it is off the plan (D2).
The web console keeps publishing directly to the marketplaces on the API branch, and hands the no-API ones to the desktop agent.

### Reconciling the two estimates

The two research files count in different units and the arithmetic has to be shown rather than asserted.
`client-surfaces-and-cross-compile.md` §10 gives a six-step sequence in weeks: 1 + (4 to 6) + (6 to 10) + (6 to 8) + (4 to 6) + (4 to 6), which sums to 25 to 37 weeks.
That same file's architecture table gives A2 as 18 to 28 weeks and A3 as 18 to 27, and the sequence exceeds both because the sequence includes the extension *and* both phones while A2 excludes the extension and A3 replaces native mobile with a Capacitor wrapper.
`cross-marketplace-mapping-tpt-base.md` and `education-standards-sources.md` count in engineer-days: 18 to 28 days for the mapping engine, and 9.5 to 14.5 plus 3 to 5 for standards, so 12.5 to 19.5 days.
Converting at the client-surfaces file's own stated convention of roughly four focused days per calendar week gives 4.5 to 7 weeks for the mapping engine and 3.1 to 4.9 weeks for standards.
The mapping file states its own 18-28 days as "four to six weeks", which implies about 4.7 days per week; both figures are shown rather than reconciled, and the wider conversion is used in the phase plan because it matches the surface estimates' own assumption.
One engineer working serially therefore needs 25-37 plus 4.5-7 plus 3.1-4.9, which is 32.6 to 48.9 weeks, stated as 33 to 49 weeks or roughly eight to eleven months.
The first shippable product is Phases 0 through 2, which is 1 + (4 to 6) + (6 to 10) = 11 to 17 weeks; the client-surfaces file states the equivalent A1 as 10 to 16 weeks, a one-week difference arising because A1 folds the feature gate into the seam work rather than counting it separately.
Assumptions behind all of it, taken from `client-surfaces-and-cross-compile.md` §9: one senior engineer full-time and already fluent in this tree, calendar weeks at roughly four focused days, confidence about plus or minus 30 percent on the seam work and plus or minus 50 percent on anything crossing a store or a webview we have not driven, excluding legal review, marketing, store-account waiting time and the live probe, and including CI wiring, signing setup and one submission cycle per store.
Two things sit outside every figure: the subject-level crosswalk between marketplaces, which is authoring rather than engineering and should be allowed to stay partial, and the standards crosswalk between frameworks, which is a declared non-goal.

That arithmetic is now the first column of the phase table above and nothing more.
It records how the research reached 33 to 49 weeks, so the figure can be checked rather than taken on trust, and it is not the plan.
The plan is the re-baselined column, and the first shippable product is Phases 0 to 2 plus 5 rather than Phases 0 through 2.

## Decisions taken

Every decision below has been taken.
D1 to D27 were put to the founder on 2026-09-02 and answered across four rounds on 2026-09-02 and 2026-09-03; D28 to D30 arose from research done after the first draft and were taken on 2026-09-03.
The numbers are kept so that the discussion which produced each one stays traceable to the first draft's question list.
Where the founder changed or narrowed the recommendation, the change is stated rather than folded in silently.
The build itself still awaits an explicit founder go on the plan.

| # | The decision as taken | Date |
|---|---|---|
| D1 | The architecture non-negotiable changes, but to a two-branch rule rather than the wholesale move the memo recommended. Where a marketplace publishes an official API and issues a token for the purpose, automation stays server-side under that token, which keeps Etsy and Shopify on the server; where no official API exists, TPT and Tes today, every marketplace request originates on the seller's own device under the seller's own session. The rule is explicit in code as a transport class per marketplace in the registry, with a test that fails the build if a no-API marketplace gains a server transport, and explicit in the interface as a badge on every marketplace row, a connect flow that says where the login happens, and publish progress that names the device doing the work. Round four settled the question the founder raised in round three: the seller's device is the only thing that opens a connection to a no-API marketplace, so there is no shared-session handover to the server, and an opt-in cloud mode is reserved as a later decision taken with counsel and never the default. | 2026-09-02, completed 2026-09-03 |
| D2 | Windows desktop first, then Android, then iOS, all Tauri v2, with desktop bundles distributed through CrabNebula Cloud. This narrows the memo's order twice: macOS leaves the first release because the founder has no Mac and no macOS virtual machine is permissible, and the browser extension is deferred rather than sequenced second. The extension is deferred by the founder's own rule, that they would take it only as a one-day build: the one-day version is the server-composes relay that D1 rules out, and the honest version is the six-to-eight-week one the research priced. | 2026-09-02, narrowed 2026-09-03 |
| D3 | As recommended, and narrowed to the no-API branch. A phone is limited to work the seller starts only for marketplaces with no official API; an API-branch marketplace such as Etsy has full parity on a phone, because the server schedules it and the phone never had to originate the request. | 2026-09-02 |
| D4 | Meter connected marketplaces, with a catalogue cap on the entry tier, and never Vendoo's new-items-per-month. Paddle stays. Stripe was evaluated at the founder's request, because its agent and plugin ecosystem is real, and rejected on one fact: Stripe Managed Payments, the merchant-of-record product that would match what Paddle does, is not available to New Zealand sellers, so plain Stripe would make us the seller of record with our own registrations and filings outside the US. Revisit when New Zealand becomes eligible, or when revenue justifies the filing burden. | 2026-09-02, metering axis and Paddle confirmed 2026-09-03 |
| D5 | As recommended. A new product version, every mapping marked out of date, one "update everywhere" action lowering to a revise per mapping, per-marketplace results reported individually, and no buyer notification unless the marketplace offers a native one. | 2026-09-02, by silence |
| D6 | As recommended. The canonical model carries the additional-licence price explicitly and defaults it to TPT's 90 percent pre-fill in the form only, never in the projection, so a seller's own figure survives every sync. | 2026-09-02, by silence |
| D7 | As recommended. The tool never defaults a TPT tax code: the designation is the seller's under TPT's terms, and the existing refusal to synthesise a copyright attestation is the precedent. | 2026-09-02, by silence |
| D8 | As recommended. Tes, Etsy and Classful are the first three projection targets. | 2026-09-02, by silence |
| D9 | As recommended. A free TPT product projected onto Etsy raises a named publish gate offering a minimum-price election, never a silent floor. | 2026-09-02, by silence |
| D10 | Confirmed, with one nuance the founder asked for. The entitlement token is an accepted exception because Postgres remains the decision-maker and the token only transports a decision already made; the nuance is that the server is the authority per check-in rather than per marketplace request, since the token is cached for its validity window, and the client-side gate is secondary, fail-closed and advisory. The server also withholds the catalogue, the mapping decisions and the queue from a lapsed account, which is the enforcement that does not depend on the client at all. | 2026-09-02, nuance recorded 2026-09-03 |
| D11 | Confirmed as recommended: one-hour token validity plus a 24-hour grace, failing closed. This is now the only subscription enforcement for work on the no-API branch, because that work runs on the seller's machine, and the kill switch is the same token's per-marketplace grant flag. | 2026-09-02, grace confirmed 2026-09-03 |
| D12 | Yes. The probe runs read-only from the founder's own accounts: on their own Windows machine for the WebView2 half, and on Linux WebKitGTK for the indicative half. It gates the desktop client, and nothing is committed to that surface before it answers. Decided-pass 2026-09-03: both marketplaces answered on Windows WebView2, Tes CLEAR and TPT's only marker resting on a feature-flag name rather than on any bot-protection asset in the page, both with a working password form. The reports, the reading and the argument-handling defect the runs exposed are in `docs/research/rethink/login-probe-windows.md`. | 2026-09-02, host settled 2026-09-03, decided-pass 2026-09-03 |
| D13 | Done. The snapshot is in hand and verified: `~/downloads/tpt-create-form.html`, 286 KB, the full `ItemAddForm` including both copyright assertions, the standards picker and `status_user`. The control-by-control reading has landed as `docs/research/rethink/tpt-create-form-dom.md`. It closes the copyright wording as a single radio group whose value `1` is pre-selected on a blank form, which vindicates the adapter's refusal to synthesise an attestation and means our own form must pre-select nothing; it closes the `generate_thumbnail` enum as a three-way radio; it denies the Type-of-Resource picker and the Primary Audience control on the create route; it finds five wire fields with no submittable input in the DOM, leaving the HAR as the wire authority for those; and it confirms that `data[_Token][unlocked]` has not moved since the 2026-08-30 capture. | 2026-09-03 |
| D14 | Per-surface marketplace login, plus a server-side device registry and a "Your devices" page with per-device sign-out; the founder added the registry to the recommendation. It is built on better-auth's `listSessions`, `revokeSession`, `revokeOtherSessions` and `revokeSessions`, with our own device columns beside them, because better-auth's session row carries only `ipAddress` and `userAgent` and has no device-name concept. Two facts are recorded rather than engineered away: the cookie cache keeps a revoked session usable until it expires, which the page must state, and the device id is generated once into application data and labelled with the hostname, because `machine-uid` covers neither Android nor iOS. | 2026-09-02 |
| D15 | As recommended. Bundles are modelled but are not a first-release publish path, because no bundle create has ever been captured; Easel, Online Resource and Video stay out until each is captured. | 2026-09-02, by silence |
| D16 | As recommended. Multiple credentials per marketplace from the start, designed around the existing global exclusivity index rather than retrofitted into it. | 2026-09-02, by silence |
| D17 | As recommended. There is a delist verb, named "retire from this marketplace", never "refresh", and never offered as a way to update a listing. | 2026-09-02, by silence |
| D18 | Narrowed. Comply with the NGSS trademark guidance by carrying the prescribed disclaimer in the footer and using no logo, and submit no samples to WestEd. This removes the four-to-six-week turnaround from the plan entirely, and with it the memo's reason for starting a submission in Phase 0. Nominative use is ordinarily permitted and WestEd's guidance is guidance rather than statute; the disclaimer is the compliance, and it costs nothing to carry. | 2026-09-03 |
| D19 | Downgraded. Ingest TEKS from the Common Standards Project mirror under its CC BY licence and ask Texas nothing. TEA's own terms are never agreed to, and a written question becomes necessary only if we ever pull TEA's feed directly. | 2026-09-02 |
| D20 | As recommended. No Academic Benchmarks purchase: its free tier leaves either TEKS or VA SOL uncovered, and its identifier buys nothing that TPT's node id does not. | 2026-09-02 |
| D21 | As recommended. Etsy Commercial Access is the target, filed early, with per-seller Seller Apps as the bootstrap that does not wait on review. | 2026-09-02 |
| D22 | As recommended. The Google Play account is registered as an organisation now, because a personal account faces a twelve-tester, fourteen-consecutive-day production gate that cannot be compressed. | 2026-09-02 |
| D23 | As recommended. Ship US-first with an external link, and treat non-US in-app purchase at a 15 to 30 percent cut as a funded later milestone. | 2026-09-02 |
| D24 | As recommended. TPT's most-favoured-pricing rule is a publish gate rather than a warning, which converts a contractual liability into a product feature. | 2026-09-02, by silence |
| D25 | As recommended. Standards statement text is displayed, the Common Core attribution obligation is accepted, and the text is never paraphrased anywhere, including in generated listing copy. | 2026-09-02, by silence |
| D26 | As recommended. The marketplace-side import-from-TPT business is parked: it is a real business, it is a different one, and it competes with our own sellers' interests. | 2026-09-02, by silence |
| D27 | As recommended: if the upload is itself a marketplace request that must originate on the seller's machine, the bytes must be there at upload time, which moves the ingest pipeline client-side and keeps the file off our servers. D1's API branch narrows the scope by construction rather than by amendment, because an Etsy upload travels the sanctioned API from the server, so this decision binds the no-API branch. | 2026-09-02, by silence |
| D28 | New. The frontend stays SvelteKit, and with it Tauri v2 as the shell. Leptos, Crux, Dioxus, and React with TanStack Start were each assessed and each rejected; the evidence is in "Verified since the first draft" below. Four things are adopted from the assessment rather than from the frameworks: the WASM core-in-browser step, compiling `tam-types`, `tam-domain` and `tam-taxonomy` to `wasm32` and calling them from Svelte, which delivers the Rust-in-the-frontend benefit without a rewrite; `@tanstack/svelte-table` when a screen needs sorting, pagination and persisted columns; Crux's shell-contract discipline, a deliberately coarse core-to-shell API, which keeps a native phone shell open later; and Astro or a prerendered SvelteKit route for the teachouse.io landing page, because the app's root layout disables both server rendering and prerendering, which SvelteKit's own documentation calls a large negative for performance and search. | 2026-09-03 |
| D29 | New. Build infrastructure runs on NixOS. Windows bundles cross-compile locally through `cargo-xwin`, and release builds run on a GitHub Windows runner where the MSI and the signing step are native. macOS builds use free minutes on GitHub or Codemagic; no macOS virtual machine is used, because Apple's software licence agreement forbids macOS on non-Apple hardware. Android builds from NixOS through `androidenv`, and the first Play upload is done by hand, because Tauri does not automate an Android release. | 2026-09-03 |
| D30 | New. The user-facing wording is "your login never leaves your device", carried on the connect flow, the marketplace badge and the publish progress. It never claims a legal requirement, because no statute imposes this architecture: the reasons are the United States computer-access line on request origin and the credential-custody question, and both are presented as our design choice rather than as law. | 2026-09-02 |

Amended 2026-09-05: D4's metering axis is superseded by the prices the founder approved that day, and the 2026-09-05 decision governs wherever the two differ.
Metering is by resources kept in sync at every tier, with connected marketplaces retained as a second axis rather than as the primary one: Solo carries 100 resources and two marketplaces, Studio 400 resources and every available marketplace, Publisher unlimited resources and every available marketplace, and the free tier 20 resources on one marketplace.
Each paid tier also carries an annual migration allowance, of 50, 200 and 500 resources respectively.
D4's choice of Paddle and its rejection of Stripe Managed Payments are untouched by this amendment.
The prices themselves, and the rules by which a one-off migration combines with a tier, are recorded in `docs/notes/design/console-redesign-plan.md`.

### Questions answered from the sources rather than put to the founder

Should we follow PrimeLister's server-side credential storage: no, and the source recommends the same; it is evidence to note, not a pattern to adopt (`vendoo-architecture-and-market.md`, open question 1).
Should we probe how Whatnot is integrated: no, it is one marketplace outside our vertical (same file, open question 5).
Does the inventory board's third column become "needs attention": yes, because sold cannot be a column for an infinitely copyable file (`vendoo-workflows-and-ux.md` §10).
Should sales ingestion be built before a sales feed is proven readable: not in general, but TPT's per-resource stats are already proven readable through the gateway, so TPT-first ingestion is not blocked (`docs/notes/design/analytics-console.md`, "What exists today").
Is TPT's resource type still a seller-writable picker: no, denied by the DOM snapshot, which finds no Type-of-Resource control anywhere on the create route, so it is not a first-class TPT concept (`tpt-create-form-dom.md`, "Closing the seventeen gaps", item 6).
Do we build a cross-framework standards crosswalk: no, and record it as a deliberate non-goal, because no owner publishes one and TPT itself declined on structural grounds (`education-standards-sources.md`, "Cross-walks").
Is standards alignment a new term kind on the existing axis machinery or a separate field: a separate field with its own table, reusing the provenance and reconciliation machinery but not the projection-edge relation (same file, open question 4).
What happens to standards on a Tes projection: a broadened edge for Common Core preserving the framework claim, and a no-counterpart record for the other three, both of which the existing types already express (same file, open question 7).
Is Boom Learning a projection target: no, excluded, and revisited only as a conversion feature (`cross-marketplace-mapping-tpt-base.md`, open question 6).
Is Teach Simple in scope: deferred until listing-copy generation is a dependency we are happy to take, because a copied description is non-compliant there by construction (same file, open question 7).
Do per-seller overrides ship with the first multi-target release: after, except the per-seller shelf mapping, which is not optional on Etsy, Boom or Shopify (same file, open question 8).
Does the registry carry a sanctioned-transport class per inventory: yes, it is small and it makes the architecture fork auditable by a test rather than by memory (same file, open question 9).
Do we re-base the canonical vocabulary on TPT: the brief decides this, and the cost is a re-seed rather than a rewrite (same file, open question 1).
Is Linux a launch target for the desktop client: no, Windows and macOS first with Linux best-effort, because WebKitGTK is the largest maintenance liability and it is confined to Linux (`docs/notes/design/client-side-architecture.md` §10).
Does Vendoo's own browser-based execution move the architecture fork: the founder has already moved it, so the question is closed by the brief itself (`vendoo-workflows-and-ux.md`, open question 4).

## Verified since the first draft

Seven groups of facts were checked after the first draft, and each one moved a decision, so they are recorded here rather than left in the conversation that produced them.

CrabNebula Cloud has been free since 2026-06-19 with no tiers, distributes desktop bundles only and does not build them, serves updates from `cdn.crabnebula.app/update/ORG/APP/{{target}}-{{arch}}/{{current_version}}` under our own signing key, and deletes a release that has gone ninety days without a download (verified 2026-09-02 against CrabNebula's own documentation and pricing pages).
Taurify is a different product that does build on its vendor's servers, including Android to Play and iOS to the App Store, and it cannot build this application: its configuration schema admits a Deno JavaScript backend wrapping a web bundle, desktop only, with no Rust, no Cargo, no plugins and no sidecar (verified 2026-09-02).
The consequence is that our builds run in our own CI, and `tauri-plugin-updater` is desktop-only, so phone updates ship through the stores rather than through our updater.

Stripe Managed Payments is not available to New Zealand sellers: Stripe's eligibility page excludes New Zealand and its New Zealand pricing page renders "Not available in your country" (verified 2026-09-03).
Plain Stripe therefore makes us the seller of record, with Stripe Tax calculating and registering while the filing stays ours outside the US, against Paddle's flat 5 percent plus 50 cents as merchant of record.
Stripe's agent ecosystem, which is the founder's reason for asking, is real, comprising an MCP server, an agent toolkit and a documented agent setup; it is not enough to carry the tax obligation, which is what D4 turns on.

Build infrastructure was verified on 2026-09-03 against nixpkgs and Tauri's own documentation.
A Windows NSIS bundle cross-compiles from Linux through `cargo-xwin`, which Tauri's documentation calls a last resort, while an MSI needs Windows and code signing is external either way.
nixpkgs carries `cargo-xwin` 0.23, `nsis` 3.12, `cargo-tauri` 2.11.4 and `androidenv` NDK 29, so the local toolchain exists today.
Apple's software licence agreement forbids running macOS on non-Apple hardware, which rules out a virtual machine; the free macOS minutes are roughly 200 a month on GitHub's free plan at its ten-times multiplier and 500 a month on Codemagic, and a physical Mac is still needed for Keychain certificate export and on-device iOS work.

better-auth 1.7.2 exposes `listSessions`, `revokeSession`, `revokeOtherSessions` and `revokeSessions`, and its session row stores `ipAddress` and `userAgent` with no device-name concept, so the device registry adds its own columns (verified 2026-09-02).
The `multiSession` plugin is multiple accounts in one browser rather than a device registry, and the cookie cache keeps a revoked session usable until the cache expires, which is the revocation latency the "Your devices" page has to state rather than hide.
Device identity is self-generated because `machine-uid` supports neither Android nor iOS: an id written once into application data, labelled with the hostname through `os:allow-hostname`, with Google's "Your devices" and GitHub's Sessions page as the precedents for the surface.

The frontend alternatives were assessed on 2026-09-03 and all four were rejected.
Leptos is described by its own maintainer's 2026-05-08 status update as "not abandoned but will be lightly maintained going forward", a migration of this tree prices at 90 to 140 engineer-days for no user-visible capability, and the "native" and "simple" claims do not survive contact: the runtime is the same WebView2 and WebKitGTK substrate our SvelteKit build already targets, and better-auth, passkeys, Turnstile and Paddle stay JavaScript regardless (`leptos-frontend-fit.md` §2, §5, §8 and §9).
Its component ecosystem leaves tree, upload progress, rich text and tab accessibility open even after Leptodon, the one entry that changes an estimate, which saves three to five days of twenty-nine to forty-five (`leptos-ui-library-coverage.md`, "The five hard components, priced"; `awesome-leptos-review.md` §2 and §4).
Crux is the pattern this engine already implements by hand, and its HTTP capability asks the shell to perform the request, which would hand our envelopes to URLSession, OkHttp or fetch and invalidate the live-fire and cassette evidence (`crux-and-dioxus-fit.md` §3 and §8).
Dioxus has no role here: its desktop and mobile renderers are Tauri's own substrate, and its configuration cannot host our SvelteKit build at all (same file §5 and §6).
TanStack Router and Start have no Svelte adapter and a React rewrite prices at 45 to 80 engineer-days, while `@tanstack/svelte-table` is current and worth taking when a screen needs sorting, pagination and persisted columns (`tanstack-and-astro-fit.md` §2 and §3).
Astro's own documentation excludes logged-in dashboards, so it fits the landing page and nothing else (same file §5 and §6).

The TPT create-form DOM snapshot is in hand and verified as the full `ItemAddForm`: `~/downloads/tpt-create-form.html`, 286 KB, carrying both copyright assertions, the standards picker and `status_user` (2026-09-03).
Its control-by-control reading has landed as `docs/research/rethink/tpt-create-form-dom.md`.

## Corrections to existing documents, and what is still unverified

Four corrections are owed to landed documents.
The legal memo's sentence "Server-side credential-holding automation appears nowhere in the comparable set" is wrong as of today's research, and a dated correction paragraph has been appended to it in place rather than editing the original sentence, recording PrimeLister's documented practice and stating that the verdict is unchanged.
The TPT adapter derives the additional-licence price as a fixed 90 percent of price, which reproduces TPT's default and cannot represent a seller who set a different value, so the derivation must become a form default rather than a projection rule (`tpt-product-model.md`, "Multiple licences").
Nothing in the tree defaults a tax code today, and nothing should start: the field is required for paid resources and the determination is the seller's under TPT's terms (same file, "Tax codes").
The client-side architecture note's migration sequence omits the transport feature gate, which is step zero for every path because `reqwest` is currently an unconditional dependency, and it places the browser extension last at step 5, which the surfaces research promotes to step 4 on the ground that without it the web console cannot publish at all (`client-surfaces-and-cross-compile.md` §2 and §10).

Consolidated unverified items, and what would close each.

| Item | What would verify it |
|---|---|
| No cross-target compilation has been performed anywhere; every portability claim rests on upstream documentation and upstream CI | Phase 0's CI job |
| Whether the TPT and Tes login pages carry a challenge an embedded webview cannot clear | Closed 2026-09-03 by the Windows WebView2 runs of D12's probe: Tes CLEAR, TPT CHALLENGE on a marker matching a feature-flag name with no reCAPTCHA, Cloudflare or other bot-protection asset in either page, both carrying a password form (`login-probe-windows.md`) |
| The exact wording of TPT's copyright assertions, and whether they are two checkboxes over one enum or a radio | Closed 2026-09-03 by the DOM snapshot: a single radio group at values `1` and `2`, both wordings quoted in `tpt-create-form-dom.md` |
| TPT's subject-area cap: the form says three, the capture posted four | A capture of the picker refusing a fourth selection, or accepting a fifth |
| The bundle create form's field set, above all how the member list is carried | A capture of a bundle create |
| Whether TPT's standards node ids are stable across a reindex | A second crawl diffed against the first, on a schedule |
| Whether Tauri's cookie read works on iOS, which is inferred from its absence from the unsupported list | A device test in Phase 7 |
| Whether Manifest V3's WebAssembly content-security-policy allowance covers our own build's glue code | A spike in Phase 5 week one |
| The wasm redirect-classification rework's real cost | The same spike, against the existing captures |
| Etsy's seller-facing limits: 140-character title, 13 tags, 5 files of 20 MB | An API key and a read of the seller taxonomy, or a successful retrieval of the Etsy help centre, which returns 403 to this network |
| Every Made By Teachers fact, including its file-size caps | Any retrieval of the site, which returns 403 on every path |
| Classful's seller create form, its caps, requiredness and licence options | A seller account on the platform |
| Whether Vendoo's mobile apps capture sessions in an embedded webview | Inferred from a Play review describing a magic-link page inside the app; Vendoo never states the mechanism |
| Windows code-signing cost, carried forward at roughly $120 a year | Microsoft's own pricing page renders both tiers as "$-" |
| Whether zero-rated export turnover counts toward the New Zealand GST registration threshold | An accountant, before the first paid signup |
