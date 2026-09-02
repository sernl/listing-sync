# The TPT product model

Field-by-field model of a TPT product as the seller's create and edit forms expose it, where each field's allowed values come from, and what remains unknown.
Research date 2026-09-02.
Evidence is the founder's own captures on local disk, this repository's prior wire recon, and public TPT pages; no authenticated request was made in producing this document.

## Executive summary

1. TPT's create form is one CakePHP form posting `multipart/form-data` to `/My-Products/New/Digital-Next`, and the edit form posts the same field set to `/itemsDigital/editNext/{id}`; the two bodies are near-identical, so create and edit are one model, not two.
2. The form's own `data[_Token][unlocked]` hidden input enumerates 48 client-writable field paths, which is a server-authored field inventory and the most authoritative field list we hold.
3. Four visually distinct pickers — Grade Level, Subject Area, Tag, Format — all serialise into a single flat `data[TaxonomyTags][]` array of slugs, discriminated only by each slug's own `category` metadata; there is no separate subject field and no separate grade field on the wire.
4. Product type is chosen on a type-selection page before the form renders, not in a dropdown on it: `itemType` is one of `DIGITAL_PRODUCT`, `BUNDLE`, `ONLINE_RESOURCE`, `EASEL`, `VIDEO`, and it is read-only thereafter.
5. Cardinality caps are stated on the form and were absent from `docs/design/data/tpt-vocabulary.json`: up to four grades, three subject areas, six tags, three formats, four thumbnails.
6. Five tax codes exist, they are Avalara-style digital-goods product tax codes by their code shape, the field posts a row id rather than the code string, and the code is required for paid resources only.
7. Multiple licences is a price field, not a percentage: the form pre-fills 90 percent of price and the seller may overwrite it, so treating 90 percent as an invariant is wrong.
8. Education standards are a four-level tree (subject, domain, cluster, standard) behind two GraphQL operations, the write field takes numeric node ids, and the captured product carried 91 of them; 166 jurisdictions exist, of which the form surfaces four.
9. Draft versus live is a field (`status_user` 0 or 1), not a route, so publishing is an ordinary edit.
10. The two copyright checkboxes' exact wording is the largest single gap: the form is client-rendered and the capture holds no JavaScript response bodies, so the assertions are unquoted.

## Sources

The evidence base, with what each contributes.

`~/downloads/tpt-capture-options.har`, 29 MB, 646 entries, Firefox 153.0.1, captured 2026-08-30 to 2026-08-31 by the founder against their own seller session (seller id 21268787, store EBMCResources).
It contains a complete create and a complete edit of product 17532511, both multipart bodies untruncated, plus 107 calls to `/graph/graphql` and 9 to `/gateway/graphql`.

`~/downloads/tpt-capture-options-2.png` is not an image.
It is a second HAR file, 16 MB, 344 entries, mis-saved with a `.png` extension; `file` reports `ASCII text, with very long lines`.
Its entry set is a strict subset of the first capture — the create half only, with no edit — so it contributes nothing the larger capture does not.
This is worth correcting at the source, because a reader trusting the extension would try to open it as a screenshot and conclude the capture was corrupt.

`~/downloads/tpt-capture-options.png` is a genuine PNG, 730x1115, showing the create form from the Price section through the Education Standards section.
It is the only source we hold for the form's own labels, helper text and cardinality caps, because the form is client-rendered and no JavaScript response body was captured.

`docs/design/data/tpt-vocabulary.json` in this repository, polled live 2026-08-29 against the founder's session, carrying 358 taxonomy facets and every fixed option set.

`crates/tam-marketplace-tpt/src/{write_model,form,flows}.rs`, our adapter's current field coverage.

`/home/sernl/.claude/projects/-home-sernl-projects-edtech-workspace-listing-sync/memory/research/tptw-create-form.md`, prior recon on the create write's token and upload mechanics, from a separate 2026-08-28 capture.

TPT Help Centre articles, fetched 2026-09-02 through the Zendesk public content API at `help.teacherspayteachers.com/api/v2/help_center/...`.
The rendered `hc/en-us/articles/...` pages return HTTP 403 to a non-browser client; the JSON API for the same article ids returns the same content and is the citable route.

## The field catalogue

TPT label is the string the form shows, taken from the screenshot where visible and from the founder's enumeration otherwise.
Wire field is the multipart part name from the captured create body.
Our field is the adapter's canonical `FieldKey` where one exists; our canonical model currently has only six keys — `Title`, `Description`, `Price`, `Taxonomy`, `Grades`, `Files` (`crates/tam-types/src/lib.rs:492`) — so most TPT fields have no canonical counterpart and are marked as adapter-only or absent.

