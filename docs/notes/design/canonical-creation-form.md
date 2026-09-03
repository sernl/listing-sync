# Phase 5: the canonical creation form, control by control

The implementation design for Phase 5 of the re-baselined plan (`docs/notes/design/vendoo-for-teachers-rethink.md:406-410`), rebuilding the canonical creation form on the TPT DOM snapshot read at `docs/research/rethink/tpt-create-form-dom.md`.

## What this phase actually is

Phase 5 is a completion and verification pass, not a greenfield build, and an implementer who treats it as the latter will rewrite working code.
The nine-section form exists at `web/src/routes/inventory/new/+page.svelte` (770 lines), its pure client model at `web/src/lib/tpt-form.ts` (667 lines), the canonical product at `crates/tam-domain/src/product/` (1712 lines across six files), the three authoring endpoints at `crates/tam-api/src/product/`, and the sidecar at `crates/tam-storage/migrations/0040_product_tpt_base.sql`.
`docs/notes/design/creation-flow.md` is the design that produced them and remains accurate on the model, the API and the storage.

Three of its statements have since been overtaken and an implementer reading it first will be misled on each.
It names the route `web/src/routes/listings/new/+page.svelte`, which is now a one-line redirect stub; the live route is `web/src/routes/inventory/new/+page.svelte` and `web/src/routes/listings/+page.ts:7-9` performs the 308.
It states that no projection exists, but `web/src/lib/tpt-form.ts:466-491` now calls a WASM core through `web/src/lib/core/index.ts`, which returns per-field declared losses and an `undecided_axes` list (`web/src/lib/core/core.test.ts`).
It describes the Vendoo rename as pending, and the routes, the navigation entries (`web/src/lib/nav.ts:31-34`) and the redirect table (`web/src/lib/nav.ts:163`) have all landed.

So the work is the field table below, the one control the model cannot express, the axis half of the per-marketplace preview, and four naming residues.

## The 48 field paths

This table is the phase's verification and the source of its kill gate.
The paths are `data[_Token][unlocked]` decoded, verbatim and in order, from `docs/research/rethink/tpt-product-model.md:96-113`, confirmed byte-identical to the DOM snapshot at `docs/research/rethink/tpt-create-form-dom.md:17`.
Standing is one of three: expressed today, needs a new canonical field, or omitted with its reason.

