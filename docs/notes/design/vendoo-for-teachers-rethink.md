# Vendoo for teachers: the rethink

- date: 2026-09-02
- status: design memo answering the founder's rethink brief; nothing built, no marketplace contacted, no decision taken here
- inputs: the six research notes written today under `docs/research/rethink/`, and the landed notes `docs/notes/design/client-side-architecture.md`, `docs/notes/legal/marketplace-terms-assessment.md`, `docs/notes/design/creation-flow.md`, `platform-linking.md`, `analytics-console.md`, `admin-backoffice.md`, `billing-vendor-memo.md`

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
Recommendation: amend the non-negotiable, do the seam work and ship a Tauri desktop client as the first milestone, add the browser extension as the web surface's data plane next, treat phones as full parity for work the seller starts and never for unattended sync, and hold the whole programme at 33 to 49 weeks for one engineer with the first shippable product 11 to 17 weeks in.

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

### The field catalogue

| Group | Field | Type | Cardinality | Required | Conditional rule | Value source |
|---|---|---|---|---|---|---|
| Name | Title | plain text | one | yes | screened by `CheckResourceTitle` before submit | free text, 80-char client constant |
| Files | File to Upload | opaque handle | one | yes | handle from the S3 then `process_file` then `queue/results` chain | 4 GiB cap, 49 extensions |
| Files | Product Preview | opaque handle | one | no | same upload chain | 30 MiB cap |
| Files | Video preview | opaque handle | one | no | behind the `VideoPreviewForDigitalProduct` A/B flag | 1 GiB cap, 11 extensions |
| Files | Auto-generated thumbnail | integer plus handle | one | yes | create posted 1, edit posted 3; the enum is not enumerated anywhere | observed values only |
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

### The copyright gap and the title pre-check

The exact wording of the two copyright assertions is the largest single gap in the TPT model: the form is client-rendered by TPT's React bundle, the HAR records every JavaScript chunk with a zero-length body, and the captured screenshot crops above the copyright section, so the assertions are unquoted (`tpt-product-model.md`, "Copyright").
The wire carries one field with two members rather than two booleans, so if the form genuinely renders two independent checkboxes then either exactly one may be ticked or a second field exists that neither capture posted, and that is unresolved (same section).
Our adapter's handling is already the right shape and should not be relaxed: `AuthorshipDeclaration` has no default and no `from_bool`, and its only constructor names the attesting party and the instant, so nothing in the crate can put a declaration on the wire without a seller having actually attested (same section).
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
TEKS: the standards are Chapters 110 to 128 of Title 19 of the Texas Administrative Code and therefore state law, while TEA's own documentation site restricts registered users to personal, noncommercial use and forbids bulk download, and which of those controls a commercial tool caching TEKS codes is a legal question rather than an engineering one (same file, "What we may do with each framework").
Etsy adds a second non-delegable field class: `who_made` is required on every create, takes `i_did`, `someone_else` or `collective`, and is an attestation about authorship not derivable from any TPT field, with `when_made` riding along because Etsy requires the pair (`cross-marketplace-mapping-tpt-base.md`, "Resolution modes, with licence as the worked exemplar").
One correction to the legal memo, which is applied at the memo itself as a dated paragraph: PrimeLister's cloud Poshmark automation asks the seller to enter their Poshmark username and password, stores those credentials encrypted server-side, and documents that connecting without saving them is not currently possible, so server-side credential-holding does exist in the comparable set (`vendoo-architecture-and-market.md` §6).
That correction does not change the verdict: it removes "nobody does this" from the risk argument, and it leaves the memo's exposure analysis untouched, because the analysis never rested on the practice being unique, and TPT is a materially more litigious counterparty than Poshmark (same section).

## Methodology and sequence

### How the research becomes a build

The ordering principle is that the data plane rather than the surface is the unit of work, and that every step is independently useful if the next is deferred (`client-surfaces-and-cross-compile.md` §10).

Phase 0, one week: add a `live` Cargo feature to both adapter manifests, gate the transport files behind it, add the send-bound shim to the transport trait, add the iOS, Android and wasm targets to the toolchain file, and prove `cargo check --target` for the pure core in CI.
Proves: the whole portability argument, converting the largest unverified claim into a green check.
Kill gate: a target that does not build.
Verification: the CI job itself, which fails under any incorrect implementation because it compiles the real crates against the real targets.
Founder acts gating it: none.

Phase 1, four to six weeks: split the engine driver from concrete storage repositories behind a repository trait, so the effect interpreter runs against a remote ledger.
Proves: every later surface can host the engine, and none of the work is wasted if the surface choice changes.
Kill gate: the split cannot be made without changing engine semantics.
Verification: the existing engine tests pass unchanged against an in-memory ledger, which would fail under a split that leaked storage assumptions.