| TPT label | Wire field | Our field | Type | Cardinality | Required | Conditional rules | Allowed values | Source |
|---|---|---|---|---|---|---|---|---|
| Title | `data[Item][name]` | `FieldKey::Title` | plain text | single | yes | screened for trademark infringement by `CheckResourceTitle` before submit | free text; 80 chars is the client constant and the longest of 352 observed titles, counting unit unverified | HAR create body; `tpt-vocabulary.json` `constraints.titleMaxLength` |
| Description | `data[Item][description]` | `FieldKey::Description` | HTML | single | yes (empty not observed) | posted with a trailing `<!-- New description editor -->` marker | free HTML; `description_max_length` 45000 | HAR create body; `var cfg` bootstrap |
| File to Upload | `data[ItemDigital][product]` + `data[ItemsProperty][product_uploaded]` | `FieldKey::Files` | opaque handle + flag | single | yes | handle produced by the S3 then `/uploads/process_file` then `/queue/results` chain; never the file itself | 576-char opaque base64; 4 GiB cap, 49 extensions | HAR; `var cfg` `upload_digital` |
| Product Preview | `data[ItemDigital][preview]` + `data[ItemsProperty][preview_uploaded]` | absent | opaque handle + flag | single | no | same upload chain; empty in both captures | 30 MiB cap, same 49 extensions | `var cfg` `upload_digital` |
| Video preview | `data[Upload][videopreview]` + `data[Upload][custom_videopreview_uploaded]` | adapter-only | opaque handle + flag | single | no | gated by A/B flag `VideoPreviewForDigitalProduct` (`on` for this seller) | 1 GiB cap, 11 extensions, 8 MIME types | `var cfg` `upload`, `analytics_ab_tests` |
| Auto Generated Thumbnail | `data[Item][generate_thumbnail]`, `thumbs`, `thumbs_collection_key` | adapter-only | integer + handle | single | yes | create posted `1` with a collection key and `thumbs=0`; edit posted `3` with neither | `1` and `3` observed; the enum is not enumerated anywhere we hold | HAR create and edit bodies |
| Thumbnail images | `data[ItemDigital][thumb1..thumb4]` + `data[ItemsProperty][thumb1..4_uploaded]` | adapter-only | opaque handle + flag | 4 fixed slots | no | edit populated slot 1 only | 4 MiB each; `bmp, gif, ipeg, jpg, jpeg, png, tiff, tif` — `ipeg` is TPT's own typo, present verbatim | `var cfg`; help article 360042865851 |
| Free Resource | `data[Item][free]` | part of `FieldKey::Price` | boolean | single | no | when set, price is not collected and no tax code is needed | checkbox | screenshot; help article 47530264334868 |
| Price | `data[Item][price]` | `FieldKey::Price` | decimal, 2 dp | single | yes when not free | floor `min_price` 0.95 | bare decimal, no currency on the wire | screenshot; `var cfg` `min_price` |
| Multiple Licenses | `data[Item][license_price]` | adapter-only | decimal, 2 dp | single | yes when priced | form pre-fills 90 percent of price; seller may overwrite | any amount; no floor or cap documented | screenshot; help article 360042885411 |
| Bundle Discount Price | `data[Item][discountprice]` | absent | decimal, 2 dp | single | no | posted `0` in both captures; meaning on a non-bundle listing is unverified | — | screenshot; HAR |
| (unnamed) | `data[Item][discount]` | absent | unknown | — | — | present in `unlocked`, never posted by the browser | — | `data[_Token][unlocked]` |
| Tax Code | `data[ItemTaxCode][tax_code_id]` | adapter-only | integer row id | single | yes when priced | not needed for free resources; a free resource made paid must gain one | 5 values, enumerated below | screenshot; `TaxCodesQuery`; help article 47530264334868 |
| Grade Level | `data[TaxonomyTags][]` | `FieldKey::Grades` | slug | up to 4 | yes | "Not Grade Specific" is the all-grades escape | 17 checkboxes on the form; 20 `Grade-Level` facets in the vocabulary | screenshot; `tpt-vocabulary.json` |
| Subject Area | `data[TaxonomyTags][]` | `FieldKey::Taxonomy` | slug | up to 3 stated | yes | see the cap discrepancy below | 140 `PreK-12-Subject-Area` facets, 7 hidden | screenshot; `tpt-vocabulary.json` |
| Tag (Theme, Audience, Language) | `data[TaxonomyTags][]` | `FieldKey::Taxonomy` | slug | up to 6 | yes | controlled vocabulary, not free text | 52 facets across `theme` 39, `audience` 10, `language` 3; 6 audience hidden | screenshot; `tpt-vocabulary.json` |
| Format | `data[TaxonomyTags][]` | `FieldKey::Taxonomy` | slug | up to 3 | no | — | 26 `Format` facets | screenshot; `tpt-vocabulary.json` |
| Custom Category | `data[Category][Category][]` | absent | integer id | multi, cap unknown | no | seller-scoped, not a platform vocabulary | the seller's own categories | screenshot; `MyProductListingsCustomCategories` |
| Common Core State Standards | `data[ItemsCommonCoreStandard][common_core_standard_id][]` | absent | integer node id | multi | no | buyers may opt into state translation | leaves under jurisdiction 3054 | screenshot; `EducationStandardsQuery` |
| Next Generation Science Standards | same field | absent | integer node id | multi | no | — | leaves under 3055 | screenshot |
| Texas Essential Knowledge and Skills | same field | absent | integer node id | multi | no | shown only to buyers in Texas | leaves under 3326 | screenshot |
| Virginia Standards of Learning | same field | absent | integer node id | multi | no | shown only to buyers in Virginia | leaves under 5785 | screenshot |
| (standards count) | `data[ItemsCommonCoreStandard][common_core_standards_num]` | adapter-only | integer | single | yes when standards posted | equals the number of id parts; `91` in both captures | derived | HAR |
| Appropriate for New Zealand | `data[ItemsLocalization][country_id_flag]` | adapter-only | boolean | single | no | label is seller-country-derived; this seller is NZ, `countryId` 153 | `0` or `1` | screenshot; `UploadPageProductQuery` response |
| Teaching duration | `data[ItemsProperty][duration]` | absent | integer id | single | no | — | 23 values, `NA` through `OTHER` | `tpt-vocabulary.json` |
| Number of Pages or Slides | `data[ItemsProperty][pages]` | absent | integer | single | no | read back as `filePreview.pageCount` | positive integer | HAR; `UploadPageProductQuery` |
| Answer Key | `data[ItemsProperty][answer_key]` | absent | integer id | single | no | — | 6 values | `tpt-vocabulary.json` |
| Copyright, box 1 and box 2 | `data[ItemsProperty][copyright_declaration]` | `AuthorshipDeclaration` | integer id | single | yes | a legal attestation; our adapter refuses to synthesise one | `1` ORIGINAL_WORK, `2` USED_COPYRIGHTED_MATERIALS | `tpt-vocabulary.json`; `write_model.rs` |
| Making Listing Active | `data[Item][status_user]` | adapter-only | integer | single | yes | `0` draft, `1` live; publishing is an edit, not a route | `0` or `1` | HAR create posted `0` |
| Primary Audience | `data[ItemsProperty][audience]` (+ `audienceText`) | absent | integer id | single | no | `OTHER` pairs with free text; neither capture posted it | 8 values | `tpt-vocabulary.json` |
| Video Type | `videoType` / `data[ItemsVideoProperty][video_type_text]` | absent | integer id (+ free text) | single | video products only | `OTHER` pairs with free text | 8 values | `tpt-vocabulary.json`; `unlocked` |
| Bundle title | `data[ItemsBundlesProperty][title]` | absent | text | single | bundle products only | present in `unlocked`, never posted by a digital create | free text | `data[_Token][unlocked]` |
| Revision comment | `data[RevisedItem][comment]` | absent | text | single | no | present in `unlocked`, never posted | free text | `data[_Token][unlocked]` |
| (revision marker) | `data[RevisedItem][is_post]` | adapter-only | flag | single | yes | posted empty in both captures | empty | HAR |
| (product type) | not on the wire | absent | enum | single | chosen before the form | read-only after creation | `DIGITAL_PRODUCT`, `BUNDLE`, `ONLINE_RESOURCE`, `EASEL`, `VIDEO` | `tpt-vocabulary.json`; `UploadPageProductQuery` |

