# The TPT create form as the DOM presents it

Layout, control-by-control inventory and option lists for TPT's digital create form, read from a full-page DOM snapshot.
Research date 2026-09-03.
This document closes or narrows the seventeen gaps listed in `tpt-product-model.md` and is the layout spec for our own listing form.

## Executive summary

1. The form is nine ordered sections — Name, Files, Description, Price, Categories (which contains Education Standards as a nested subsection), Details, Copyright, Product Status, Submit — mounted as ten sibling React roots inside `#ItemAddForm`, with the previews and thumbnails block as one legacy non-React island between the Files and Description roots.
2. Copyright is a radio group, not two checkboxes: one `name="data[ItemsProperty][copyright_declaration]"` group with values `1` and `2`, both wordings now quoted verbatim below, and value `1` arrives pre-selected on a blank form.
3. `data[Item][generate_thumbnail]` is a three-way radio, closing the enum: `1` auto generate from the product file (default), `2` upload thumbnails now, `3` upload thumbnails later; the four thumbnail slots are the conditional body of option `2`.
4. There is no Type-of-Resource picker on the create form; the only "Resource type" string in the page is the site header's browse dropdown, outside the form.
5. There is no Primary Audience control on the form at all, so `data[ItemsProperty][audience]` is unreachable from this route.
6. Tax Code, Teaching Duration and Answer Key are custom listbox widgets whose full option lists *are* in the DOM, but as label-only `<li role="option">` elements with no value attribute, so their wire ids are not recoverable from markup — and the Answer Key display order is not the id order.
7. Four pickers — Subject Area, Tag, Format, Custom Category — are closed `react-select` MultiSelectV2 comboboxes with zero options in the DOM; their vocabularies still come only from the webpack bundle and the seller-scoped GraphQL query.
8. The form carries no native input for the taxonomy array, the standards ids, the tax code, the duration or the answer key: those five reach the wire only through JavaScript at submit, which is decisive for any plain-HTTP form-scrape-and-replay adapter.
9. `data[_Token][unlocked]` is byte-identical to the 48 paths decoded from the 2026-08-30 capture, in the same order, so the server-side field inventory has not moved.
10. Two new constraints surfaced that no earlier source held: a tooltip stating "Free resources should be 10 pages or fewer.", and the Custom Category tooltip defining what a custom category is.

## Provenance and handling

The single input is `~/downloads/tpt-create-form.html`, 286,134 bytes on 678 lines, an outerHTML copy of `https://www.teacherspayteachers.com/My-Products/New/Digital-Next` taken by the founder on 2026-09-03 after scrolling the whole form.
The page is the *blank* create form: the SSR blob's `formValues` is an empty object and every text input carries `value=""`, so every checked state and pre-filled value recorded below is TPT's own default rather than something the founder entered.
The seller is id 21268787, store EBMCResources, `user_group` `seller_upgrade`, country New Zealand, 154 products.

The snapshot is not copied into this repository and should not be.
It embeds a live CSRF key and token pair, the CakePHP security token triple, the seller's email address, and an AWS access key id used for the browser's direct-to-S3 upload.
Treat the file as a credential-bearing artefact.

Two inline scripts in the snapshot are themselves new evidence and are quoted from below: the `var cfg` bootstrap, and a `data-tpt-state` script carrying the server-rendered Redux and Apollo state plus 125 LaunchDarkly flags.

## Section structure in DOM order

`#ItemAddForm` posts `multipart/form-data` to `/My-Products/New/Digital-Next`, carries `novalidate`, and has twelve direct children: a security-token `div`, ten React mount roots, and one legacy markup island.

The order, with the anchor for each and the exact strings shown to the seller.

`#react-name-section`, which also carries the page `<h1>` "Upload New Product" at `[data-testid=upload-form-heading]`, and the props `data-title-input-name="data[Item][name]"` and `data-description-input-name="data[Item][description]"`.
Its section is `#section_Name.SectionHeading.NameSection`, heading "Name" at `.SectionHeading__headingTitle`, and the one field is labelled "Title" with a separate "Required" span at `.SectionLabelText__withLeftMargin`.
An empty `.ProductTitleModalBespoke` div follows the input; this is the mount for the `CheckResourceTitle` trademark-screening modal.