Phase 2, six to ten weeks, the first shippable milestone: a Tauri v2 desktop client for Windows and macOS, with the webview used for login only, the existing transport unchanged, a client-side scheduler, OS-keychain session storage, entitlement verification, signed installers and an updater.
Proves: the client-side thesis against real marketplaces, and it is a complete product for a seller with a computer.
Kill gate: the login-page bot-protection probe, which decides whether an embedded webview can clear the challenge at all.
Verification: a live create and publish on the founder's own TPT and Tes stores from the desktop client, read back through the authoritative API rather than trusting the write's own status code.
Founder acts gating it: the live read-only probe of the TPT and Tes login pages, an Apple developer account, and Windows code signing.

Phase 3, four and a half to seven weeks: re-base the canonical vocabulary on TPT, add the per-seller override layer, route the axes not yet bound, add the drift job.
Proves: the base-product decision is true in the data rather than only in the prose.
Kill gate: a residue report showing the TPT-to-Tes re-derivation loses a mapping the current direction holds.
Verification: the seed residue report, plus the existing projection tests re-run in the inverted direction.
Founder acts gating it: the TPT create-form DOM snapshot, which closes the copyright wording gap and eight other gaps at once with no write and no submit.

Phase 4, three to five weeks: standards ingestion for the four frameworks plus the TPT node-id crawl.
Proves: a teacher can tag standards and have them post.
Kill gate: TPT's node ids prove unstable across a re-crawl.
Verification: a hand-checked sample per framework against the owner's PDF, and a diff of Virginia's two independent mirrors against each other.
Founder acts gating it: the WestEd sample submission, which must start in Phase 0 because six weeks is longer than the phase, and a written answer from TEA on the terms-of-service question.

Phase 5, six to eight weeks: the Manifest V3 browser extension as the web surface's data plane.
Proves: a teacher can publish from a browser without installing an application.
Kill gate: the redirect-classification rework against the existing captures, which should be spiked in week one because a browser fetch has no equivalent of the manual redirect policy the TPT create classifier reads.
Verification: the existing cassette fixtures replay green through the fetch transport.

Phase 6, two to three and a half weeks: the Etsy connector, which is the only sanctioned API in the set.
Founder act gating it: the Etsy app registration, filed early because Commercial Access is a manual review with no published service level.

Phase 7, four to six weeks each: iOS then Android, reusing the same project and the same console build, with foreground-only publishing, camera capture, and a "run now" dispatch to the desktop agent.
Founder act gating it: Apple and Google developer accounts, with the Google Play account registered as an organisation now, because a personal account faces a twelve-tester, fourteen-consecutive-day production gate that is wall-clock and cannot be compressed.

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

## Decisions for the founder

"Silence" in the last column means the recommendation stands unless the founder says otherwise.
"Words" means an explicit answer is required, because the decision is security-sensitive, spends money, is irreversible, is a public commitment, or takes a live action against a marketplace.