Security and session parts, which are not product fields but must be replayed for a submit to be accepted: `_method`, `data[_Token][key]`, `data[_Token][fields]`, `data[_Token][unlocked]`, `data[_Csrf][csrfKey]`, `data[_Csrf][csrfToken]`.

### The server's own field inventory

`data[_Token][unlocked]` is a URL-encoded, pipe-separated list of every field path CakePHP's `SecurityComponent` will accept from the client.
Decoded from the captured create body it is exactly 48 paths.

```
Category, Category.Category, Item.description, Item.discount, Item.discountprice,
Item.error, Item.free, Item.generate_thumbnail, Item.generate_thumbnail.error,
Item.generate_thumbnail.size, Item.generate_thumbnail.tmp_name,
Item.generate_thumbnail.type, Item.license_price, Item.name, Item.price, Item.size,
Item.status_user, Item.tmp_name, Item.type, ItemDigital.preview, ItemDigital.product,
ItemDigital.thumb1, ItemDigital.thumb2, ItemDigital.thumb3, ItemDigital.thumb4,
ItemTaxCode.tax_code_id, ItemsBundlesProperty.title,
ItemsCommonCoreStandard.common_core_standard_id,
ItemsCommonCoreStandard.common_core_standards_num, ItemsLocalization.country_id_flag,
ItemsProperty.answer_key, ItemsProperty.copyright_declaration, ItemsProperty.duration,
ItemsProperty.pages, ItemsProperty.preview_uploaded, ItemsProperty.product_uploaded,
ItemsProperty.thumb1_uploaded, ItemsProperty.thumb2_uploaded,
ItemsProperty.thumb3_uploaded, ItemsProperty.thumb4_uploaded,
ItemsVideoProperty.video_type_text, RevisedItem.comment, RevisedItem.is_post,
TaxonomyTags, Upload.custom_videopreview_uploaded, Upload.videopreview, thumbs,
thumbs_collection_key
```

Three things follow.
The `Item.generate_thumbnail.{error,size,tmp_name,type}` quadruple is PHP's `$_FILES` shape, so `generate_thumbnail` can carry an uploaded file and not only the integer `1` or `3` the captures posted; a seller-supplied master thumbnail is the plausible reading and it is unverified.
`ItemsBundlesProperty.title` and `ItemsVideoProperty.video_type_text` are unlocked on a *digital* product's form, so the same form template serves every product type and the type-specific sections are shown or hidden client-side rather than being separate forms.
The list is a superset of what the browser posts — the browser omitted ten of the 48 — and per the prior recon in `tptw-create-form.md` the locked-field list after the colon in `data[_Token][fields]` is empty, so posting a subset is accepted provided the three token parts are replayed byte-for-byte.

### Create against edit

The two captured bodies differ in five places and nowhere else.
`data[ItemDigital][product]` carries the 576-char handle on create and is empty on edit, with `product_uploaded` staying `1` both times, so an edit that does not replace the file posts an empty handle rather than re-uploading.
`thumbs` and `thumbs_collection_key` carry values on create and are empty on edit, while `thumb1` and `thumb1_uploaded` are populated on edit only.
`data[Item][generate_thumbnail]` is `1` on create and `3` on edit.
`data[Item][price]` is `4` on create and `4.00` on edit, so the server accepts both scales.
Field order differs: on create the tax code sits between `status_user` and `Category`, on edit it sits after `answer_key`.
Since our adapter reproduces recorded wire order deliberately (`write_model.rs` module docs), that the two captures disagree on order is itself evidence that order is not load-bearing for acceptance.

## Controlled vocabularies

### Where each picker's values come from