`#react-file-section` holds `#section_Files.SectionHeading.FileSectionLayout.FileSectionLayout__empty`, heading "Files" with an inline "Supported File Types" button at `.FileSectionLayout__supportedFileTypes` whose modal mount `#react-upload-box-supported-formats` is empty until clicked.
The upload box `#upload-product.upload-fls-el.product-file` is titled "Downloadable File" with a "Required" span at `.upload-fls-required`, the affordances "Select file", "or drag and drop", a hidden "Drop files here" state, and the cap text "Up to 4 GB" at `.max-filesize`.
A hidden `.video-resource-on-tpt-popup` dialog carries the video-steering text: "It looks like you're uploading a video. You might want to create a Video Resource on TPT instead. You can upload a Video Resource here and learn more about Videos on TPT here.", linking `/My-Products/New/Video-Next` and help article 360042449172.
Three further React mounts sit here — `#react-easel-opt-in-prompt`, `#react-attach-assessments-section` and `#react-easel-alert-section` — the last rendering the alert "Did you know? To add an Easel Activity and/or Assessments to this product, save your changes then go to My Easel Listings", with a "Learn more" link.

`.yui3-g.form_item.upload-fls-form` is the legacy island, jQuery-era markup rather than React.
Its subheading is "Product Previews" at `.upload-fls-subheader`, holding `#upload-preview` titled "Preview" with "Up to 30 MB", and `#videoupload-videopreview` titled "Video Preview" with "Up to 1 GB".
Below sits `.yui3-u.thumbnails-generation`, the three-way thumbnail mode radio, whose auto branch shows the placeholder "Thumbnails will generate once you upload your product file" and the error state "We cannot generate images from your file. Please upload a different file or your own thumbnail images.", plus a "See all thumbnails" button (initially `display: none`) opening a "Select Thumbnails to Feature" dialog with the sub-labels "Selected Thumbnails" and "Auto generated Thumbnails".
The manual branch `.manually-uploaded` is `display: none` on load and contains `#upload-thumb1` titled "Main Cover" and `#upload-thumb2` through `#upload-thumb4` titled "Thumbnail (Optional)", each with "Select file or drag and drop" and "Up to 4 MB".

`#react-upload-description-editor` carries `data-description-input-name="data[Item][description]"` and renders a Lexical editor at `[data-testid=description-editor]`.
The label "Description" plus "Required" sits at `#label_DescriptionEditor`; the toolbar exposes bold, italic, underline, numbered list, unordered list, indent, outdent, a link button and one disabled button; the editing surface is a `contenteditable` `[data-lexical-editor=true]` with the placeholder "Describe your product and how it can be helpful to another educator".

`#react-price-section` holds `#section_Price`, heading "Price", then in order: the "Free Resource" checkbox with a tooltip icon reading "Free resources should be 10 pages or fewer."; "Price" (Required); "Multiple Licenses" (Required) with an unlabelled info popover; "Bundle Discount Price" with an info popover and no required marker; and `.PriceSectionTaxCodeLayout` holding the "Tax Code" (Required) listbox, its help line "Completion of this field is required in order for sales tax to be collected on this product.", and the link "View Full Code Descriptions" to `/University/Sales-Tax/Tax-Codes`.

`#react-category-section` holds `#section_Categories.SectionHeading.CategorySectionLayout`, heading "Categories", and five pickers plus a nested standards subsection and a localisation checkbox.
"Grade Level" (Required) at `.CategorySectionLayout__grade`, help text `Select up to four grades. If your product works for all grades, select "Not Grade Specific."`, rendered as a `.MultiCheckbox` of seventeen `.MultiCheckbox__option4` cells in a four-column grid.
"Subject Area" (Required) at `.CategorySectionLayout__categorySubject`, placeholder "Select up to three subject areas".
"Tag" with the parenthetical "(Theme, Audience, Language)" and "Required" at `.CategorySectionLayout__categoryTag`, placeholder "Select up to six tags".
"Format", no required marker, at `.CategorySectionLayout__formatTag`, placeholder "Select up to three formats".
"Custom Category", no required marker, at `.CategorySectionLayout__categoryCustom`, placeholder "Select Custom Categories", with a tooltip reading "A custom category is any word or phrase you'd like to use to categorize your resources. You can manage your custom categories on your “My Product Listings” page."
Then `#section_Education Standards`, heading "Education Standards", nested inside the Categories section rather than being a sibling of it, with four slots described below.
Then "Appropriate for New Zealand" at `.CategorySectionLayout__appropriate`, with a "How is this used?" button.