| TPT path | Canonical field | Standing |
|---|---|---|
| `Category` | — | Omitted: a CakePHP container path, not a field |
| `Category.Category` | `CategoryGroup.custom_categories` (`fields.rs:241`) | Expressed, with a stated gap: TPT takes integer ids and we hold seller strings, and no option set exists to seed the binding |
| `Item.description` | `TptBaseProduct.description` (`mod.rs:72`) | Expressed |
| `Item.discount` | — | Omitted: no control anywhere in the form and its meaning is unknown (gap 9, `tpt-create-form-dom.md:143`) |
| `Item.discountprice` | `PaidPrice.bundle_discount` (`fields.rs:154`) | Expressed |
| `Item.error` | — | Omitted: PHP `$_FILES` transport shape |
| `Item.free` | `PriceGroup::Free` (`fields.rs:145`) | Expressed |
| `Item.generate_thumbnail` | `FileGroup.thumbnail_mode` (`fields.rs:128`) | Expressed |
| `Item.generate_thumbnail.error` | — | Omitted: `$_FILES` quadruple; whether the field accepts a file is gap 4 and still open |
| `Item.generate_thumbnail.size` | — | Omitted: same quadruple |
| `Item.generate_thumbnail.tmp_name` | — | Omitted: same quadruple |
| `Item.generate_thumbnail.type` | — | Omitted: same quadruple |
| `Item.license_price` | `PaidPrice.additional_licence` (`fields.rs:153`) | Expressed |
| `Item.name` | `ProductName` (`fields.rs:35`) | Expressed |
| `Item.price` | `PaidPrice.price` (`fields.rs:152`) | Expressed |
| `Item.size` | — | Omitted: `$_FILES` transport shape |
| `Item.status_user` | `ListingStatus` (`mod.rs:81`) | Expressed |
| `Item.tmp_name` | — | Omitted: `$_FILES` transport shape |
| `Item.type` | — | Omitted: `$_FILES` transport shape |
| `ItemDigital.preview` | `FileGroup.preview` (`fields.rs:125`) | Expressed |
| `ItemDigital.product` | `FileGroup.payload` (`fields.rs:123`) | Expressed |
| `ItemDigital.thumb1` | `FileGroup.thumbnails[0]` (`fields.rs:131`) | Expressed in the model; the form renders the slot and collects no bytes (`tpt-form.ts:672-678`) |
| `ItemDigital.thumb2` | `FileGroup.thumbnails[1]` | Expressed, same caveat |
| `ItemDigital.thumb3` | `FileGroup.thumbnails[2]` | Expressed, same caveat |
| `ItemDigital.thumb4` | `FileGroup.thumbnails[3]` | Expressed, same caveat |
| `ItemTaxCode.tax_code_id` | `PaidPrice.tax_code` (`fields.rs:155`) | Expressed |
| `ItemsBundlesProperty.title` | — | Omitted: bundles are out of scope and no bundle form has been captured (gap 7) |
| `ItemsCommonCoreStandard.common_core_standard_id` | `StandardAlignment.tpt_node_id` (`fields.rs:264`) | Expressed |
| `ItemsCommonCoreStandard.common_core_standards_num` | — | Omitted: derived at projection as the length of the id list, never authored |
| `ItemsLocalization.country_id_flag` | `CategoryGroup.appropriate_for_country` (`fields.rs:249`) | Expressed: added by this phase, with the label served nullable because the country vocabulary behind it is still unmeasured |
| `ItemsProperty.answer_key` | `DetailGroup.answer_key` (`fields.rs:273`) | Expressed |
| `ItemsProperty.copyright_declaration` | `TptBaseProduct.copyright` (`mod.rs:80`) | Expressed |
| `ItemsProperty.duration` | `DetailGroup.teaching_duration` (`fields.rs:270`) | Expressed |
| `ItemsProperty.pages` | `DetailGroup.pages_or_slides` (`fields.rs:272`) | Expressed |
| `ItemsProperty.preview_uploaded` | — | Omitted: derived flag, set from whether the handle is present |
| `ItemsProperty.product_uploaded` | — | Omitted: derived flag |
| `ItemsProperty.thumb1_uploaded` | — | Omitted: derived flag |
| `ItemsProperty.thumb2_uploaded` | — | Omitted: derived flag |
| `ItemsProperty.thumb3_uploaded` | — | Omitted: derived flag |
| `ItemsProperty.thumb4_uploaded` | — | Omitted: derived flag |
| `ItemsVideoProperty.video_type_text` | — | Omitted: video products only, and no video form has been captured (gap 11) |
| `RevisedItem.comment` | — | Omitted: no control anywhere in the form (gap 14, `tpt-create-form-dom.md:149`) |
| `RevisedItem.is_post` | — | Omitted: revision marker, posted empty in both captures |
| `TaxonomyTags` | `CategoryGroup.grades`, `.subject_areas`, `.tags`, `.formats` (`fields.rs:235-238`) | Expressed: four pickers serialise into one flat slug array |
| `Upload.custom_videopreview_uploaded` | — | Omitted: derived flag |
| `Upload.videopreview` | `FileGroup.video_preview` (`fields.rs:127`) | Expressed |
| `thumbs` | — | Omitted: transport artefact of TPT's own auto-generation branch |
| `thumbs_collection_key` | — | Omitted: transport artefact of the same branch |

Twenty-four paths are expressed and twenty-four are deliberate omissions, which sums to 48 with no row left over.
Every expressed row names the field that holds it and every omitted row names why, so the table is closed: this is the phase's kill gate and it passes.
The line references were re-read after the field landed, because the earlier ones had shifted.

## The kill gate