| Picker | Source | Scope | Count | Enumerable offline |
|---|---|---|---|---|
| Grade Level | webpack chunk, one flat facet set | TPT-global | 17 on the form, 20 facets | yes, in `tpt-vocabulary.json` |
| Subject Area | same facet set, `category: PreK-12-Subject-Area` | TPT-global | 140, 7 hidden | yes |
| Tag | same facet set, categories `theme`, `audience`, `language` | TPT-global | 52, 6 hidden | yes |
| Format | same facet set, `category: Format` | TPT-global | 26 | yes |
| Resource type | same facet set, `category: Type-of-Resource` | TPT-global | 71, 1 hidden | yes, but no picker for it is visible on the current form |
| Custom Category | `MyProductListingsCustomCategories` GraphQL | seller-scoped | 7 for this seller | no, per-seller fetch |
| Tax Code | `TaxCodesQuery` GraphQL | TPT-global | 5 | yes |
| Education standards | `EducationStandardsJurisdictionsQuery` then `EducationStandardsQuery` | TPT-global | 166 jurisdictions, thousands of leaves | only by crawling |

No network request in either capture fetches the grade, subject, tag or format vocabularies.
They ship inside the `tpt-frontend` webpack bundle, which is why `tpt-vocabulary.json` cites `JSON.parse` blobs in chunks rather than an endpoint.
The HAR records the chunk requests but with `response.content.size` of 0 and no body, so the chunks cannot be re-mined from this capture; the 2026-08-29 poll that produced `tpt-vocabulary.json` remains the only extraction we hold.

Tags are a controlled vocabulary, not free text.
The picker is labelled `Tag (Theme, Audience, Language)` and its helper text is `Select up to six tags`, and every value the captured create posted resolves to a facet slug in the 358-facet set.

The four pickers are one wire field.
The captured create posted seven `data[TaxonomyTags][]` parts spanning four pickers: `not-grade-specific` (Grade-Level), `graphic-arts`, `other-arts`, `other-art` and `balanced-literacy` (PreK-12-Subject-Area), `staff-and-administrators` (audience), `image` (Format).
`UploadPageProductQuery` reads the same seven back as an unordered `taxonomyTags` array, so the round trip is symmetric and lossless but carries no picker attribution; reconstructing which picker a slug came from requires the facet's own `category`.

The subject-area cap does not match the capture.
The form says `Select up to three subject areas` and the create posted four `PreK-12-Subject-Area` slugs.
Either the cap is advisory and client-side, or the picker does not count all four as subject areas — `other-art` is a root facet while `other-arts` hangs off `art`, so a parent-plus-leaf expansion is one candidate explanation.
This is unresolved and is listed under gaps.

Hidden facets are retired values that still read back on existing products.
The vocabulary file's guidance is to accept them on read and never offer them on write, and it holds across the 16 hidden facets (7 subject, 6 audience, 2 supports, 1 resource type, plus `Featured` and all 6 `topic` facets).

### Grade level

The form renders 17 checkboxes: Preschool, Kindergarten, 1st through 12th Grade, Higher Education, Adult Education, Not Grade Specific.
Helper text: `Select up to four grades. If your product works for all grades, select "Not Grade Specific."`
The seller blog corroborates the four-grade guidance and attributes it to buyer-conversion testing.

The vocabulary file's 20-entry grade map is a different, legacy numeric encoding — ids 1 to 23 with `formLabel` values `PreK`, `K`, `1st` and so on — in which Homeschool (17) and Staff (19) resolve to `category: audience` rather than `Grade-Level`.
Those two are not grade checkboxes on the current form; `homeschool` and `staff-and-administrators` are audience facets reachable through the Tag picker.
A third, unrelated 18-value string encoding (`early_childhood, prek, k, 1..12, higher, adult, other`) drives buyer-facing filters.
Three encodings coexist and must not be crossed.

### Format, 26 values

`activboard-activities`, `activeinspire-flipchart`, `audio`, `boom-cards`, `canva`, `digital`, `easel`, `easel-activities`, `easel-assessments`, `ebook`, `fonts`, `google-apps`, `image`, `interactive-whiteboards`, `microsoft`, `microsoft-excel`, `microsoft-onedrive`, `microsoft-powerpoint`, `microsoft-publisher`, `microsoft-word`, `other-digital`, `pdf`, `prezi`, `seesaw`, `smart-notebook`, `video`.
Eleven are roots and the rest hang off `digital`, `easel`, `interactive-whiteboards` or `microsoft`, at a maximum depth of 2.

### Tag, 52 values

Theme, 39: the seasonal and observance calendar — `aapi-history-month`, `april-fools-day`, `arbor-day`, `autumn`, `back-to-school`, `black-history-month`, `christmas-chanukah-kwanzaa`, `cinco-de-mayo`, `day-of-the-dead`, `diwali`, `earth-day`, `easter`, `end-of-year`, `fathers-day`, `groundhog-day`, `halloween`, `hispanic-heritage-month`, `holiday`, `july-4-independence-day`, `juneteenth`, `labor-day`, `lunar-new-year`, `mardi-gras`, `martin-luther-king-day`, `memorial-day`, `mothers-day`, `new-year`, `passover`, `presidents-day`, `ramadan`, `seasonal`, `spring`, `st-patricks-day`, `summer`, `thanksgiving`, `valentines-day`, `veterans-day`, `winter`, `womens-history-month`.

Audience, 10 of which 4 are writable: `homeschool`, `parents`, `staff-and-administrators`, `tpt-sellers`; hidden are `by-tpt-sellers-for-tpt-sellers`, `for-administrators`, `gate`, `gifted-and-talented`, `products-for-tpt-sellers`, `staff`.