`#react-details-section` holds `#section_Details.SectionHeading.DetailsSectionLayout.DetailsSectionLayoutRHF`, heading "Details", with "Teaching Duration", "Number of Pages or Slides" and "Answer Key" in that order.
None carries a required marker.

`#react-copyright-section` holds `#section_Copyright`, heading "Copyright".
`#label_intellectualPropertyRightsTitle` reads: "Intellectual Property Rights: By uploading the selected material I certify that I have read and agree to the Teachers Pay Teachers terms of service and that the selected material does not infringe the copyrights, trademark rights, or any other rights of any third party, and:", with "terms of service" linking `/Terms-of-Service`.
Below it is the radio group `#data.ItemsProperty.copyright_declaration`.

`#react-product-status-section` holds `#section_Product Status`, heading "Product Status", the "Make Listing Active" checkbox, the caption "Active listings are visible on the site and searchable. Inactive listings are only visible to you. When you activate a listing for the first time, your followers will receive an email (if they opted in).", and a dismissible alert: "Did you know? To save this as a draft that's visible only to you, uncheck "Make Listing Active" and click Submit to save your changes. For Easel listings, this keeps your listing private while you create and attach."

`#react-submit-section` holds the "Submit" and "Cancel" buttons and the caption "Please allow up to one hour for new or edited products to appear in your store listings."
The valueless `<input type="hidden" name="data[TaxonomyTags][]">` sits immediately after the Cancel button, and the two remaining security tokens close the form.

## Control-by-control inventory

Every named control inside `#ItemAddForm`, in DOM order.
Where a name is written in dot notation it is a react-hook-form field path on a `div`, not a submittable input.