The gate is a control the snapshot shows that the canonical model cannot express, and there is exactly one.
`data[ItemsLocalization][country_id_flag]` is a real seller-facing checkbox in the Categories section, labelled "Appropriate for New Zealand" for this seller, sitting after the five pickers and the standards subsection (`tpt-create-form-dom.md:64` and `:120`).
A grep for country or localisation across `crates/tam-domain/src/product/` and across `web/src/lib/tpt-form.ts` returned nothing when this note was written, so neither the model nor the form held it.
The field landed in step two and the record at the end of this note says where.

This does not fail the phase, and the reason matters.
The gate asks whether the model *cannot* express a control, and a boolean on `CategoryGroup` expresses this one at the cost of one field and one migration column.
What is genuinely unknown is the country vocabulary behind it, which is gap 13 and still open (`tpt-create-form-dom.md:148`): the label is derived from the seller's own country, no country list appears in the DOM, and we hold no way to render the label for a seller whose country we have not observed.
So the field is added as a boolean whose label the server supplies, and the label's source is a founder decision below.

## The form's structure

Nine sections in TPT's own order, Education Standards lifted out of Categories because a jurisdiction picker is not a category picker, which is what `FormGroup` already declares (`crates/tam-domain/src/product/validation.rs:151-161`).
The control per field follows the snapshot's own reading at `tpt-create-form-dom.md:199-227` and is already implemented; this section states only what changes.

Categories gains the localisation checkbox as its last control, so it closes the section.
TPT renders it after the standards subsection because TPT nests standards inside Categories; this form lifts Education Standards into a section of its own, so the checkbox closes Categories and the standards section follows it rather than preceding it.
The field belongs to the Categories group in the model as well as on the page, which is what settles the order.
Its label is served rather than hard-coded, so a seller outside New Zealand does not read another country's name.

The per-marketplace preview is the destination rail across the top: the canonical listing first, then one tab per selected marketplace.
Today a tab renders three field rows — title, description, price — each carrying the declared loss the WASM core computed (`tpt-form.ts:466-491`), and `OVERRIDABLE` (`tpt-form.ts:366-371`) is deliberately those three because a field one platform lacks is a disclosed loss rather than an override.
The missing half is axis rows, and the core already returns `undecided_axes`, so the tab renders one row per axis carrying its resolution mode.
The three modes come from the founder direction on mapping equivalence and are rendered distinctly: a clean high-confidence equivalence resolves silently and shows its resolved values; a best-fit suggestion shows the suggestion marked as ours and is inert until the seller opts in; and a seller-decides axis shows no value at all and offers the queue entry point, because anything not clearly derivable is the seller's to settle.
Licence is the exemplar and the one axis that is never delegable: TPT has no licence field, Tes requires one, and no TPT field derives which Creative Commons value a free resource deserves, so the axis is refused an override in both the domain constructor and a database CHECK (`docs/notes/design/tpt-vocabulary-rebase.md:66`).
Multi-value axes render their values as a set rather than a scalar, which `ProjectedRow.values` is already shaped for (`tpt-form.ts:434-444`), and an unresolved element of a set enqueues individually.
Caps a target declares and this listing's set exceeds are shown on the row as the loss, not discovered after the write.

There are two seller override entry points and they are different things.
The per-listing override is the marketplace tab itself: editing a value there writes `overrides` keyed `inventory:field` (`tpt-form.ts:359-361`), whereupon "Update all" promotes it to canonical and clears every other override of that field, and "Reset" drops it so the field follows canonical again rather than being emptied (`tpt-form.ts:396-406`).
The standing override is per-organisation and belongs on the Templates screen rather than here, keyed `(org_id, inventory, axis, from_term)` in `crates/tam-storage/migrations/0048_projection_override.sql`, and the form links to it from an axis row rather than editing it inline.
Keeping the two apart is why `DecidedBy` has four cases (`tpt-form.ts:420-425`): `listing` and `listing_override` are this listing's doing, `relation` and `override` are the projection's.