Language, 3: `en-espanol`, `en-francais`, `english-uk`.
The first two carry mojibake in the vocabulary file (`En espaxf1ol`, `En franxe7ais`), which is an extraction artefact of the 2026-08-29 poll rather than TPT's own encoding, and should be repaired to `En español` and `En français` before the file is used for display.

The tag cap rose from three to six in TPT's March 2025 tagging overhaul, and the same overhaul added a "Not subject specific" subject tag so the required subject field can be satisfied when no specific subject fits.

### Custom categories

Seller-scoped shelves, not a platform vocabulary; each seller defines their own and assigns products to them.
The wire field is `data[Category][Category][]` carrying integer ids, read back as `categories` with `{id, name}` and on the gateway as `customCategories` with the ids stringified.

Two operations enumerate them.
`MyProductListingsCustomCategories($sellerId: ID)` returns the caller's own with a `resourceCount` per category.
`GetCategoryList($id: ID!)` returns any seller's by id, without counts.
The founder's store returned seven: `1368991` Level 2: 5th/6th Grade (0), `1368990` Level 1: 4th/5th Grade (101), `1368989` Complete Topic (101), `1359903` Full Units (25), `1361942` Unit PowerPoints (24), `1361943` Math Homework Books (4), `1361944` Mental Math (1).

TPT's help centre describes a custom category as a way to organise resources in a store, assignable from the Edit Product page and from Quick Edit, with an optional buyer-facing description on the category itself.
No per-product or per-store cap is documented and none was observed.

## Tax codes

Five codes, from `TaxCodesQuery { taxData { id name taxCode } }`.

| id | taxCode | name |
|---|---|---|
| 1 | DA051011 | Digital audio works sold to an end user with rights for permanent use |
| 2 | DB031013 | Digital books sold to an end user with rights for permanent use |
| 3 | DI010200 | Digital Images - Streaming / Electronic Download |
| 4 | DV010200 | Videos - Streaming / Electronic Download |
| 5 | DO010000 | Other Digital Goods - No Physical Media |

The wire field carries the row `id`, not the `taxCode` string: the captured create and edit both posted `5`, and `UploadPageProductQuery` read back `taxCode: { id: "5" }`.
An earlier capture recorded a product posting `2` and reading back as `DB031013`, which fixes the id-to-code correspondence in both directions.

The codes are Avalara product tax codes by their shape.
`DA`, `DB`, `DC`, `DD`, `DI`, `DM`, `DV` and `DO` are Avalara's digital-goods prefixes and `DO010000` is Avalara's catch-all for other digital goods, and the phrase `sold to an end user with rights for permanent use` is Avalara's own wording distinguishing permanent from subscription digital rights.
No TPT source we hold names the provider.
Marking this as strong inference from code shape and description wording rather than as a verified fact: TPT's help centre and terms name neither Avalara nor Vertex.

Which code applies to what, from the descriptions rather than from TPT guidance, because TPT gives none.
A PDF worksheet is a digital book (`DB031013`, id 2) if read as a book, or other digital goods (`DO010000`, id 5) if not; TPT's own default for the founder's test product was id 5.
A video product is `DV010200`, id 4.
A bundle has no code of its own in the list, so it takes whichever of the five its contents most resemble, and `DO010000` is the safe general answer.
Clip art and image packs are `DI010200`, id 3.
Audio is `DA051011`, id 1.

Operative statements from help article 47530264334868, `What are Tax Codes and how do I apply them to my resources?`, fetched 2026-09-02.
Only paid resources need tax codes, since "free resources are not taxed."
Sellers edit codes individually on the Upload/Edit page under a Price section field called Tax Code.
Resources uploaded before 18 December 2018 carry an automatically assigned code the seller may edit.
If a free item becomes paid, a tax code selection is required.
Bulk editing runs from My Product Listings then Edit Tax Codes, selecting "up to 100 products" at a time.

TPT's Terms of Service place the obligation on the seller: "You acknowledge and agree that you are responsible for designating the appropriate tax codes for your Resources."
This matters for us directly.
A cross-listing tool that defaults a tax code on the seller's behalf is making a tax determination the seller is contractually answerable for, and the same reasoning our adapter already applies to `copyright_declaration` — refusing to synthesise an attestation nobody made — applies here.

The form marks Tax Code `Required` with helper text `Completion of this field is required in order for sales tax to be collected on this product.` and a `View Full Code Descriptions` link, and the captured render shows the validation message `Please select a tax code.`

## Multiple licences, bundles, Easel and video

### Multiple licences

The form field is labelled `Multiple Licenses`, marked `Required`, and rendered as a dollar amount with a `$` prefix, not a percentage.
The client bootstrap carries `multiple_license_price_percentage: 90`, and the captured pair is price `4` against `license_price` `3.60`, which is exactly 90 percent.

The 90 percent is a pre-fill, not a rule.
Help article 360042885411 states "Although the default discount is 10% off, you can choose whatever discount seems right to you," and sellers may set different discounts, or none, per resource, by editing the `Additional License Price` field.
No floor or cap is documented.

This is a correction to how our adapter reasons about the field.
`write_model.rs` derives `license_price` from price by a fixed 90 percent with a documented half-up rounding choice, which reproduces TPT's default but cannot represent a seller who has deliberately set a different additional-licence price.
Treating the derived value as the only possible one would silently overwrite that seller's pricing on every sync.
Whether to carry an explicit additional-licence price in the canonical model is a product decision, not an adapter detail.