| Label as shown | name | Control type | Anchor | Required marker | Default | Notes |
|---|---|---|---|---|---|---|
| — | `_method` | hidden | — | — | `POST` | CakePHP method override |
| — | `data[_Token][key]` | hidden | `#Token1412316549` | — | 40-hex | security token |
| — | `data[_Csrf][csrfKey]` | hidden | `#_Csrf1372922858` | — | opaque | |
| — | `data[_Csrf][csrfToken]` | hidden | `#_Csrf2020425062` | — | same value as csrfKey | |
| Title | `data[Item][name]` | text | `#ItemName`, `[data-testid=upload-form-item-name]` | yes | empty | `maxlength="80"`, placeholder "Name your product" |
| Description | `data[Item][description]` | Lexical contenteditable, no input element | `#label_DescriptionEditor`, `[data-testid=description-editor]` | yes | empty | name appears only as `data-description-input-name` on two mount roots |
| Downloadable File | `data[ItemDigital][product]` | hidden, plus a sibling unnamed `type=file` | `#ItemDigitalProduct`, `.upload-key` | yes | empty | 4 GB cap shown |
| — | `data[ItemsProperty][product_uploaded]` | hidden | `#ItemsPropertyProductUploaded`, `.file-uploaded` | — | `0` | |
| Preview | `data[ItemDigital][preview]` | hidden + unnamed file input | `#ItemDigitalPreview` | no | empty | 30 MB cap shown |
| — | `data[ItemsProperty][preview_uploaded]` | hidden | `#ItemsPropertyPreviewUploaded` | — | `0` | |
| Video Preview | `data[Upload][videopreview]` | hidden + unnamed file input | `#UploadVideopreview` | no | empty | 1 GB cap shown; A/B flag `VideoPreviewForDigitalProduct` is `on` for this seller |
| — | `data[Upload][custom_videopreview_uploaded]` | hidden | `#UploadCustomVideopreviewUploaded`, `.custom-file-uploaded` | — | `0` | |
| Auto generate thumbnails from the product file | `data[Item][generate_thumbnail]` | radio | `#ItemGenerateThumbnail1` | — | `checked` | value `1` |
| Upload thumbnails now | same | radio | `#ItemGenerateThumbnail2` | — | — | value `2`; reveals `.manually-uploaded`, which is `display: none` on load |
| Upload thumbnails later | same | radio | `#ItemGenerateThumbnail3` | — | — | value `3` |
| — | `thumbs` | hidden | inside the auto branch | — | empty | |
| — | `thumbs_collection_key` | hidden | inside the auto branch | — | empty | |
| Main Cover | `data[ItemDigital][thumb1]` | hidden + unnamed file input | `#ItemDigitalThumb1`, box `#upload-thumb1` | no | empty | 4 MB |
| Thumbnail (Optional) | `data[ItemDigital][thumb2..thumb4]` | as above | `#ItemDigitalThumb2..4`, boxes `#upload-thumb2..4` | no | empty | 4 MB each |
| — | `data[ItemsProperty][thumb1_uploaded]`..`thumb4_uploaded` | hidden | `#ItemsPropertyThumbNUploaded` | — | `0` | |
| Free Resource | `data[Item][free]` | checkbox, Radix pattern | button `#item-free`, visually hidden `input[type=checkbox]` | no | unchecked | native value `on`; tooltip "Free resources should be 10 pages or fewer." |
| Price | `data[Item][price]` | text | `#item-price`, `[data-testid=upload-form-item-price]` | yes | empty | placeholder `0.00`; no `maxlength`; no `$` in markup, so the currency mark seen in the screenshot is CSS |
| Multiple Licenses | `data[Item][license_price]` | text | `#item-license-price`, `[data-testid=upload-form-item-license-price]` | yes | empty | placeholder `0.00`; info popover `[data-testid=info-icon][data-state=closed]` with no text in the DOM; no pre-fill in markup |
| Bundle Discount Price | `data[Item][discountprice]` | text | `#item-bundle-price`, `[data-testid=upload-form-item-discount-price]` | no | empty | placeholder `0.00`; same closed info popover |
| Tax Code | `data.ItemTaxCode.tax_code_id` | custom listbox, no input element | toggle `#taxCode-toggle-button[role=combobox]`, menu `#taxCode-menu[role=listbox]` | yes | none selected, toggle shows "Select a tax code" | five options, appendix A |
| Common Core State Standards | `data[ItemsCommonCoreStandard][common_core_standard_id]` | modal picker; the name sits on a `div`, not an input | `#education-standards-3054`, `[data-testid=standards-section-3054]` | no | none | button "Select CCSS" |
| Next Generation Science Standards | same | same | `#education-standards-3055` | no | none | button "Select NGSS" |
| Texas Essential Knowledge and Skills | same | same | `#education-standards-3326` | no | none | button "Select TEKS" |
| Virginia Standards of Learning | same | same | `#education-standards-5785` | no | none | button "Select VA SOL" |
| — | `data[ItemsCommonCoreStandard][common_core_standards_num]` | hidden | — | — | `0` | the only real input the standards block emits |
| Grade Level | none on the wire; ids are `data.Grade.Grade-checkbox_<slug>` | checkbox group, 17 members | `.MultiCheckbox`, cells `.MultiCheckbox__option4` | yes | all unchecked | serialises into `data[TaxonomyTags][]`; appendix B |
| Subject Area | none | react-select MultiSelectV2, searchable | `#subject-areas[role=combobox]`, label `#label_subject-areas` | yes | empty | zero options in the DOM |
| Tag (Theme, Audience, Language) | none | same | `#tags`, label `#label_tags` | yes | empty | zero options in the DOM |
| Format | none | same | `#formats`, label `#label_formats` | no | empty | zero options in the DOM |
| Custom Category | none | same | `#custom-categories`, label `#label_custom-categories` | no | empty | zero options in the DOM |
| Appropriate for New Zealand | `data[ItemsLocalization][country_id_flag]` | checkbox, Radix pattern | button `#item-appropriate` | no | unchecked | native value `on`; "How is this used?" button beside it |
| Teaching Duration | `data.ItemsProperty.duration` | custom listbox | toggle `#teachingDuration-toggle-button`, menu `#teachingDuration-menu` | no | `N/A` selected | 23 options, appendix C |
| Number of Pages or Slides | `data[ItemsProperty][pages]` | text | `#numberOfPagesOrSlides`, `[data-testid=upload-form-item-pages]` | no | empty | placeholder "Total pages or slides"; no `maxlength`, no numeric type |
| Answer Key | `data.ItemsProperty.answer_key` | custom listbox | toggle `#answerKey-toggle-button`, menu `#answerKey-menu` | no | `N/A` selected | 6 options, appendix D |
| I attest … original work | `data[ItemsProperty][copyright_declaration]` | radio | `#data.ItemsProperty.copyright_declaration-0` | the group is implicitly required | `checked`, `aria-checked="true"` | value `1` |
| I attest … used copyrighted and/or trademarked materials | same | radio | `#data.ItemsProperty.copyright_declaration-1` | — | unchecked | value `2` |
| Make Listing Active | `data[Item][status_user]` | checkbox, Radix pattern | button `#makeProductActive` | no marker | `checked` | native value `on` |
| — | `data[TaxonomyTags][]` | hidden, single, valueless | after the Cancel button | — | none | a placeholder; the real repeated parts are produced at submit |
| — | `data[_Token][fields]` | hidden | `#TokenFields1953624371` | — | `e45c98…%3A`, locked list empty after the colon | |
| — | `data[_Token][unlocked]` | hidden | `#TokenUnlocked1749253974` | — | 48 pipe-separated paths | identical to the 2026-08-30 capture |

