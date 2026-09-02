# Cross-marketplace mapping with TPT as the base model

Read-only public-source research, conducted 2026-09-02.
Every URL below was fetched on 2026-09-02 unless a different date is stated beside it.
No account was created, nothing was signed into, and no authenticated request was sent to any marketplace.
The TPT side of the mapping is taken as given from `docs/research/rethink/tpt-product-model.md`; this document works the other platforms and the engine.

## Executive summary

Only two of the eight target families have an official write API, and only one of those is a teacher marketplace: Etsy, whose Open API v3 creates a digital listing in four calls but forbids a free price, caps digital files at five of twenty megabytes, and holds no concept of grade, subject or standard.
Shopify is the second, and it is a storefront rather than a marketplace, so its "listing" carries none of the discovery metadata TPT's form is mostly made of.
Gumroad, Payhip, Sellfy and Lemon Squeezy all have APIs that cannot create a product, which puts them in the same automation class as the no-API marketplaces.
No marketplace we found offers an import-from-TPT except TeachBuySell, an Australian marketplace whose import is a working proof of exactly our thesis: it pulls titles, descriptions, prices, thumbnails and preview files from a public TPT store URL, auto-maps TPT categories onto its own subjects, year levels and resource types, converts currency, rewrites in-description TPT links, and explicitly cannot transfer the sellable files.
The metadata projects; the file never does.
Standards alignment is the worst-supported canonical field: it survives as a coarse framework name on Tes and Classful, as prose on Boom, and not at all on Etsy or any storefront.
Grade is the best-supported: Classful's sixteen values are near-identical to TPT's list, Boom's span preschool to university, Tes needs the age-band crosswalk we already hold, and Etsy has nothing but thirteen twenty-character tags.
Two targets are not projection targets at all — Boom Learning, whose product is an interactive deck authored in its Studio rather than a file, and Teach Simple, which has no seller-set price and forbids descriptions copied from another marketplace.
Extending the current engine to the design below is roughly four to six engineer-weeks, and each new target costs one to two days of vocabulary capture plus eight to twelve days of adapter for an API target and ten to eighteen for a form-driven one.
The mapping engine is indifferent to the server-side and client-side split because projection is a pure function producing a field set and a loss report, and where that value is spent is the adapter's business, not the mapping's.

## Method and sources

Primary sources were preferred throughout, and the strongest of them is machine-readable: Etsy publishes its whole contract as an OpenAPI 3.0.2 document at https://www.etsy.com/openapi/generated/oas/3.0.0.json (fetched 2026-09-02, 896 KB, 76 paths), and every Etsy field, enum and requirement below is read out of that file rather than from prose.
Classful publishes its browse facets as server-rendered HTML at https://classful.com/shop/ (fetched 2026-09-02), so its grade, subject, standard, language and resource-type vocabularies are captured rather than described.
Tes needs no new research: `docs/design/data/tes-vocabulary.json` and the two taxonomy captures beside it are live polls of the uploader's own reference data from 2026-08-25 and 2026-08-29, and `docs/notes/mapping/vocab-equivalence.md` already states the structural mismatch.
The legal reading of each platform's terms is not repeated here; `docs/notes/legal/marketplace-terms-assessment.md` (2026-08-31) owns it, and this document cites it only where a term constrains the upload path.

Three sites refused every request from this network.
`help.etsy.com` returns 403 to both the fetch tool and a plain curl, so Etsy's seller-facing limits — the 140-character title, thirteen tags of twenty characters, five digital files of twenty megabytes each, seventy-character filenames — are carried here from secondary write-ups that quote the help centre, and are listed under "Unverified" accordingly.
`madebyteachers.com` returns 403 on every path including its homepage, so every Made By Teachers fact below comes from search-result snippets quoting its own seller FAQ.
`boomlearning.zendesk.com` returns 403, but the newer `helpcenter.boomlearning.com` serves fine and is the source used.

## Target profiles

### Tes

Tes is a UK-registered marketplace (Tes Global Ltd, company 02017289) running disjoint per-country inventories, of which GB, US and NZ are already modelled as separate `InventoryId` values because their vocabularies genuinely differ.