A multiple licence is a pricing mechanism, not a rights grant.
TPT has no licence field and no Creative-Commons-style licence vocabulary at all, which is a real asymmetry against Tes and Etsy and is already recorded in `tpt-vocabulary.json` under `notFound.licence`.

### Bundles

`BUNDLE` is a distinct `itemType`, chosen on the type-selection page.
A bundle is composed of other resources rather than of uploaded files: help article 360042191412 states "A Bundle must contain at least 2 resources and can include up to 500 resources or bundles," and the 500 matches `bundle_max_allowed_files: 500` in the client bootstrap.
Bundles may nest, since the same sentence admits bundles as members.
TPT Bundles "automatically update when changes are made to the resources included in them," which distinguishes them from a traditional bundle assembled as a single zip.

The bundle form adds `data[ItemsBundlesProperty][title]`, which is unlocked even on the digital form.
No other bundle-specific field appears in the unlocked list, so the bundle's member list is very likely carried by a mechanism outside this form; no capture we hold includes a bundle create, so the member-list wire shape is unknown.

`Bundle Discount Price` maps to `data[Item][discountprice]` and was posted as `0` on a digital product in both captures.
Its semantics on a non-bundle listing are unverified.
Secondary seller-community sources describe a default bundle discount around 20 percent and a platform guardrail near 50 percent, but the authoritative help article on the price constraint (id 360042192372) returned HTTP 401 through the content API and its text was not obtained, so no discount bound is recorded here as fact.

### Easel

`EASEL` is a distinct `itemType` selected as its own option at Add New Product, per help article 7918722760724.
A standalone Easel listing pulls content in through `Select from Easel` rather than a file upload, may contain only Easel Activities or Assessments, and once an Activity is attached to a standalone listing it cannot be attached to any other listing.
Sellers needing PDFs or Google Slides alongside must choose a different product type.

Easel is also an attachment, not only a type.
`UploadPageGetAssetsQuery` shows `EaselAssessmentResourceAsset` and `DigitalActivityResourceAsset` as possible `primaryAssets` on `DigitalDownloadResource`, `OnlineResource` and `VideoResource` alike.
So an Easel Activity can hang off an ordinary digital product, and `EASEL` as an `itemType` denotes only the standalone case.
Modelling Easel as a product type alone would miss the attachment case entirely.

The `Format` vocabulary carries `easel`, `easel-activities` and `easel-assessments` facets, which are the buyer-facing surface of the same thing.

### Video

`VIDEO` is a distinct `itemType` with its own create route, `/My-Products/New/Video-Next`, referenced from the digital form's own `video_message` warning that steers a seller uploading a video file toward the video flow instead.
Video products add `videoType` (8 values, `OTHER` pairing with free-text `videoTypeText`, wire field `data[ItemsVideoProperty][video_type_text]`).
A video *preview* is separate from a video *product*: the digital form carries `data[Upload][videopreview]` behind the `VideoPreviewForDigitalProduct` A/B flag, so a digital product may have a video preview without being a video product.

`ONLINE_RESOURCE` is the fifth type, distinct from `DIGITAL_PRODUCT`; help article 20315328410132 covers upgrading a Digital Download to an Online Resource, so the two are convertible.
Only `DIGITAL_PRODUCT` occurs across the founder's 154 products, so every other type is untested by us.

## Copyright

The founder's enumeration describes "Two boxes to tick regarding intellectual rights."
The wire carries a single field, `data[ItemsProperty][copyright_declaration]`, with two members: `1` `ORIGINAL_WORK` and `2` `USED_COPYRIGHTED_MATERIALS`.
Both captures posted `1` and `UploadPageProductQuery` read back `ORIGINAL_WORK`.

The exact wording of the two assertions is not recoverable from anything we hold.
The form is client-rendered by the `tpt-frontend` React bundle, the HAR records every chunk request with a zero-length body, and the captured screenshot crops above the Copyright section.
Grepping both captured page HTML documents for `original work`, `copyrighted material`, `trademark rights` and `copyrights` returns nothing.

What the public record does say, from help article 360042864711 on adding a product, is that sellers must confirm their materials do not violate "the copyrights, trademark rights, or any other rights of a third party."
Help article 360042626591, TPT's Seller Guidelines, states sellers "should only sell resources that you've produced or designed yourself" and that on uploading a seller represents and warrants they "have the necessary rights to use all of the content" in a resource, that "neither TPT nor any member will have to obtain a license or pay royalties to any third parties," and that use of the resource "will not infringe on anyone's rights."
The policies referenced are TPT's Seller Guidelines and its Copyright and Trademark policy, and enforcement runs through DMCA notice-and-takedown with account closure for repeat violators.

Our adapter's handling is already the right shape and should not be relaxed.
`AuthorshipDeclaration` in `write_model.rs` has no `Default` and no `from_bool`; the only constructor names the attesting party and the instant, so nothing in the crate can put a `1` on the wire without a seller having actually attested.
The commentary there is explicit that a hard-coded literal would have the connector attesting on a seller's behalf to something nobody asked them.

One consequence for the base-product model: the two boxes are mutually exclusive on the wire, being one enum rather than two booleans.
If the form genuinely renders two independent checkboxes, then either exactly one may be ticked, or a second field exists that neither capture posted.
This is unresolved.

### Title screening

Separately from the copyright declaration, TPT screens the title.
`CheckResourceTitle($title: String!) { checkTitleForCopyrightInfringement(title: $title) { rightsholderId rightsholderName rightsholderUrl keywords } }` fires as the seller types, and the read-side `UploadPageProductQuery` exposes a matching `copyrightInfringement { name }` on the product.
This is a real pre-submit validation with a rightsholder identity attached, and any tool that writes titles to TPT should call it before submitting rather than discovering the problem as a rejection.
Help article 360042971611 gives the seller-facing rationale — titles are generally not copyrightable but may be trademarked — but documents no automated screen, so the operation itself is the only evidence that one exists.