Every checkbox follows the same Radix pattern: a `button[role=checkbox][data-state]` carrying the visible state, and a visually hidden `input[type=checkbox]` carrying the wire name with `value="on"`.
The wire integers seen in the capture — `status_user` `0` or `1`, `country_id_flag` `0` or `1` — are therefore produced by JavaScript, not by the markup.

## Closing the seventeen gaps

1. The copyright wording. Closed. Option 1 reads: "I attest that this product I am about to post is an original work and it does not infringe upon the Intellectual Property rights of others." Option 2 reads: "I attest that I have used copyrighted and/or trademarked materials in my product and it does not infringe upon the Intellectual Property rights of others. I have either received express permission to use such materials, or I hereby certify that the use of such materials is otherwise non-infringing, for example as a fair use."
2. Two checkboxes or a radio. Closed. It is a single radio group, `.RadioGroup-module__root` at `#data.ItemsProperty.copyright_declaration`, with `role="radio"` buttons and native `input[type=radio][name="data[ItemsProperty][copyright_declaration]"]` at values `1` and `2`; the two are mutually exclusive and the earlier "two boxes to tick" reading is wrong. One consequence matters: value `1` is pre-selected on a blank form, so a naive scrape-and-replay would post an attestation the seller never made, which is exactly what our `AuthorshipDeclaration` type exists to prevent.
3. The `generate_thumbnail` enum. Closed. Three radio values with these labels: `1` "Auto generate thumbnails from the product file" (default), `2` "Upload thumbnails now", `3` "Upload thumbnails later". The captured edit posting `3` therefore means the seller deferred thumbnails, not a fourth mode.
4. Whether `generate_thumbnail` accepts a file. Still open, and now less likely. Mode `2` routes seller files to `thumb1..thumb4`, so nothing in the visible form posts a file to `generate_thumbnail`; the `$_FILES` quadruple in `unlocked` looks vestigial. Only a server-side probe would settle it, and that is a write.
5. The subject-area cap. Partially closed. The visible hint is exactly "Select up to three subject areas" and it is the react-select placeholder, not a validation attribute; there is no `max`, `maxlength`, or data attribute anywhere on the control. The DOM cannot say whether the cap is enforced, so the four-slug create remains unexplained, and the parent-plus-leaf hypothesis is untested.
6. Type-of-Resource picker. Closed, denied. "Resource type" occurs twice in the page, both inside `.CategoryDropDown[data-testid=CategoryDropDown]` in the site header's browse menu, outside `#ItemAddForm`. The create form offers Grade Level, Subject Area, Tag, Format and Custom Category and nothing else.
7. The bundle create form's field set. Still open. This snapshot is the digital form; no bundle form was captured.
8. The bundle discount price constraint. Still open, but now locatable. "Bundle Discount Price" carries a `[data-testid=info-icon]` popover whose content is not rendered until opened, so a second snapshot with that popover open would very likely carry TPT's own explanation.
9. `discountprice` semantics on a non-bundle listing, and what `data[Item][discount]` is. Partially closed. The field is present, editable and unmarked on a digital product, sharing the Price section with Price and Multiple Licenses; it is not hidden or disabled. `data[Item][discount]` has no control anywhere in the form and remains unexplained.
10. Easel and Online Resource forms. Still open. The digital form does carry three Easel mounts — `#react-easel-opt-in-prompt`, `#react-attach-assessments-section`, `#react-easel-alert-section` — all empty on a blank create, which corroborates that Easel attaches to an ordinary digital product after the first save.
11. The video create form. Still open. The digital form only links to `/My-Products/New/Video-Next` from its steering dialog.
12. Custom category caps. Still open. The picker carries no cap text; its tooltip explains what a custom category is and where to manage them, and states no limit.
13. The country vocabulary behind `country_id_flag`. Still open. The label reads "Appropriate for New Zealand" for this NZ seller and no country list appears in the DOM or in `var cfg`.
14. `ItemsProperty.audience`, `RevisedItem.comment`, `ItemsVideoProperty.video_type_text`. Closed for this route, in the negative. None of the three has a control anywhere in the form; there is no Primary Audience picker, no revision-comment field, and no video-type field. They are unlocked but unreachable from the digital create form.
15. The title length counting unit. Partially closed. `maxlength="80"` is a real HTML attribute on `#ItemName`, so the browser truncates at 80 UTF-16 code units; whether the server counts the same way is still unmeasured.
16. Whether a seller can exceed a stated cap by posting directly. Still open; unchanged, and it needs a deliberate write.
17. Which tax engine supplies the codes. Still open. The DOM shows the five names only, and links to `/University/Sales-Tax/Tax-Codes`; no provider is named.