One warning an implementer must carry: nothing calls the override layer yet.
`crates/tam-engine/src/seed.rs` and `crates/tam-import/src/lib.rs` both project with an empty override set, so a standing override is durable, tenant-isolated, tested and inert until a caller switches to `project_listing_with_overrides` (`docs/notes/design/tpt-vocabulary-rebase.md:282-285`).
Linking to a control that silently does nothing is worse than not linking to it, so the axis row states the standing override's current effect honestly or does not offer it.

## Validation and the totality rule

The totality rule is that every marketplace-required field is either canonical or has a stated default, and no third case is permitted.
`TptBaseProduct::check` returns every refusal and every advisory in one pass rather than stopping at the first, so a seller who left three controls wrong reads three messages (`crates/tam-domain/src/product/validation.rs:86-146`).
The client mirrors the rules inline through the same compiled core so a message appears as the seller types, and `POST /{version}/authoring/check` is the authority, which is why a drifted client still gets the same answer (`web/src/routes/inventory/new/+page.svelte:159-166`).

Three fields are required by TPT and deliberately have no default, and the reason is the same in each case: a default would make our statement out of the seller's.
The tax code is a tax determination the seller is contractually answerable for.
The copyright declaration is a legal attestation, and TPT's own form pre-selects value `1` on a blank form, which is precisely the behaviour ours must not copy (`tpt-create-form-dom.md:135`).
The title has no default because a blank title is not a product.
Every other required field either has a canonical value or is refused by name, and `every_canonical_product_field_is_reachable_from_the_tpt_base_model` destructures the derived product exhaustively so a field added without a derivation stops the build.

Caps are a parameter and never a constant (`crates/tam-domain/src/product/mod.rs:23-30`), and an absent cap is unmeasured rather than unlimited, so nothing is refused on the subject-area cap of three that a real create already contradicted.
The free-resource page guidance stays an advisory and never a refusal, because TPT states it on a tooltip rather than validating it.

The localisation checkbox is not required by TPT and takes `false` as its stated default, which is what an unticked checkbox posts.

## How the form feeds the create job

The wire does not change, and this is the constraint the phase is built to respect.
`createBodyOf` (`web/src/lib/tpt-form.ts:637-656`) assembles `POST /{version}/products` from the draft: the title, body, price, payload, cover, previews, grades and inventories on the body itself, and everything `product` has no column for inside the optional `tpt_base` block that migration 0040's sidecar stores.
The new localisation boolean travels inside `tpt_base` alongside the thumbnail decision, the tax code and the detail fields, so no existing body field moves and no endpoint gains a parameter.
The create validates the assembled draft through the same verdict function `POST /{version}/authoring/check` answers with, so the endpoint that reports a refusal and the endpoint that acts on one cannot disagree.
A create carrying no attestation is refused by name rather than writing a product nobody attested to.

The engine job downstream is untouched: the create already produces one mapping per selected inventory and the console navigates to the product (`web/src/routes/inventory/new/+page.svelte:167-176`).

## Console naming

The Vendoo cross-reference rename map (`docs/research/rethink/vendoo-console-cross-reference.md:200-230`) is substantially applied: the routes, the navigation labels and the redirect stubs have all landed.
Four residues remain and only the first is on this form.

The page title reads "New listing" at `web/src/routes/inventory/new/+page.svelte:212` and the rename map says the seller-facing noun is item, so it becomes "New item"; the description one line below already says item, which is what makes the title read as an oversight rather than a choice.
Elsewhere in that file the word resource survives only inside TPT's own control label "Free Resource" and its tooltip (`:361`, `:365`, `:427-428`), and those must stay verbatim, because a control quoted from TPT that we have renamed is a control the seller cannot find on TPT.
The stub at `web/src/routes/listings/new/+page.svelte` already says "new item form", so the vocabulary is settled and only the heading disagrees with it.

## Build order

Four steps, each landable on its own, each with the verification that would fail it if it were wrong.

Step one, the naming residue, owned by the console alone.
Change the page title at `web/src/routes/inventory/new/+page.svelte:212` to "New item" and leave every "Free Resource" string untouched.
Verification is `just web-check`, which runs svelte-check and vitest; it fails on a broken template and the existing form tests fail if a quoted TPT label was renamed with it.