## Education standards

### How the form exposes them

Four collapsed sections, each with a `Select ...` link opening a picker: `Select CCSS`, `Select NGSS`, `Select TEKS`, `Select VA SOL`.
Helper text on CCSS: "Buyers can opt-in to translating CCSS into their state-specific standards, when possible. They'll be able to see both the standards that you've tagged and their state-specific equivalents."
On TEKS: "TEKS will only be displayed for Buyers in Texas."
On VA SOL: "VA SOL will only be displayed for Buyers in Virginia."

### The API

Two operations, both on `/graph/graphql`, both unauthenticated in shape though issued with a session in the capture.

`EducationStandardsJurisdictionsQuery` returns `educationStandards { jurisdictions: roots { id name notation } }` and yields 166 roots.
`EducationStandardsQuery($id: ID!, $depth: Int)` returns `educationStandards { children(id, depth) { id name depth notation grades type parentIds sequence categorySequence descriptionText descriptionHtml } }` and expands one subtree.

The picker drives this hard: 58 `EducationStandardsQuery` calls in the capture, expanding one node at a time as the seller drills down, with `depth: 1` for the top-level expansion of each jurisdiction and no `depth` argument for deeper nodes.
Across those 58 responses the capture holds 4162 nodes of `type: standard`, 272 `domain`, 186 `cluster` and 36 `subject`.

`EducationStandardsByIds($ids: [ID]!) { educationStandards { find(ids) { id name parentIds } } }` resolves a saved set back to names, which is how the edit form re-renders an existing product's selections.

### Identifier format

Two identifiers coexist and must not be confused.
The wire field `data[ItemsCommonCoreStandard][common_core_standard_id][]` takes TPT's own opaque numeric node id — the same `id` the GraphQL tree returns, confirmed because `EducationStandardsByIds` in the capture queries exactly the 91 ids the create posted.
The human-readable notation is the node's `name`, for example `CCRA.L.1`, which is the published standard code.
The read side returns the numeric id under the alias `sphinxId`, which is a further hint that the id space is a search-index artefact rather than a stable public identifier.

A node carries `grades` as an array of strings (`["Kindergarten","1","2",...,"12"]`), `parentIds` as the full ancestor chain rather than a single parent, and `descriptionText` plus `descriptionHtml` carrying the standard's prose.
The four-level shape for Common Core is jurisdiction 3054, subject (3052 ELA, 3053 Math), domain, cluster, standard.

### Jurisdictions

166 roots, of which the form offers four: 3054 Common Core State Standards (`ccss`), 3055 Next Generation Science Standards (`ngss`), 3326 Texas Essential Knowledge and Skills (`teks`), 5785 Virginia Standards of Learning (`va sol`).
The remaining 162 span every US state's own frameworks plus Ontario Curriculum (8044) and Wales National Curriculum (8175).
28 of the 166 carry a `notation`.
That the other 162 exist but are not offered on the create form means TPT holds far more standards data than the form writes to, and the four offered are a deliberate product choice.

TPT exposes no public listing of supported standards that we found; the jurisdictions query is the enumeration.

## Public-web evidence

Every article below was fetched 2026-09-02 through `https://help.teacherspayteachers.com/api/v2/help_center/en-us/articles/{id}.json`.
The rendered `hc/en-us/articles/...` URL for the same id returns HTTP 403 to a non-browser client.

| id | Title | Operative sentence used |
|---|---|---|
| 47530264334868 | What are Tax Codes and how do I apply them to my resources? | "free resources are not taxed"; bulk edit "up to 100 products" |
| 360042864711 | How do I add a product to my TPT store? | sellers must confirm materials do not violate "the copyrights, trademark rights, or any other rights of a third party"; the "Make Listing Active" checkbox controls immediate availability |
| 360042865851 | What's the difference between a thumbnail and a preview? | "can showcase up to four thumbnail images per product"; a preview is "a demo-length version of the actual product" and "can be any length, but most are 1-3 pages" |
| 360042885411 | Can I change the multiple license price of my products from the default of 10% off? | "Although the default discount is 10% off, you can choose whatever discount seems right to you" |
| 360042191412 | What is the TPT Bundle feature and how is it different from traditional bundles? | "A Bundle must contain at least 2 resources and can include up to 500 resources or bundles"; "TPT Bundles automatically update when changes are made to the resources included in them" |
| 7918722760724 | How do I create a standalone Easel listing? | Easel is selected as its own product option; an attached Activity "cannot" be attached to another listing |
| 360042626591 | What are TPT's Seller Guidelines? | sellers "should only sell resources that you've produced or designed yourself"; "have the necessary rights to use all of the content" |
| 360042971611 | Can the title of my resource infringe on someone's intellectual property? | titles are generally not copyrightable but may be trademarked |
| 360042449172 | What kind of video can I upload? | referenced from the create page's own `video_message`; not fetched |

Section listing 360009101391 (My Products) holds 27 articles and is the index for upload-side documentation.

Two secondary sources were consulted and are cited as secondary.
The TPT Seller Blog post on tags and filters records the March 2025 overhaul, the tag cap rising from three to six, and the addition of a "Not subject specific" tag; the blog is not fetchable through the content API and this is from search-result summary rather than from the article body.
Seller-community posts describe bundle discount conventions; none is authoritative and no number from them is carried into this document as fact.

## Gaps