Two further items from the earlier open questions also resolve.
The `data[_Token][unlocked]` value decodes to exactly the same 48 paths in exactly the same order as the 2026-08-30 capture, so the field inventory is stable across a four-day interval and a different session.
The Multiple Licenses pre-fill is not in the markup: the field renders empty with placeholder `0.00`, and `var cfg` still carries `multiple_license_price_percentage: 90`, so the 90 percent is applied by client JavaScript when the seller types a price and remains a default rather than a rule.

## Vocabulary reconciliation

Four option lists are enumerable from this snapshot; four are not.

Grade Level: 17 checkboxes against 20 `Grade-Level` facets in `tpt-vocabulary.json`.
The slugs are exactly the facet slugs, taken from the checkbox ids `data.Grade.Grade-checkbox_<slug>`, and all 17 labels match the facet `name` field character for character.
Three facets are held in the vocabulary but not offered on the form: `elementary`, `middle-school`, `high-school`.
These are the band roll-ups that drive buyer-facing filters, so the correct reading is that the vocabulary is right and 17 of its 20 are seller-writable.
No additions, three removals for write purposes, zero label changes.

Tax Code: 5 options against 5, in the same order, with all five labels matching the vocabulary `name` exactly.
The DOM index maps to the wire id as `id = index + 1`.
No additions, no removals, no label changes.

Teaching Duration: 23 options against 23, in the same order, so the DOM index equals the vocabulary id.
Sixteen labels differ in capitalisation only: the DOM uses title case throughout ("30 Minutes", "1 Hour", "2 Hours", "2 Days", "1 Month", "Lifelong Tool") where the vocabulary recorded sentence case ("30 minutes", "1 hour", "2 hours", "2 days", "1 month", "Lifelong tool").
Ids 0, 13, 14, 15, 19, 20 and 22 already match.
The DOM is the display authority, so the vocabulary's labels should be corrected to title case; the ids are unaffected.

Answer Key: 6 options against 6, with all six labels matching — but the display order is not the id order.
The DOM sequence is N/A, Included, Not Included, Included with Rubric, Rubric Only, Does Not Apply, which maps to vocabulary ids 0, 1, 2, 4, 5, 3.
Anything that infers an id from a menu position will silently write "Included with Rubric" as "Does Not Apply".
This is the one reconciliation finding with a correctness consequence, and it applies to Teaching Duration and Tax Code too as a matter of principle even though those two happen to be ordered.

Subject Area, Tag, Format and Custom Category contribute nothing: the file contains zero `Select__option` and zero `Select__menu` elements, because react-select renders its menu only while open.
Their counts in the vocabulary — 140 subject areas of which 7 hidden, 52 tags, 26 formats, 7 seller custom categories — are neither confirmed nor contradicted by this snapshot.

## What is static markup and what is a client-rendered mount

Static in the DOM, and therefore extractable without JavaScript: the section skeleton and every heading, all label and help text, the seventeen grade checkboxes with their slugs, the three thumbnail-mode radios, the two copyright radios, every plain text and hidden input, the three custom listboxes with their full option label lists, all four upload boxes with their size caps, and both security-token triples.

Present as a mount point with its content absent until the client acts: the "Supported File Types" modal (`#react-upload-box-supported-formats`, empty); the title-screening modal (`.ProductTitleModalBespoke`, empty); the four education-standards modals (`.EducationStandardsModal > .DialogModal`, each empty, populated by `EducationStandardsQuery` one node at a time as the seller drills down); the four react-select menus; the Multiple Licenses and Bundle Discount Price info popovers (`data-state="closed"`); and the three Easel mounts.