Step two, the canonical field, owned by `crates/tam-domain` and `crates/tam-storage`.
Add a boolean to `CategoryGroup` (`crates/tam-domain/src/product/fields.rs:233-241`), carry it through `into_canonical`, and add its column to the sidecar in a new migration rather than editing 0040.
Verification is `every_canonical_product_field_is_reachable_from_the_tpt_base_model`, which stops compiling the moment a field is added to the derived product without the derivation carrying it, plus `just db-test` for the two-tenant isolation on the widened row.
This is severe: a derivation that dropped the field would not compile, and a migration that missed the RLS policy would fail the isolation test rather than passing quietly.

Step three, the control and its served label, owned by the console and `crates/tam-api`.
Serve the label and its default from `GET /{version}/authoring/vocabulary` (`crates/tam-api/src/product/mod.rs`), render the checkbox as the last control in Categories, and carry the value in `TptDraft` (`web/src/lib/tpt-form.ts:73-107`) and in `tptBaseOf`.
Verification is a vitest case asserting that `createBodyOf` puts the value in `tpt_base` and nowhere else, which fails under any implementation that promoted it to a body field and so changed the wire.

Step four, the axis rows on the marketplace tab, owned by the console and the projection.
Render one row per entry in the core's `undecided_axes` with its resolution mode, its values as a set, its declared cap loss, and the standing-override link where the axis is delegable.
Verification is a vitest case over a draft whose subject set exceeds a target's cap, asserting the row reports the loss and reports no narrowed value; it fails under any implementation that truncates the set silently, which is the same property `docs/notes/design/tpt-vocabulary-rebase.md:219` holds server-side.
Licence must render as seller-decides with no suggestion and no override control, and a test asserting the licence row offers no override is what keeps the two-layer refusal honest at the third layer.

## Two contradictions found while reading

Both are recorded rather than resolved, because resolving either is a research act and this phase is a build.

The DOM snapshot states at `docs/research/rethink/tpt-create-form-dom.md:149` that `ItemsProperty.audience` is "unlocked but unreachable", and the decoded 48-path list at `docs/research/rethink/tpt-product-model.md:96-113` does not contain that path at all.
The list is the stronger evidence, being a verbatim decode confirmed identical across two captures four days apart, so the correct reading is that audience is not client-writable on this route by any means, and the snapshot's wording overstates it.
Nothing in this design depends on the difference, but the field table above omits the path because it is not one of the 48.

The doc comment above `FormGroup` at `crates/tam-domain/src/product/validation.rs:148-149` says "the eight headings" while the enum has nine variants and `ALL` is declared `[Self; 9]`.
The code is right and the comment is stale by one.

## Founder decisions this design needs

1. Should the localisation checkbox be added as a plain boolean with a server-supplied label, given that the country vocabulary behind it is unmeasured? Recommended yes: the boolean is what the wire carries, the label is presentation, and waiting for the vocabulary blocks the only field standing between us and a complete 48.
2. Should the label be derived from the connected TPT account's own country, or from the organisation's country in our own settings? Recommended the connected account's country, because TPT derives it that way and a mismatch would show a seller a country TPT will not honour.
3. Should the axis rows ship in Phase 5, or wait until the override layer has a caller and stops being inert? Recommended ship them in Phase 5 reading only, with the standing-override link withheld until a caller exists, because the preview is the largest ease gain available and a link to an inert control is the only part that would mislead.
4. Should the four thumbnail slots stay rendered-but-uncollected, or be hidden until `POST /{version}/uploads` gains a slot parameter? Recommended keep them rendered with the stated caveat already at `web/src/lib/tpt-form.ts:660-666`, because the slots are how a seller reads TPT's own thumbnail model and hiding them would make our form the less legible of the two.

## What landed

Written as the phase was built, one entry per step, so the record is what happened rather than what was planned.

Step one changed the page title at `web/src/routes/inventory/new/+page.svelte` from "New listing" to "New item" and left every quoted TPT string untouched, which `just web-check` confirmed at 1195 files with no errors, 425 vitest cases and a clean build.