Sign-up is free and self-service, requires an account holder aged 18 or over, and publishing is a five-step uploader — title and description, file upload and resource type, tag and categorise, price or licence, preview and agree to the Author Code — after which resources go live "within the next three working days" (https://www.tes.com/author-academy/getting-started/share-or-sell-your-teaching-resources-tes).
Product types are file attachments: fifty-eight extensions across seven type families, maximum 200 MB per file (`tes-vocabulary.json`, `fileConstraints`).

There is no public API and no developer programme; the retrieved corpus offers only a corporate partnerships channel (`marketplace-terms-assessment.md`).
The reference-data and taxonomy endpoints the vocabulary capture used are internal to the uploader rather than a documented seller interface, and treating them as an API is a decision about risk, not a finding about availability.
There is no import-from-TPT and no bulk upload of any kind.

The fields, from the captured reference data: title with no measured cap and guidance of 35 to 45 characters; description as Markdown or HTML declared per resource by `descriptionRawType`, round-tripping byte-intact; price as an integer in minor units with a floor of 1.00 everywhere except the US at 1.50 and a ceiling of 300.00; a required `licence` from five writable values, gated by price so that a free resource picks among CC-BY, CC-BY-SA and CC-BY-ND and a paid one takes TES-PAID or the TES-PAID-SCHOOL school tier; exactly one `mainType` from nine values; age as seven `ageRanges` bands in GB or thirty `yearGroups` rows elsewhere; roughly ten subject-and-topic pairs from a 43-root tree with exactly one `primaryCategory`; and an optional three-level curriculum cascade of orientation, framework and awarding body.
Status is publish-then-review rather than a state we set directly.
Tes carries no free-text tag field, no tax code, no teaching duration, no page count and no answer key.

### Etsy

Etsy is the only teacher-adjacent target with a documented, sanctioned, token-based write path, and it is a general commerce marketplace rather than an education one.

The API is the Open API v3, authenticated by OAuth 2.0 authorization code with PKCE mandatory on every authorization request, an access token that expires in 3600 seconds and a refresh token with a ninety-day functional lifetime (https://developer.etsy.com/documentation/essentials/authentication/).
The scopes that matter are `listings_r`, `listings_w`, `listings_d`, `shops_r` and `shops_w`, read from the spec's own security schemes.

Listing creation with digital file upload is supported, in a fixed four-call sequence.
`POST /v3/application/shops/{shop_id}/listings` (`createDraftListing`, scope `listings_w`) requires exactly seven fields — `quantity`, `title`, `description`, `price`, `who_made`, `when_made`, `taxonomy_id` — and takes `type` from the enum `physical | download | both`.
Files cannot be attached at create: `POST .../listings/{listing_id}/files` (`uploadListingFile`) takes a binary `file` as multipart, and `POST .../listings/{listing_id}/images` (`uploadListingImage`) takes a binary `image`, both after the draft exists.
`PATCH .../shops/{shop_id}/listings/{listing_id}` (`updateListing`) moves `state` from `draft` to `active`; the `ShopListing` state enum is `active`, `inactive`, `sold_out`, `draft`, `expired`, and an active listing requires at least one image.

The taxonomy is obtained from `GET /v3/application/seller-taxonomy/nodes` (`getSellerTaxonomyNodes`), which returns a tree whose nodes carry `id`, `level` with roots at zero, `parent_id`, `children` and `full_path_taxonomy_ids`; `getPropertiesByTaxonomyId` then returns the properties that node supports.
That tree is a commerce category tree.
It has no grade dimension, no subject dimension and no standards dimension, and Etsy's own market pages for teaching resources are search facets rather than taxonomy nodes (https://www.etsy.com/market/teachers_resources).
The consequence is structural rather than incidental: three of TPT's category fields have nowhere to land on Etsy except thirteen tags and the title.

Price is constrained in the spec's own words to "the positive non-zero price of the product", so Etsy cannot hold a free listing at all.
Tax is a single `is_taxable` boolean, and `shop_section_id` is a genuine counterpart to TPT's seller-defined custom categories.
Title validity is a character-class rule as well as a length rule: only letters, numbers, punctuation, mathematical symbols, whitespace and the ™ © ® marks, with `%`, `:`, `&` and `+` each usable only once.
Tags admit only letters, numbers, whitespace, hyphen, apostrophe and those same three marks.

Rate limits are per API key, expressed as queries per second and queries per day over a rolling twenty-four-hour sliding window, reported in `x-limit-per-second`, `x-remaining-this-second`, `x-limit-per-day` and `x-remaining-today`, and enforced with a 429 carrying `retry-after`; the documentation's illustrative figures are 150 QPS and 100,000 QPD, and higher quotas are requested by email (https://developer.etsy.com/documentation/essentials/rate-limits/).

App approval has three tiers, and the tier choice is a product decision rather than a formality.
A Seller App reaches only the developer's own shop and is approved within minutes; a Personal App reaches a limited number of other shops after a deeper review; Commercial Access is not a separate application but a second request made against an already-approved Personal App, and is manually reviewed against the proposed use case (https://help.etsy.com/hc/en-us/articles/41918478450967-How-to-Register-a-Seller-App-with-Etsy-s-API, https://github.com/etsy/open-api/discussions/1361).
Serving arbitrary sellers from one app requires Commercial Access; asking every seller to register their own Seller App avoids the review entirely at the cost of an onboarding step per seller.

There is no import-from-TPT and no bulk create; a bulk operation is N calls against the rate limit.

### Classful

Classful is a US-only marketplace (Classful LLC, Nevada) with free self-service sign-up as a "Shop" account type, charging a 5 percent seller fee plus 2.9 percent and 30 cents processing, and remitting sales tax in all fifty states on the seller's behalf (https://classful.com/sell-products/).
It states plainly that selling the same products elsewhere is permitted.

Its browse facets, captured from https://classful.com/shop/ on 2026-09-02, are the closest public evidence of its product model.
Grade is sixteen values: Kindergarten, Pre-K, 1st through 12th, Adult Ed and Higher Ed.
Subject is a two-level tree of roughly eighteen roots — Arts & Music, Back to School, Classroom, Computer, ELA, Engineering, History/Social Studies, Languages, Math, Other, Pro. Develop, Religion, Science, Seasonal, Social Emotional, Specialty, Vocational — with an "Other" leaf under most roots.
Resource type is a flat list of about fifty-seven values, from Activities and Assessments through Task Cards, Word Walls and Worksheets.
Standards is a facet of six frameworks: Behavior and Skills, CASEL Social Emotional Learning Competencies, Common Core English Language Arts Standards, Common Core Mathematics Standards, ISTE Standards, Next Generation Science Standards.
Language is a facet of about twenty-three values, and price is a browse band rather than a field.

Classful has no API and no developer programme, and its terms prohibit unauthorised programmatic interface access and operating "an unauthorized automated or machine-learning system" — the one clause across the assessed platforms that plausibly reaches automated writes (`marketplace-terms-assessment.md`).
There is no import-from-TPT.
The seller-side create form is not publicly reachable, so its per-field caps and requiredness are unmeasured.

### Made By Teachers

Every fact in this profile is second-hand: the site returns 403 to this network on every path.

Products are added through an "Add a New Product" button with all required fields plus a minimum of one preview image, and the resource file size cap depends on the account tier — 90 MB on a basic account, just under 200 MB on premium (https://www.madebyteachers.com/seller-faq/, quoted in search results, not retrieved).
Payouts are PayPal-only.
Updating a product file means deleting the old file and uploading the new one, and the FAQ recommends verifying with a "Test Download" button because incomplete or wrong uploads happen.
No API, no bulk upload and no import-from-TPT were found.
The listing vocabulary — grades, subjects, standards, tags — is unmeasured.

### Teach Simple

Teach Simple is not a marketplace in the sense the other targets are, and this is the single most consequential per-target finding after Boom.

There is no seller-set price.
Earnings come from a subscriber-share pool: each product is assigned a weighted value "based on the quality of the product, such as the design, number of pages, type of product ... and how much information you've provided (standards, description, etc.)", and a customer download attributes that product's share points to the contributor, with royalties released around the fifteenth of each month, a $50 minimum payout and PayPal disbursement (https://teachsimple.com/blog/contributors/new-contributor-onboarding/).

Two content rules bind a cross-listing tool directly.
Every product must carry "an original description of at least 150 words" and "You can not copy/paste descriptions from other marketplaces", which makes a verbatim TPT description non-compliant by construction.
Descriptions must not link to Teachers Pay Teachers, Made by Teachers or Etsy.
Products must be tagged "with the correct subject, grade level, and type", images must not be watermarked because the platform applies its own, and additional terms and conditions inside the product are forbidden in favour of a standard licence.

Every submission passes a QA review; new contributors sit in a regular queue until ten products have been approved, after which the account is flagged for express approval with a 48-hour turnaround.
No API, no bulk upload and no import-from-TPT were found.

### Amped Up Learning

Amped Up Learning is a small US marketplace selling printables, digital lessons, Google Forms, Boom Cards and physical goods side by side.

There is no self-service seller sign-up: the page directs prospective sellers to email `askus@ampeduplearning.com` to have a store set up (https://ampeduplearning.com/sell-with-us/).
Commission is 80 to 90 percent to the contributor with no monthly fee, rising to 90 percent during sitewide sales, and sellers can issue their own coupon codes.
The site's assets are served from `cdn11.bigcommerce.com`, which indicates a BigCommerce storefront rather than a bespoke marketplace; that is an inference from the asset host, not a statement the site makes.
No API, no bulk upload and no import-from-TPT are documented, and no listing-field detail is public.

### Boom Learning

Boom Learning is not a file marketplace, and treating it as a projection target would be a category error.

The product is a deck of interactive cards authored in the Boom Studio deck editor, and selling requires a Publisher subscription, a confirmed PayPal account and a chosen pen name (https://helpcenter.boomlearning.com/quick-start-for-authors-learn-to-sell-boom-cards).
A "Converting Files to Boom Cards" help category exists, so a PDF can become a deck, but conversion is authoring work rather than a field mapping.

The listing fields are set in the deck editor's Details box: grades from preschool through university with multi-select via shift and control and a "Not Grade Specific" option; subject with subtopics; an "About" description; keywords; price; acknowledgments for image and font credits; and card-order customisability.
Titles "should be 48-60 characters long max", with hashtags of four to twenty characters kept separate from the title (https://helpcenter.boomlearning.com/publishers-optimizing-product-discoverability).
There is no structured standards field; the guidance is "Do include information about standards" in the description, and it also forbids links to external stores there.
Subjects are credential-gated — Speech and Language is for trained speech-language pathologists, Occupational Therapy for trained occupational therapists, Special Education & ABA for trained specialists — and the platform reserves the right to demand proof.

Two platform rules constrain any automated publisher.
Only one copy of any deck may exist, and unpublishing, cloning and republishing a near-identical deck earns a strike against the account.
Free decks are capped: a non-Premier publisher may have twenty free decks at a time, and a Premier publisher fifteen percent of their catalogue plus one (https://helpcenter.boomlearning.com/publishing-decks-to-the-boom-store-for-free).
No public API, bulk upload or import was found.

### Generic digital storefronts

Grouped as one row because their answer to the mapping question is nearly identical: they hold the commerce fields and none of the discovery vocabulary.

Shopify is the only one with a real create path.
`productCreate` exists in the current Admin GraphQL API, requires the `write_products` scope, and takes `title`, `descriptionHtml`, `productType`, `vendor`, `status`, `tags`, `handle`, `seo`, `productOptions` and `metafields`, with a separate `media` argument for images and video (https://shopify.dev/docs/api/admin-graphql/latest/mutations/productCreate).
Digital file delivery is not part of the core product model and is delegated to an app, so a digital listing needs an app dependency on top of the API.

Gumroad publishes a `POST /v2/products` route whose own documentation states that product creation via the API is not currently supported, and an open issue requests it (https://github.com/antiwork/gumroad/issues/4019).
Payhip's public API covers licence keys and webhooks; no create-product endpoint was found, and products are created in the dashboard, where each file may be up to 5 GB.
Sellfy has no public REST API; products are created in the dashboard, with up to fifty files per product.
Lemon Squeezy's API exposes Products, Variants, Prices and Files as retrieve-and-list only, and its create verbs are for checkouts, discounts, licence keys and usage records (https://docs.lemonsqueezy.com/api).

So the grouped row is one API target and four that are, for our purposes, no-API targets.
None of the five carries grade, subject or standards; the nearest homes are tags, product type, collections and metafields.

### Two targets found during this research

TeachBuySell is an Australian marketplace and the only platform found that offers an official import from TPT, which makes it the strongest external evidence for the projection thesis.
The seller pastes a TPT store URL, the platform confirms the store name and product count, then "collect[s] your products directly from TPT", usually in under a minute (https://teachbuysell.com.au/help/import-tpt-products).
The import carries titles, descriptions, prices, thumbnails and preview files; converts USD prices to AUD at an adjustable rate defaulting to a 45 percent markup; auto-maps TPT categories onto its own subjects, year levels and resource types; rewrites in-description links pointing at other TPT products so they point at the seller's imported listings, and removes links to products that were not imported; deduplicates against prior imports and identical titles; and produces draft listings only.
It explicitly cannot carry the sellable files — "we can't transfer files you sell there" — so each draft still needs the resource downloaded from TPT and dragged into its Files & Previews tab before publishing.
Access to the tool is gated behind publishing a first listing and passing a quality review.
Its own listing form is organised as Title and description, Specifics (categories, year levels, curriculum alignment) and Files & Previews (resource files, preview images, preview PDFs) (https://teachbuysell.com.au/help/creating-a-listing).

Teacha! is a South African marketplace operated with Snapplify, running since 2015 and claiming presence in more than fifty countries, with free membership, a 35 percent commission on paid products and a roughly two-working-day review before a resource goes live (https://teachahelp.snapplify.com/support/solutions/articles/80000763792-uploading-your-resources-to-start-selling).
Files may be PDF, ZIP, Word, PowerPoint, Excel or SMART Notebook, with a 60 MB maximum, and two or more files in one product make a bundle.
No API, no bulk upload and no import were found, and its per-field constraints are unmeasured.

## The projection table

Rows are the TPT canonical fields as `docs/research/rethink/tpt-product-model.md` names them.
Cells use five verdicts, and the distinction between the last two is the one the engine already draws in `crates/tam-domain/src/registry/mod.rs`.

- adopt — the target has the same field and takes the value unchanged.
- adjust — the target has a counterpart reachable by the named transformation.
- drop — the target has no such field; the value stays in the canonical product, the projection records the loss, and the seller sees it in the field diff before publish.
- unsupported — the target's model refuses the concept, so there is no honest projection and the axis blocks rather than degrading.
- unmeasured — nothing is captured, so the projection blocks; this is not evidence that the field is absent.

| TPT field | Tes | Etsy | Classful | Made By Teachers | Teach Simple | Amped Up Learning | Boom | Storefronts |
|---|---|---|---|---|---|---|---|---|
| Title | adopt | adjust: 140 cap, char class, `%:&+` once each | unmeasured | unmeasured | adopt | unmeasured | adjust: 48-60 chars | adopt |
| Description | adjust: strip external URLs | adjust: HTML flattened to text | unmeasured | unmeasured | unsupported: must be original, 150+ words, not copied | unmeasured | adjust: strip store links, fold standards into prose | adopt |
| File to Upload | adjust: 58 extensions, 200 MB | adjust: 5 files, 20 MB each | unmeasured | adjust: 90 MB basic, ~200 MB premium | unmeasured | unmeasured | unsupported: deck authored in Studio | adopt |
| Product Preview | adjust: a typed attachment | adjust: preview PDF becomes images | unmeasured | adjust: at least one preview image | unmeasured | unmeasured | adjust: cover image | adjust: media image |
| Auto-generated thumbnail | adjust: we supply it | adjust: we supply it, required for active | adjust: we supply it | adjust: we supply it | adjust: we supply it | adjust: we supply it | adjust: cover with content icons | adjust: we supply it |
| Free Resource | adjust: forces a CC licence election, and free to paid is irreversible | unsupported: price must be positive non-zero | adopt | unmeasured | unsupported: no price exists | unmeasured | adjust: capped at 20 free decks, or 15% + 1 | adopt |
| Price | adjust: minor units, floor 1.00 (US 1.50), ceiling 300.00, currency by inventory | adjust: positive non-zero, shop currency | adopt | unmeasured | unsupported: subscriber-share pool | unmeasured | adopt | adopt |
| Multiple Licenses | adjust: no per-seat price; TES-PAID-SCHOOL is the only tier | unsupported: a variation is a SKU, not a grant | unmeasured | unmeasured | unsupported | unmeasured | unsupported | unsupported |
| Bundle Discount Price | drop | unsupported | unmeasured | unmeasured | unsupported | unmeasured | adjust: bundles with complete-my-bundle differential | unsupported |
| Tax Code | unsupported: platform handles tax | adjust: 5 codes collapse to `is_taxable` | unsupported: Classful remits in all 50 states | unmeasured | unsupported | unmeasured | unsupported: platform handles tax | adjust: store tax settings |
| Grade Level | adjust: 15 exact US `yearGroups` rows, GB via age-band covering, 4 no-counterpart | drop: no grade concept, tags only | adjust: 16-value crosswalk, 3 TPT values no counterpart | unmeasured | adjust: tag with grade level | unmeasured | adjust: preschool to university, multi-select | drop: tags or metafields |
| Subject Area | adjust: 43-root tree, ~10 pairs, one `primaryCategory` | drop: `taxonomy_id` is a commerce node, tags only | adjust: 18-root two-level crosswalk | unmeasured | adjust: tag with subject | unmeasured | adjust: subject plus subtopics, credential-gated | drop: tags or metafields |
| Tag | unsupported: no free tag field | adjust: 13 tags, 20 chars each, char class | adjust: keywords, cap unmeasured | unmeasured | adjust | unmeasured | adjust: keywords, anti-stuffing enforced | adopt |
| Format | adjust: per-file attachment type | drop: derived from the files | unmeasured | unmeasured | adjust: type tag | unmeasured | unsupported | drop |
| Custom Category | adjust: collections, not a create-form field | adopt: `shop_section_id`, per-seller ids | unmeasured | unmeasured | unmeasured | unmeasured | adjust: store folders | adopt: collections |
| Education standards | adjust: coarse curriculum triple, leaf ids dropped | drop | adjust: 6-framework facet; TEKS and VA SOL have no counterpart | unmeasured | adjust: an input to the share weighting, format unverified | unmeasured | drop: prose in the description | drop |
| Teaching duration | drop | drop | unmeasured | unmeasured | unmeasured | unmeasured | drop | drop |
| Number of pages or slides | drop | drop | unmeasured | unmeasured | adjust: an input to the share weighting | unmeasured | drop: card count is intrinsic | drop |
| Answer Key | drop | drop | unmeasured | unmeasured | unmeasured | unmeasured | drop | drop |
| Copyright declaration | adjust: compliance checkbox and agency warranty | adjust: `who_made`, `when_made`, `is_supply` are required and non-delegable | unmeasured | unmeasured | adjust: platform-standard licence, no per-product terms | unmeasured | adjust: acknowledgments | unsupported |
| Making Listing Active | adjust: publish, then review up to 3 working days | adopt: `draft` to `active`, image required | adopt | unmeasured | adjust: QA queue gates live | unmeasured | adjust: publish to store or private publish | adopt: created unpublished, then `ACTIVE` |

The two targets found during this research, on the rows that carry information:

| TPT field | TeachBuySell | Teacha! |
|---|---|---|
| Title, Description, Price | adjust: imported, with USD to AUD conversion and TPT links rewritten | unmeasured |
| File to Upload | unsupported by their own import; the seller re-uploads by hand | adjust: 60 MB, PDF, ZIP, Office, SMART Notebook |
| Grade Level, Subject Area, Format | adjust: TPT categories auto-mapped to year levels, subjects and resource types | unmeasured |
| Education standards | adjust: a curriculum-alignment field exists | unmeasured |
| Making Listing Active | adjust: import creates drafts only, quality review gates publish | adjust: ~2 working days review |

### What happens to the standards data

Standards is the field where "drop" needs saying precisely, because the data is valuable and the temptation to approximate is strongest.

On Tes the claim degrades rather than vanishing.
A TPT product carrying Common Core leaf ids can justifiably set orientation American and framework Common Core, which preserves the alignment claim and loses every specific standard; NGSS, TEKS and Virginia SOL have no counterpart in the eleven-orientation cascade at all.
On Etsy nothing carries it: the taxonomy is a commerce tree, there is no properties slot for an education standard, and the only writable surface is thirteen twenty-character tags that are already contested by grade and subject.

In both cases the leaf ids stay in the canonical product and the projection emits a loss record naming the axis, exactly as `TermsOutcome.loss` and the `NoTargetField` variant already do for grades.
The seller sees "these 91 standards are not carried to this marketplace" in the field diff before publish, and the data is recoverable the day a target grows a field, because nothing was deleted.
Spending tag slots on standard codes is available as a seller election and must never be a default, because it trades discovery weight on the target for a claim buyers there cannot filter by.

### The crosswalk each target needs

Tes needs the two we already hold: grade to US `yearGroups` as a fifteen-row exact table, and grade to GB `ageRanges` by joining on `humanAges` and taking the narrowest covering band.

| Target | Audience crosswalk needed | Subject crosswalk needed | Standards crosswalk needed |
|---|---|---|---|
| Etsy | none: no audience axis exists, so grade becomes tag text or nothing | none: pick one commerce `taxonomy_id` per resource type, not per subject | none |
| Classful | 20 TPT grade facets to 16 values, near one-to-one; Homeschool, Staff and Not Grade Specific have no counterpart | 140 TPT subject facets to ~18 roots and their leaves | 166 TPT jurisdictions to 6 framework names, coarse only |
| Boom | 20 TPT grade facets to preschool-through-university, plus Not Grade Specific | 140 TPT subject facets to Boom subjects and subtopics, with credential gating on several | none: prose only |
| Teach Simple | grade as a tag value | subject as a tag value | unmeasured |
| Made By Teachers, Amped Up Learning | unmeasured | unmeasured | unmeasured |
| Storefronts | none: encode as tags or metafields, a per-seller naming decision | none | none |

Two of these are genuinely cheap, and it is worth saying which and why.
Classful's grade list is a near-copy of TPT's, so that crosswalk is a table with three explicit no-counterpart rows and no judgement in it.
Classful's standards facet is six names against TPT's 166 jurisdictions, so the mapping is a many-to-one fold onto four reachable targets and two unreachable ones, which is a decision procedure rather than a table.
The subject crosswalks are the expensive ones everywhere, for the reason `vocab-equivalence.md` already gives: there is no shared identifier and no shared naming convention, so root level comes first and topic-level coverage stays partial for a long time.

## The mapping engine for N platforms

The engine that exists is already N-platform in shape; what changes under the TPT-base decision is which vocabulary the canonical terms are minted from, and what has to be added is per-seller override, drift detection and coverage of the axes not yet routed.
Nothing below asks for a new architecture.

### The canonical model as a TPT superset, and the one inversion it forces

Today the canonical grade vocabulary is minted from Tes `yearGroups` because that was the richest set on file — thirty rows carrying their own ages — and TPT's nineteen options pair against it through one authored fifteen-row table (`crates/tam-taxonomy/src/grades.rs`).
Under a TPT-base product model that direction inverts: canonical terms are minted from TPT's facets, and Tes becomes a projection target like every other.

This is a re-seed rather than a rewrite, and the reason is worth stating because it is the property that makes the whole design cheap.
`project`, `project_terms`, `project_axis` and `ingest` decide on the edge set alone, with no fallback and no nearest-neighbour search, so which side the terms were minted from is a fact about the seeded rows in `projection_edge` and `canonical_term`, not about the functions.
The authored artefact that survives inversion is the fifteen-row grade table and the GB age-band covering rule; both are already expressed as derivations over the captured JSONs rather than as transcriptions, so they re-derive in the other direction against the same captures.

The superset claim also has to be honest about where TPT is not a superset.
Tes requires a licence and TPT has no licence field anywhere on its wire, which is why `AxisAbsent` exists and why licence is the legal exemplar.
Etsy requires `who_made`, `when_made` and `quantity`, none of which TPT holds.
Boom requires a deck.
So the canonical model is TPT plus a per-target set of natives the canonical product cannot supply, and each of those is either a seller election or a target-specific default that the registry names rather than the adapter invents.

### Projection as a lossy function with residue and provenance

The outbound function already returns the four buckets a publish gate needs: `TermsOutcome` carries `included` for what the listing gets, `loss` for the broadening the seller sees in the field diff, `blocked` for what raises reconciliation items, and `omitted` for terms a `NoCounterpart` record says to drop.
`project_axis` adds the axis-level facts a term never carries — the target's cardinality, its requiredness, and the product the question is about — and it never truncates: a cap overflow is taken whole into an election so that no partially-narrowed set exists for a caller to publish by accident.
The inbound direction, `ingest`, is deliberately the reverse of `Exact` edges only, because inverting a `Broader` edge would restore a distinction the edge dropped.

Residue is the seeder's report rather than a runtime concept, and it generalises directly.
The GB-to-NZ derivation already emits `Residue { gb_only, nz_only, mismatched }` with a typed `MismatchReason`, which is exactly the shape a "TPT facets with no Classful counterpart" report needs, and a new target adds a third residue bucket rather than a new mechanism.

Provenance is the one piece that is currently TPT-shaped and needs generalising.
`check_native_ids` refuses an edge whose target identifier is not the shape its target vocabulary issues, and it only knows TPT's slug namespace, because Tes addresses everything by number and Etsy binds nothing.
Each new target with a recognisable identifier shape adds a rule there; a target whose ids are opaque numbers adds nothing and is protected only by the reverse-uniqueness index.
That asymmetry is worth keeping visible: the guard is strongest exactly where the identifiers are most legible, which is the opposite of where the risk is.

### Resolution modes, with licence as the worked exemplar

The founder's three modes are already data in the registry rather than branches in code.
`Delegation::ByOptIn` is seller-decides by default with best-fit available on explicit opt-in; `Delegation::Never(NonDelegable::LegalContent)` is seller-decides always; and a clean one-to-one resolves silently because it is an `Exact` edge and no election is raised.

Licence works end to end today.
TPT declares no licence axis anywhere, recorded as an `AxisAbsent` entry rather than as silence, so a licence projected into TPT is a disclosed loss and not a block.
Tes declares licence required, so a product whose rights are unstated raises a `Supply` election rather than being published under a grant nobody chose, and the seven Tes licence tokens exist as canonical terms precisely so the election has candidates to offer.
The price gate narrows those candidates — a free resource picks among the three CC values, a paid one takes TES-PAID or the school tier — which is the shape every other conditional vocabulary should take: read the branch off the product, offer only what the target will accept, and never default.

Etsy adds the second member of the non-delegable class, and this is a finding rather than an inference.
`who_made` is required on every create, takes `i_did`, `someone_else` or `collective`, and is an attestation about authorship, not a fact derivable from any TPT field — TPT's `copyright_declaration` distinguishes original work from work using copyrighted material, which is a different statement.
The registry's own comment says a second non-delegable field must state which bar it clears; `who_made` clears the same one licence does, and `when_made` rides along with it because Etsy requires the pair.

### Where per-seller overrides live

They cannot live in `projection_edge`, and the reason is written into the code: that table carries no organisation, its rows are global and permanent, and its uniqueness indexes refuse a corrected row.
A seller who wants "my TPT Math tag always becomes Tes Mathematics / Number, not Mathematics / Algebra" is not asserting a global equivalence and must not be able to write one.

The shape that fits is a per-organisation override slice consulted before the global relation, keyed by organisation, inventory and axis, and carried into `ListingContext` alongside `edges`, `no_counterparts`, `rules` and `settled`.
`project_axis` then prefers an override over an edge and records which won, so the field diff can say "you set this" rather than "the relation says this", and a later re-poll that changes the global relation does not silently change a seller's published listings.
This is the Vendoo per-marketplace-template idea reduced to its load-bearing part: the template is not a second mapping engine, it is a per-seller layer over the same relation, and it inherits the same loss reporting.

A per-seller layer also has one target where it is not optional.
`shop_section_id` on Etsy, store folders on Boom and collections on Shopify are all per-seller numeric identifiers that must exist on the target before they can be used, so TPT's custom categories map through a per-seller table that is populated by reading the target, never by a global crosswalk.

### Adding a new target

Four steps, of which one is code.

Capture the target's vocabularies as JSON committed under `docs/design/data/`, the way the Tes and TPT captures already are, recording the source, the date and what was not found.
Add an `InventoryId` and an `InventoryRegistry` declaring the canonical six's caps and requiredness, the natives that exist only on that wire, the equivalence axes with their cardinality and delegation, and — this is the part that is easy to skip — an explicit `AxisAbsent` for every axis measured to be absent, because an axis merely missing from the registry means unmeasured and blocks.
Derive the edges from canonical TPT terms onto the new vocabulary, run `check_native_ids` over the whole derivation before any row becomes durable, and keep the residue report.
Write the adapter.

Only the adapter is code, and it is most of the cost, which is why the effort table below splits on transport rather than on vocabulary size.

### Detecting and re-ingesting vocabulary drift

Drift is a diff against a committed capture, and the mechanism the GB-to-NZ derivation already uses generalises without change.

Re-poll each target's vocabulary on a schedule and diff by native identifier.
A facet TPT has added appears as a canonical term with no outbound edges, so every target reports it as a gap and it enters the reconciliation queue as a question for an operator rather than silently projecting to nothing.
A target value that has disappeared appears as an edge pointing at an identifier the new capture does not hold, which is a `MismatchReason` in the seed report; the safe response is to stop using the edge and raise an item, never to guess a replacement.
A relabelled value with a stable identifier is a label refresh and touches nothing structural, which is why `native_label` reads labels out of the captures rather than storing them beside the terms.

The one drift class the diff cannot see is a change in cardinality or requiredness — a target that starts refusing a second subject, or starts demanding a field that used to be optional — because those are properties of the form rather than of the vocabulary.
Those surface as adapter errors on a live write, which argues for treating the first failed create after a re-poll as a registry question rather than a retry.

### Multi-select onto single-select

This is the case with the most instances and the least new machinery.

A TPT product carries up to four grade facets, up to three subject-area facets, up to six tags and up to three format facets.
Tes takes exactly one `mainType` and exactly one `primaryCategory`; Etsy takes exactly one `taxonomy_id`; Boom takes one price but many grades and subjects.
`Cardinality::One` against a resolved set of several is already `project_axis`'s cap-overflow path, and its behaviour is the right one: the whole resolved set goes into an election, and no narrowed set is published by accident.

The election's candidate list is the resolved set, so the seller picks one of their own values rather than choosing from the target's whole vocabulary, which is a much smaller question.
Best-fit, where the seller has opted into it, is a ranking over that same set and never over the target's vocabulary — that constraint is what keeps a suggestion from becoming an invention.
Deriving the Tes `primaryCategory` from the first TPT tag by sort order is a defensible default and is still a decision rather than a translation, so it belongs behind the opt-in and not in the projection.

## Effort

Assumptions: one engineer; the M6 connector contract and the reconciliation and election queues stay as they are; vocabulary capture means the target's own published or browsable option sets, not a hand-authored topic-level crosswalk; each adapter gets a live-fire budget against the founder's own seller account; no new legal review is inside these numbers.

Extending the current two-platform engine to the design above:

| Work | Days | Note |
|---|---|---|
| Re-base the canonical vocabulary on TPT and re-derive the Tes crosswalks in the inverted direction | 3-5 | Re-seed plus test churn; the derivations already read the captures |
| Per-seller override layer: table, `ListingContext` slice, preference in `project_axis`, provenance, API surface | 4-6 | The one genuinely new concept |
| Route the axes not yet bound: resource type, tag, format, standards, custom category, status | 5-8 | Mostly seeding and registry entries; resource type needs its 71-to-9 table |
| Best-fit suggestion layer over the election queue, strictly ranked over the resolved set | 3-5 | Opt-in only |
| Vocabulary drift job: scheduled re-poll, diff, residue to the reconciliation queue | 3-4 | Generalises the existing seed report |
| Total | 18-28 | Four to six weeks |

Per new target, on top of that:

| Target class | Vocabulary capture | Adapter | Elapsed not in the estimate |
|---|---|---|---|
| API target, Etsy | 1-2 days: `getSellerTaxonomyNodes` plus the spec | 8-12 days: OAuth with PKCE, refresh rotation, the four-call create sequence, error taxonomy, rate-limit budget | App approval: minutes for a Seller App, weeks for Commercial Access |
| API target, Shopify | 1 day | 6-10 days, plus a digital-delivery app dependency | App listing if we distribute it |
| No-API marketplace: Classful, Made By Teachers, Amped Up Learning, Teacha! | 1-2 days from browse facets; more if the create form is only visible to a seller | 10-18 days each: form scrape, session handling, upload chain, publish, read-back | Live-fire capture per platform |
| No-create-API storefronts: Gumroad, Payhip, Sellfy, Lemon Squeezy | under a day; there is almost no vocabulary | same class as a no-API marketplace, or out of scope | — |
| Teach Simple | 1-2 days | 10-18 days plus a dependency on listing-copy generation, because a copied description is non-compliant | QA queue latency per product |
| Boom | not applicable | not a projection target; the product must be authored | — |

The subject crosswalk is deliberately outside those figures because it is authoring rather than engineering.
Root level against a new target is one to two days of careful work; topic level is open-ended and should be allowed to stay partial, with the reconciliation queue carrying the remainder.

The single largest cost driver is not vocabulary size and not the number of targets.
It is that seven of the nine platforms have no API, so the adapter is a form-driven write path, and the M7 TPT connector is the honest reference for what that costs.

## Server-side, client-side, and why the mapping does not care

Two targets can be served server-side on a credential the platform itself issued for the purpose.
Etsy's OAuth token is granted by the seller through Etsy's own consent screen, scoped to `listings_w`, and refreshable for ninety days; that is a sanctioned delegation, and a server holding it is doing what the token was minted for.
Shopify is the same story with `write_products`.

Every other target has no API, so any automation drives the seller's own authenticated session against a form.
That is precisely the pattern `marketplace-terms-assessment.md` identifies as the exposed one: server-side execution concedes the access prong that Perplexity won on and leaves only the authorization question, which turns on a cease-and-desist.
So Tes, Classful, Made By Teachers, Teach Simple, Amped Up Learning, Teacha!, TeachBuySell, Gumroad, Payhip, Sellfy and Lemon Squeezy all sit on the client side of that line, and Boom sits outside it because there is nothing to automate that is not authoring.

The mapping engine is indifferent to that split, and the indifference is structural rather than a convenient claim.
`project_listing` performs no I/O; `ListingContext` carries an organisation, a mapping, an inventory, an instant and four relation slices, and no credential; the function's output is a rendering plus a loss report plus, where blocked, the named gate.
Where that value is spent — a server exchanging it for an Etsy `createDraftListing` call, or a client-side agent filling a Tes uploader under the seller's own session — is entirely the adapter's business, on the far side of a boundary the projection never crosses.

One small addition makes the split legible instead of tribal.
The registry should record a sanctioned-transport class per inventory — official token, seller session, or neither — beside the fields it already records.
Then the product can tell a seller which of their marketplaces run unattended from our infrastructure and which need the client running, the legal posture becomes data that a test can assert over, and a future target arrives with its transport class declared rather than assumed.

## Open questions for the founder

1. Re-base the canonical vocabulary on TPT, inverting the current mint from Tes `yearGroups`? Recommended yes; it is a re-seed and it makes the base-product decision true in the data rather than only in the prose.
2. Which three targets ship first? Recommended Tes, Etsy and Classful — Tes is built, Etsy is the only sanctioned API, and Classful has the closest vocabulary to TPT's and therefore the cheapest crosswalk.
3. What happens to a free TPT product projected onto Etsy, where a free price is impossible? Recommended a named publish gate offering a minimum-price election, never a silent floor.
4. Etsy app tier: one Commercial Access app that we hold, or a Seller App registered by each seller? Recommended Commercial Access as the target, with per-seller Seller Apps as the bootstrap that does not wait on review.
5. Should the price projection enforce TPT's most-favoured-pricing rule, so no target is ever cheaper than TPT? Recommended yes, as a gate rather than a warning; it converts a contractual liability into a product feature.
6. Is Boom Learning excluded from the mapping engine, or scoped separately as authoring? Recommended excluded, and revisited only as a conversion feature.
7. Is Teach Simple in scope given it has no price and forbids copied descriptions? Recommended deferred until listing-copy generation is a dependency we are happy to take.
8. Do per-seller overrides ship with the first multi-target release or after? Recommended after, except for the per-seller shelf mapping, which is not optional on Etsy, Boom or Shopify.
9. Should the registry carry a sanctioned-transport class per inventory? Recommended yes; it is small and it makes the architecture fork auditable.

## Unverified

- Etsy's seller-facing limits — 140-character title, 13 tags of 20 characters, 5 digital files of 20 MB each, 70-character filenames, description without HTML — come from secondary write-ups quoting the Etsy help centre; `help.etsy.com` returns 403 to this network on every path and to both retrieval methods.
- Etsy's per-listing fee and four-month listing expiry were not retrieved; the API's `should_auto_renew` field corroborates that expiry exists, but no figure here is sourced.
- Etsy's Personal App shop limit and the reported lower starting rate tier for new apps come from community forum posts, not from Etsy documentation.
- The Etsy seller taxonomy's size, and whether any node is a reasonable home for a teaching resource, need an API key to check; `getSellerTaxonomyNodes` requires one.
- Every Made By Teachers fact — the 90 MB and ~200 MB file caps, the one-preview-image minimum, the Add a New Product flow, PayPal payouts — comes from search snippets quoting its own seller FAQ; the site returns 403 to this network on every path.
- Classful's seller create form, its per-field caps, requiredness, tag cap and licence options are unmeasured; only the public browse facets were captured.
- Amped Up Learning running on BigCommerce is inferred from its asset host `cdn11.bigcommerce.com`, not stated by the site; its listing fields are entirely unmeasured.
- Boom Learning's minimum price, and whether decks can be created programmatically or only in the Studio, were not established.
- Teach Simple's upload-form fields, and the form in which standards are recorded, were not retrieved; the onboarding article names standards only as an input to the share weighting.
- Teacha!'s per-field constraints beyond file type and the 60 MB cap were not retrieved.
- Whether TeachBuySell's TPT import is contractually cleared with TPT is unknown; the help page makes no statement about it, and the import reads TPT store pages programmatically, which is the one clause TPT's guidelines actually have.
- No traffic or market-share data was retrieved for any target, so "meaningful traffic" is not evidenced here for the two platforms added during this research.
- Tes's title length cap, if one exists, is unmeasured; 35 to 45 characters is uploader guidance rather than a validation bound.