What is still unknown after this snapshot, and why.
The subject, tag and format vocabularies, because they ship inside the `tpt-frontend` webpack bundle and are injected into react-select at runtime; the 2026-08-29 poll remains the only extraction.
The custom-category list, because it is fetched per seller by `MyProductListingsCustomCategories`.
The standards tree, because it is fetched per node by `EducationStandardsQuery`.
The wire id for every listbox option, because the `<li>` elements carry only labels; the ids come from the vocabulary file and from the captured wire.
And the exact submit-time serialisation, because five wire fields — the taxonomy array, the standards id array, the tax code, the duration and the answer key — have no submittable input in the DOM at all.
That last point is the operative one for our adapter: a form-scrape-and-replay strategy that reads inputs out of this HTML will silently omit five fields, one of them required.

## Implications for our own form

The founder's instruction is that the drop-downs, checkboxes and options must show the way TPT shows them, with the same ease or better, and that the seller must clearly see which category belongs to what.
Taking that literally, control type per field should be:

Title as a plain text input with a hard 80-character limit and a live counter.
Description as a rich-text editor with the same six formatting affordances TPT offers — bold, italic, underline, ordered list, unordered list, indent and outdent — because a seller pasting TPT-shaped HTML into a plainer editor loses formatting on the round trip.
Files, preview, video preview and the four thumbnail slots as drop targets with the cap stated on the target, exactly as TPT states "Up to 4 GB", "Up to 30 MB", "Up to 1 GB" and "Up to 4 MB".
Thumbnail mode as a three-way radio with the manual slots revealed only under "upload now", matching TPT's own conditional.
Free Resource as a checkbox that hides Price, Multiple Licenses and Tax Code when set, since TPT's own help text says free resources are untaxed.
Price, Multiple Licenses and Bundle Discount Price as three separate money inputs, not one price control with modifiers.
Tax Code as a single-select whose options are the five full descriptions, never abbreviated to the Avalara code.
Grade Level as a checkbox grid of the same seventeen labels in the same four-column arrangement, because the arrangement is itself the segregation: primary grades, middle grades, high-school grades, and the three non-grade bands.
Subject Area, Tag and Format as searchable multi-selects with their caps stated in the placeholder, as TPT does.
Custom Category as a searchable multi-select of the seller's own shelves, visually separated from the three platform vocabularies, because it is the one picker whose values are the seller's rather than the marketplace's.
Education standards as a modal tree per jurisdiction, since the tree is far too large to inline.
Teaching Duration and Answer Key as single-selects.
Copyright as a radio group with both attestations quoted in full, never pre-selected, and never satisfiable by a default.
Product status as a checkbox with the draft explanation beside it.

Four places where we can do better without disturbing the segregation.
TPT states its caps in placeholder text and then lets the seller discover the limit by being refused, so a live counter against each cap — four grades, three subjects, six tags, three formats — is strictly more informative at no structural cost.
TPT's Answer Key menu is ordered differently from its own ids, which is invisible to a seller but a trap for us; ordering ours by id and displaying the label removes a whole class of mapping bug.
TPT gives no per-marketplace projection, so a preview showing what this listing will look like on each target marketplace, and which of our fields that marketplace will drop, is the single largest ease gain available and does not change the layout.
And TPT hides its explanatory text behind info popovers that render nothing until hovered; ours can state the same guidance inline, since we know what the fields mean.

One thing we must not copy.
TPT pre-selects the "original work" attestation on a blank form.
Ours must render the copyright group with nothing selected and refuse submission until the seller chooses, because the attestation is the seller's legal statement and a default makes it ours.

## Appendix: full option lists from the DOM

Only these four lists are in the markup; the four react-select pickers render no options while closed.
None of the `<li role="option">` elements carries a value attribute, so the wire ids below come from `docs/design/data/tpt-vocabulary.json` and the captured wire, not from this snapshot.

Appendix A, Tax Code, menu `#taxCode-menu`, five options in DOM order, wire id equals DOM index plus one.

| DOM index | li id | Label as shown | Wire id | Tax code |
|---|---|---|---|---|
| 0 | `taxCode-item-0` | Digital audio works sold to an end user with rights for permanent use | 1 | DA051011 |
| 1 | `taxCode-item-1` | Digital books sold to an end user with rights for permanent use | 2 | DB031013 |
| 2 | `taxCode-item-2` | Digital Images - Streaming / Electronic Download | 3 | DI010200 |
| 3 | `taxCode-item-3` | Videos - Streaming / Electronic Download | 4 | DV010200 |
| 4 | `taxCode-item-4` | Other Digital Goods - No Physical Media | 5 | DO010000 |