Steps two and three landed as one commit, because the field and the control are the same thing and a commit holding only half of it would carry a column nothing writes.
`CategoryGroup` gained `appropriate_for_country: bool`; migration 0049 added the matching column as `boolean NOT NULL DEFAULT false`, with no CHECK, because a boolean column already holds exactly the control's two members and TPT marks the control optional.
`GET /{version}/authoring/vocabulary` gained a `LocalisationView` carrying a nullable `label` and a `generic_label`, the checkbox renders as the last control in Categories, and the value travels in `tpt_base` and nowhere else on the create body.
The stale doc comments this note found — "the eight headings" in `validation.rs` and "the eight-group form" in the API's `FormVocabularyView` — are both corrected against a nine-variant `FormGroup`.

Step four added the axis half of the marketplace tab.
Each axis the compiled core could not decide is joined to what `GET /{version}/vocabulary/{inventory}` already declares about it, so the tab needed no new endpoint and no new client method: the page was already fetching that view per selected marketplace.
An axis row carries the mode, whether the axis may ever be delegated, the platform field it lands in, the seller's own terms and the cap those terms may exceed, and it carries no value at all, because the equivalence relation lives in Postgres and a value chosen in the browser would be a mapping nobody recorded.
A one-valued axis is read as a cap of one rather than as a separate case, which is what stops a set arriving silently as its first element.
Per decision 3 no standing-override link is offered on any row, because the override layer still has no caller and the note's own rule is that linking to an inert control is worse than not linking.

Step five made the adapter post the flag instead of a constant, and fixed a live defect while doing it.
`TptListing` carries `Option<bool>`, where `None` is a projection that states nothing and is deliberately not `Some(false)`: the create posts the carried value and treats `None` as unticked, while the edit posts the carried value and falls back on `None` to what the product's own `UploadPageProductQuery` read-back returned.
That fallback is the fix rather than a nicety. An edit is a full replace, `crates/tam-marketplace-tpt/src/flows.rs` posted a constant `0` on every edit and publish, and the captured edit posted `1` because that seller has the box ticked — so every revise of such a product silently cleared it.
The read-back costs one extra request per edit and is skipped the moment a projection carries a value, so the queued plumbing above repays it.
A failed read propagates rather than defaulting, because not knowing the current state and posting `0` anyway is the same silent clear; a read that succeeds and carries no localisation object is a measured absence and posts unticked.
The form render was not scraped for this instead, though it would have cost no request: `tpt-create-form-dom.md:132` records that this control's wire integers are produced by JavaScript rather than by the markup, and a wrong guess here is exactly the clear being fixed.

Step six made the sidecar able to say "not stated", which is the one thing standing between the stored flag and the projection.
Migration 0050 drops the column's `NOT NULL` and its default and sets every existing row to `NULL`, and its comment records why nothing is lost by that: 0049 backfilled `false` into every row that predated it, so no value the column has ever held was a seller's answer.
The `UPDATE` runs with the tenant fence lifted for that one statement and restored three lines later, because `product_tpt_base` carries `FORCE ROW LEVEL SECURITY`, a migration connects as `tam_app` with no `app.current_org` set, and the statement would otherwise match no row, report nothing, and leave the whole backfill in place.
`CategoryGroup.appropriate_for_country` is an `Option<bool>` whose `None` means the seller has not answered, and `TptBaseInput` and `DraftInput` carry the same option with absent reading as `None` rather than as `false`.
The console keeps a plain boolean and the two agree: the wire's absent case is a product nobody asked, a seller looking at the control has been asked, so a saved form states the checkbox's own answer whichever way it is ticked.
The round trip at `crates/tam-storage/tests/tpt_base.rs` now walks the flag through `Some(true)`, `Some(false)` and `None` inside the whole-record assertion, which is what fails if a `None` comes back as `false`, and a vitest case asserts both answers are stated on save.
One statement above is superseded by this step: the validation section's "takes `false` as its stated default" is now true of the console's control alone and not of the model, which defaults to stating nothing.