| # | Question | Recommendation | Adoption |
|---|---|---|---|
| D1 | Does the architecture non-negotiable change? | Yes. Replace "Automation runs server-side, on infrastructure we operate. The client is thin and performs no automation; it renders progress." with: "Automation runs on the seller's own device. The server is a control plane: it holds the catalogue, the mapping decisions, the ledger, the dashboard, the subscription and the kill switch, and it sends declarative intent describing an outcome. It never composes, signs or issues a marketplace request. The client composes and issues every marketplace request under the seller's own session, and the seller owns the schedule; the schedule remains deterministic and cron-shaped, and only the location of the timer moves." Amend the matching lines in `docs/design/decisions.md` under "Architecture", including "Web client first, Android second" and "Mobile is a full client rather than a read-only one, because the client performs no automation"; strike "Desktop clients, which the server-side architecture removes the need for" from "Scope explicitly deferred"; and add a dated entry recording the reversal and its reason. | Words |
| D2 | Which surfaces, in which order? | Desktop first, then the browser extension, then iOS, then Android, per the phase plan; A4 (extension only) is the cheaper alternative and is rejected because it delivers no phone and no deterministic schedule. | Words |
| D3 | What does "same functionality as web" mean on a phone? | Redefine it as full parity for work the seller starts, with unattended sync stated in the product as requiring the desktop agent, because no phone can run a deterministic schedule. | Words |
| D4 | What is the billing metering unit? | Not Vendoo's new-items-per-month, which punishes exactly our customer. Meter connected marketplaces or catalogue size, and set it before the Paddle plan table is built. | Words |
| D5 | What is the update contract when a live file changes? | A new product version, every mapping marked out of date, one "update everywhere" action lowering to a revise per mapping, per-marketplace results reported individually, and no buyer notification unless the marketplace offers a native one. | Silence |
| D6 | Does the canonical model carry an explicit additional-licence price? | Yes. Carry it explicitly and default the field to TPT's 90 percent pre-fill in the form only, never in the projection, so a seller's custom price is not overwritten on every sync. | Silence |
| D7 | Does the tool ever default a TPT tax code? | No. TPT's terms make the seller responsible for designating it, and the existing refusal to synthesise a copyright attestation is the precedent. | Silence |
| D8 | Which three projection targets ship first? | Tes, Etsy and Classful: Tes is built, Etsy is the only sanctioned API, and Classful's vocabulary is closest to TPT's and therefore cheapest to crosswalk. | Silence |
| D9 | What happens to a free TPT product projected onto Etsy, where a free price is impossible? | A named publish gate offering a minimum-price election, never a silent floor. | Silence |
| D10 | Is the entitlement token an accepted exception to "authorisation is never asserted by a token claim"? | Yes, scoped: Postgres remains the decision-maker, the token only transports a decision already made, the server re-checks on every control-plane call, and the client-side gate is fail-closed and advisory. | Words |
| D11 | What is the kill-switch latency we commit to publicly? | One-hour token validity plus a 24-hour grace, failing closed, chosen deliberately rather than discovered under pressure. | Words |
| D12 | Do we run the live login-page bot-protection probe? | Yes, read-only, from the founder's own account on both TPT and Tes, from a stock browser and from an embedded webview. It gates the entire desktop client and nothing else should be committed before it answers. | Words |
| D13 | Do we take a DOM snapshot of the whole TPT create form? | Yes. It needs no write and no submit, and it closes nine of the seventeen recorded TPT gaps including the copyright wording. | Silence |
| D14 | Per-surface marketplace login, or an encrypted session relay between the seller's own devices? | Per-surface login, stated plainly in the UI; the relay reintroduces the custody question the whole architecture exists to remove. | Words |
| D15 | Are bundles and the non-digital TPT product types in scope for the first release? | Bundles yes as a modelled concept but not as a first-release publish path, because no bundle create has ever been captured; Easel, Online Resource and Video out until each is captured. | Silence |
| D16 | Multiple credentials per marketplace from the start? | Yes. Teachers with a personal and a school-district account will hit Vendoo's one-per-marketplace limit, and the existing global exclusivity index needs designing around it now rather than later. | Silence |
| D17 | Is there ever a delist verb? | Yes, but named "retire from this marketplace", never "refresh", and never offered as a way to update a listing. | Silence |
| D18 | Do we submit NGSS samples to WestEd now? | Yes, in Phase 0, because the stated turnaround is four to six weeks and that is longer than the phase that needs it. | Words |
| D19 | Do we ask TEA for a written answer on the TEKS terms of service? | Yes, and until it arrives ingest TEKS from the Common Standards Project mirror, which carries a CC BY licence and no bulk-download restriction. | Words |
| D20 | Do we pay for Academic Benchmarks? | No. Its free tier leaves either TEKS or VA SOL uncovered, its paid tier is unpriced publicly, and its identifier buys nothing that TPT's node id does not. | Words |
| D21 | Etsy app tier: one Commercial Access app we hold, or a Seller App per seller? | Commercial Access as the target, filed early, with per-seller Seller Apps as the bootstrap that does not wait on review. | Words |
| D22 | Is the Google Play account registered as an organisation? | Yes, now, because a personal account faces a twelve-tester, fourteen-consecutive-day gate that is wall-clock and cannot be compressed. | Words |
| D23 | Do we plan for in-app purchase outside the US storefront? | Ship US-first with an external link, and treat non-US in-app purchase at a 15 to 30 percent cut as a funded later milestone. | Words |
| D24 | Do we enforce TPT's most-favoured-pricing rule as a publish gate? | Yes, as a gate rather than a warning, which converts a contractual liability into a product feature. | Silence |
| D25 | Do we display standards statement text, or codes only? | Display the text, accept the Common Core attribution obligation, and never paraphrase it anywhere; a code-only picker is close to unusable. | Silence |
| D26 | Do we offer a marketplace-side import-from-TPT to challenger marketplaces, the TeachBuySell shape? | Park it. It is a real business, it is a different one, and it competes with our own sellers' interests. | Silence |
| D27 | Do the resource file bytes stay client-side too? | Yes. If the upload is itself a marketplace request that must originate on the seller's machine, the bytes must be there at upload time, which moves the ingest pipeline client-side and keeps the file off our servers entirely. | Silence |

### Questions answered from the sources rather than put to the founder

Should we follow PrimeLister's server-side credential storage: no, and the source recommends the same; it is evidence to note, not a pattern to adopt (`vendoo-architecture-and-market.md`, open question 1).
Should we probe how Whatnot is integrated: no, it is one marketplace outside our vertical (same file, open question 5).
Does the inventory board's third column become "needs attention": yes, because sold cannot be a column for an infinitely copyable file (`vendoo-workflows-and-ux.md` §10).
Should sales ingestion be built before a sales feed is proven readable: not in general, but TPT's per-resource stats are already proven readable through the gateway, so TPT-first ingestion is not blocked (`docs/notes/design/analytics-console.md`, "What exists today").
Is TPT's resource type still a seller-writable picker: unmeasured, so do not carry it as a first-class TPT concept until the DOM snapshot says otherwise (`tpt-product-model.md`, open question 3).
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
| Whether the TPT and Tes login pages carry a challenge an embedded webview cannot clear | The founder-gated live read-only probe, D12 |
| The exact wording of TPT's two copyright assertions, and whether they are two checkboxes over one enum or a radio | The DOM snapshot, D13 |
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