Appendix B, Grade Level, seventeen checkboxes in DOM order, which is row-major across a four-column grid.
The slug is the suffix of the checkbox id `data.Grade.Grade-checkbox_<slug>` and is also the `data[TaxonomyTags][]` value.

Row one: Preschool (`preschool`), 4th Grade (`4th-grade`), 9th Grade (`9th-grade`), Higher Education (`higher-education`).
Row two: Kindergarten (`kindergarten`), 5th Grade (`5th-grade`), 10th Grade (`10th-grade`), Adult Education (`adult-education`).
Row three: 1st Grade (`1st-grade`), 6th Grade (`6th-grade`), 11th Grade (`11th-grade`), Not Grade Specific (`not-grade-specific`).
Row four: 2nd Grade (`2nd-grade`), 7th Grade (`7th-grade`), 12th Grade (`12th-grade`).
Row five: 3rd Grade (`3rd-grade`), 8th Grade (`8th-grade`).

Read down the columns and the segregation is legible: primary grades, middle grades, high-school grades, and the three non-grade bands.
That column arrangement is the thing to reproduce, not the DOM order.

Appendix C, Teaching Duration, menu `#teachingDuration-menu`, twenty-three options.
DOM index equals the wire id for every entry.

`0` N/A (selected by default), `1` 30 Minutes, `2` 40 Minutes, `3` 45 Minutes, `4` 50 Minutes, `5` 55 Minutes, `6` 1 Hour, `7` 90 Minutes, `8` 2 Hours, `9` 3 Hours, `10` 2 Days, `11` 3 Days, `12` 4 Days, `13` 1 Week, `14` 2 Weeks, `15` 3 Weeks, `16` 1 Month, `17` 2 Months, `18` 3 Months, `19` 1 Semester, `20` 1 Year, `21` Lifelong Tool, `22` Other.

Appendix D, Answer Key, menu `#answerKey-menu`, six options.
DOM index does not equal the wire id.

| DOM index | li id | Label as shown | Wire id | Enum |
|---|---|---|---|---|
| 0 | `answerKey-item-0` | N/A (selected by default) | 0 | NA |
| 1 | `answerKey-item-1` | Included | 1 | INCLUDED |
| 2 | `answerKey-item-2` | Not Included | 2 | NOT_INCLUDED |
| 3 | `answerKey-item-3` | Included with Rubric | 4 | INCLUDED_WITH_RUBRIC |
| 4 | `answerKey-item-4` | Rubric Only | 5 | RUBRIC_ONLY |
| 5 | `answerKey-item-5` | Does Not Apply | 3 | DOES_NOT_APPLY |

## Open questions

1. Should our own form pre-select nothing in the copyright group and block submit until chosen, accepting one extra click against TPT's flow? Recommended yes, on the grounds above.
2. Should the vocabulary file's Teaching Duration labels be corrected to the DOM's title case, and an explicit id-versus-display-order note added to Answer Key? Recommended yes, as a small follow-up edit to `docs/design/data/tpt-vocabulary.json`.
3. Is a second snapshot worth requesting with the Multiple Licenses and Bundle Discount Price popovers open, and one react-select menu open? Recommended yes: it is another no-write artefact and it would close gap 8 and give us TPT's own wording for two fields we currently guess at.
4. `~/downloads/tpt-create-form.html` embeds a live CSRF pair, the seller's email and an AWS access key id. Should it be moved out of `~/downloads` or deleted once mined? Recommended: keep it for now, since re-capture costs a founder session, but do not copy it into any repository.

## Remaining unknowns

The subject, tag and format vocabularies, still bundle-only.
The custom-category list, still per-seller.
The standards tree, still per-node.
The wire id behind every listbox option label.
The submit-time serialisation for the five fields that have no input in the DOM.
Whether the subject-area cap of three is enforced at all.
Whether `generate_thumbnail` can carry a file.
What `data[Item][discount]` is for and what `discountprice` does on a non-bundle listing.
The bundle, Easel, Online Resource and Video create forms in their entirety.
The country vocabulary behind `country_id_flag`.
Any server-side counting unit for the 80-character title.