Three corrections to this design, found while building it.

Step two's stated verification was not severe for the change it verifies.
`every_canonical_product_field_is_reachable_from_the_tpt_base_model` destructures `CanonicalProduct`, and the localisation flag has no `CanonicalProduct` field, so once the fixture literal is filled that test passes whether or not the derivation, the column or either query carries the value.
The severe check is `crates/tam-storage/tests/tpt_base.rs::every_field_the_form_collects_survives_the_round_trip`, which fails unless the column, the insert and the select all carry it; its fixture sets the flag true rather than false, because false would have matched the column's own default and passed on a value that never reached the row.

The structure section above contradicted the build order on where the checkbox goes, and the build order won.
TPT renders the control after the standards it nests inside Categories, and this form lifts Education Standards into a section of its own, so the two orders cannot both hold; the field belongs to the Categories group in the model, so the control closes Categories and the standards section follows it.

`docs/notes/design/creation-flow.md` was corrected on the route it names and on the losses it describes as absent.
It stated one further staleness this note predicted — the Vendoo rename described as pending — that no sentence in it actually makes; the stale route path was the whole of that defect.

Four gaps this phase did not close, each stated rather than resolved. One has since been closed and is kept here with its reason, because the reason is the part worth having.

The country behind the localisation label is still unmeasured, and the label is served as null for every seller with a generic sentence rendered in its place.
Closing it means observing the country on the adapter's own read, which already fetches `localization { countryId countryIdFlag country { name } }`, and storing it on the connection, which needs the adapter crate and a device-to-server report; that is a separate design rather than a step here.

Closed. The seam carries the flag, the sidecar can say "not stated", and `seed.rs` now reads the one into the other.
`FieldSet` gained a typed `appropriate_for_country: Option<bool>` beside `body_format`, `ProjectedListing` gained the same, `project_fields` puts a stated value in and `listing_from_field_set` takes it back out, so a value that reaches the projection reaches the posted `country_id_flag` and the recorded intent carries it too.
What does not happen is `crates/tam-engine/src/seed.rs` reading the sidecar, and it was built and then deliberately removed rather than never attempted.
The cause was that the sidecar could not say "not stated", and step six removed it: the column is nullable, the domain field is an `Option<bool>`, and a row written before either reads back as `None` rather than as a seller's deliberate "no".
That distinction is the one `post_edit` turns on — `if listing.appropriate_for_country.is_none()` is the whole guard on the protective read-back — so a `None` fed from the sidecar defers to what TPT itself holds and only a value the seller stated overrides it.
`seed.rs` reads the sidecar at the one production site, gated on the inventory being TPT so a Tes item costs no query, and the read is `and_then` rather than `map` for exactly the reason above: no row and a row stating nothing are both "not stated", and only a row that states a value may answer.
The engine test asserts four cases — no row, a stored `true`, a row stating nothing, and a Tes item reading none with a row present — and the third is the one the whole detour was about.

Building the whole TPT-base record from the upload-page read on import is queued separately, and it is worth doing for a reason that is not the obvious one.
The obvious reason does not hold: a box ticked on TPT before adoption already survives the first edit, because `post_edit` reads the current flag back whenever the projection states nothing.
The real reason is the opposite direction. Once a product has a sidecar row our value wins on edit, so a seller who ticks the box on TPT after creating the item here sees our stored choice reposted over it, and the only way that stored choice is right is if we adopted what TPT held at import.
It costs no extra request: `fetch_for_import` already issues `upload_page_product_request` and `parse_upload_page_product` already returns the flag, and `ImportedListing` simply does not carry it.

The DOM snapshot states at `docs/research/rethink/tpt-create-form-dom.md:149` that `ItemsProperty.audience` is "unlocked but unreachable", and the decoded 48-path list does not contain that path at all.
The list is the stronger evidence, being a verbatim decode confirmed identical across two captures four days apart, so the snapshot's wording overstates it; nothing in this design depends on the difference and the field table omits the path because it is not one of the 48.
Both statements are recorded here as they stand, because resolving either is a research act and this phase was a build.