What we cannot enumerate from any source we hold, and what would close each.

1. The two copyright checkboxes' exact wording. The form is client-rendered and no JavaScript body was captured. Closed by a DOM snapshot of the Copyright section, or a screenshot of the form below Education Standards.
2. Whether the two boxes are two independent checkboxes over one enum or a two-value radio. Same capture closes it.
3. The `data[Item][generate_thumbnail]` value set. `1` and `3` are observed with no enum anywhere. Closed by the same DOM snapshot, or by mining the `tpt-frontend` chunk that renders the thumbnail control.
4. Whether `generate_thumbnail` accepts a file upload, as its `$_FILES` quadruple in the unlocked list implies. Closed by a capture of a seller uploading a custom master thumbnail.
5. The subject-area cap discrepancy: the form says three, the create posted four. Closed by a capture of the Subject Area picker refusing a fourth selection, or accepting a fifth.
6. Whether a Resource Type picker exists on the current create form. 71 `Type-of-Resource` facets exist and drive buyer filters, but the visible form region shows Grade Level, Subject Area, Tag, Format and Custom Category only, and the founder's enumeration lists no resource type. Closed by a full-page screenshot of the Categories section.
7. The bundle create form's field set, above all how the member resource list is carried. No bundle capture exists. Closed by a capture of a bundle create.
8. The bundle discount price constraint. Help article 360042192372 returned HTTP 401 through the content API. Closed by fetching that article through a browser, or by a capture showing the form rejecting an out-of-range discount.
9. `data[Item][discountprice]` semantics on a non-bundle listing, and what `data[Item][discount]` is for. Closed by a capture of a seller setting a bundle discount.
10. The Easel and Online Resource create forms. Neither is captured and neither type exists in the founder's 154 products. Closed by captures of each.
11. The video create form at `/My-Products/New/Video-Next`. Referenced but never fetched. Closed by a capture.
12. Custom category caps, per product and per store. Undocumented and unobserved. Closed by a seller hitting a limit, or by a help article we have not found.
13. The country vocabulary behind `countryId` and whether the `Appropriate for New Zealand` label varies by seller country. `countryId 153` is New Zealand for this seller and no country list was found in any chunk. Closed by rendering the form as a US seller.
14. `ItemsProperty.audience`, `RevisedItem.comment` and `ItemsVideoProperty.video_type_text` are unlocked but were never posted, so their form controls are unlocated. Closed by the same full-form DOM snapshot.
15. The title length cap's counting unit — bytes, code points or grapheme clusters. 80 is a client constant and the longest observed title; no server refusal was measured. Closed by submitting an 81-character title and a title with astral-plane characters.
16. Whether the seller may exceed a stated picker cap by posting directly. Every cap we hold is client-side text. Closed by a deliberate over-cap submit, which is a write and needs founder consent.
17. Which tax engine supplies the codes. Inferred as Avalara from code shape; no TPT source names a provider. Closed by TPT support, or left as inference since nothing downstream depends on it.

## Open questions for the founder

1. Should the canonical model carry an explicit additional-licence price, or continue deriving it as 90 percent of price? Deriving it silently overwrites the pricing of any seller who set a custom additional-licence price, and TPT's help centre is explicit that sellers may set any value they like.
2. Should the tool ever default a tax code? TPT's Terms of Service make the seller responsible for designating tax codes, so defaulting one is a tax determination made on their behalf. The `AuthorshipDeclaration` pattern already in the adapter is the obvious precedent for refusing to.
3. Is `Type-of-Resource` still a seller-writable picker? 71 facets exist and drive buyer filters, but no picker for them is visible in the captured form region and your enumeration omits it. If it is gone, the canonical model should not carry a resource-type field as a first-class TPT concept.
4. Which non-digital product types are in scope for the base model? Bundle, Easel, Online Resource and Video each have their own form and none is captured or represented in your 154 products, so modelling them means capturing them first.
5. Do you want a fresh capture that scrolls the whole create form with the DOM saved rather than the network? Nine of the seventeen gaps close with one such artefact, and it needs no write and no submit.
6. `~/downloads/tpt-capture-options-2.png` is a HAR file with a `.png` extension. Worth renaming at the source so a future reader does not treat it as a corrupt screenshot.

## Unverified

Claims in this document that rest on inference rather than direct observation, restated so they are not read as settled.

- The tax codes are Avalara product tax codes. Inferred from code prefixes and description wording; no TPT source names a provider.
- Which tax code suits a PDF worksheet. Inferred from the code descriptions; TPT publishes no mapping, and its own default for the founder's test product was `DO010000`.
- `generate_thumbnail` accepts an uploaded file. Inferred from the `$_FILES` quadruple in the unlocked list; never observed.
- Posting a subset of the 48 unlocked fields is accepted. Inferred in the prior recon from an empty locked-field list and observed to work for the browser's own ten omissions; no rejection case was observed.
- The subject-area cap is client-side and advisory. One of two candidate explanations for the four-slug create; the other is parent-plus-leaf expansion in the picker.
- The `Appropriate for New Zealand` label is seller-country-derived. Consistent with `countryId 153` and the founder's `+12:00` capture timezone, but only one seller's form has been seen.
- Wire field order does not affect acceptance. Inferred from the create and edit captures disagreeing on the position of `tax_code_id`; not tested by a deliberate reorder.
- Bundle discount conventions near 20 percent default and 50 percent cap. Secondary seller-community sources only; the authoritative article was unreachable.
- The `en-espanol` and `en-francais` mojibake is an extraction artefact of the 2026-08-29 poll rather than TPT's own encoding.
