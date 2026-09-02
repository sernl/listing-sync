# Creation flow: the canonical form on the TPT base

How a seller authors a product in the browser, on the fields TPT's own create form asks for, and chooses which marketplaces carry it.

This describes what exists in the tree.
Where something is declared and not yet wired, the section says so and names what is missing rather than describing an intention as a fact.

## 1. The shape

TPT is the canonical base model by founder decision, so the form is TPT's own nine sections in TPT's own order.
Name, Files, Description, Price, Categories, Education Standards, Details, Copyright, Product Status.
Education Standards is lifted out of Categories, where TPT nests it, because a jurisdiction picker is not a category picker and the seller reads the two differently.
Each section is a heading with its own helper text and its own refusals, so a message about the tax code appears under Price, which is where TPT puts the control.

The three layers are the pure model, the API that serves the form's vocabulary and decides what it refuses, and the client that renders it.

## 2. The model

`crates/tam-domain/src/product/` holds the canonical product as a pure type with no serde, no I/O and no dependency on the taxonomy crate, split along responsibility: `fields.rs` for the values the seller supplies, `vocabularies.rs` for the closed sets a control chooses among, `validation.rs` for what the whole product refuses, `canonical.rs` for the derivation, and `fixtures.rs` for the one product the three test modules share.
The vocabularies sit apart from the fields because they share one hazard: the wire id is the value and a menu position is not, so every member answers `wire_id` and `from_wire_id`, only `AnswerKey` answers `menu_index`, and nothing converts the other way.
`TptBaseProduct` is the source of truth for what a product is.
`CanonicalProduct` is derived from it by `TptBaseProduct::into_canonical` rather than being a second definition beside it, so the two cannot drift into disagreeing.
The derivation is total and takes a `ProductIdentity` for the three kinds of fact a form cannot state about itself: the row identity and the tenant, the `ProductFile` rows the upload produced, and the two values needing a relation this crate cannot reach — the canonical subject terms behind the subject-area slugs, and the age interval `tam_taxonomy::derive_interval` computes.
The tag facets, the format facets and the seller's own custom categories reach `native_residue` verbatim with no axis claimed for them, because residue is for values in axes this model does not type and a later upgrade moves them into a typed axis with no data migration.
`every_canonical_product_field_is_reachable_from_the_tpt_base_model` destructures the derived `CanonicalProduct` exhaustively, so a field added there stops the test compiling until the derivation carries it.

The storage side has not folded: `product` keeps its own table and migration 0040 adds a sidecar beside it.
Folding the two together is a scheduled step after the engine driver split, not a permanent shape.

The field list, group by group.
Name is `ProductName`, a smart constructor refusing a blank title and one over 80 UTF-16 code units, which is the unit `#ItemName`'s own `maxlength` counts in.
Files is `FileGroup`: a required payload handle, an optional preview, an optional video preview, a three-way `ThumbnailMode` and up to four thumbnail handles.
Description is `ListingCopy`, the existing type, carrying the body and its declared format.
Price is `PriceGroup`, either `Free` or `Paid(PaidPrice)`, where `PaidPrice` carries the price, the additional-licence price, an optional bundle discount and a `TaxCode`.
Categories is `CategoryGroup`: grades, subject areas, tags and formats as `FacetSlug` lists, plus the seller's own custom categories.
Standards is a list of `StandardAlignment`, each a `StandardsFramework`, the framework owner's published code, and TPT's node id where one is known.
Details is `DetailGroup`: an optional `TeachingDuration`, an optional page or slide count, an optional `AnswerKey`.
Copyright is an optional `CopyrightDeclaration`, and the option being absent is the point.
Status is `ListingStatus`, draft or live, which is a field on TPT rather than a route.

Four properties of the model are deliberate.

The selection caps are a parameter rather than a constant.
`SelectionCaps` arrives from the caller, the API reads it from `tam_taxonomy::TptForm`, and `TptForm` reads it from `docs/design/data/tpt-vocabulary.json`.
The dependency edge runs from `tam-taxonomy` to `tam-domain`, so the model cannot read the capture itself, and injecting the caps keeps the numbers in the capture rather than in a literal.
A cap that is absent is unmeasured rather than unlimited: the subject-area cap of three was contradicted by a create TPT accepted, so nothing is refused on it.

The three listbox vocabularies carry both a wire id and a menu position, because for Answer Key the two disagree.
TPT's menu runs N/A, Included, Not Included, Included with Rubric, Rubric Only, Does Not Apply, which is ids 0, 1, 2, 4, 5, 3.
Anything deriving an id from a menu position writes "Included with Rubric" as "Does Not Apply" and no seller can see that it happened.
The id is the value, the position is a function of it, and nothing converts the other way.

Nothing has a `Default`.
A defaulted tax code is a tax determination the seller is contractually answerable for under TPT's terms.
A defaulted copyright declaration is an attestation we made rather than they did, and TPT's own form arrives with value 1 pre-selected, which is exactly the thing ours must not do.

The additional-licence price is carried explicitly and never derived in a projection.
`suggested_additional_licence` computes the 90 percent pre-fill for the form and is called nowhere else, because TPT's help centre states the seller may choose any discount and recomputing it would overwrite their own figure on every sync.

`TptBaseProduct::check` returns every refusal and every advisory in one pass rather than stopping at the first, so a seller who left three controls wrong reads three messages.
The free-resource page guidance is an advisory and never a refusal: TPT states ten pages on a tooltip rather than as a validated bound, and a seller who breaches it should learn it here rather than from TPT after the write.

## 3. The API

`crates/tam-api/src/product/` serves three endpoints, all session-authenticated like every other read on this surface, split into `mod.rs` for the vocabulary, `check.rs` for the verdict and the sidecar's own input, and `standards.rs` for the jurisdiction search.
The route paths keep the segment `authoring` rather than `product`: `/{version}/products` already exists, and a singular `/{version}/product/...` beside it is a footgun for anyone reading a log line.

`GET /{version}/authoring/vocabulary` serves every controlled list the form renders, assembled from the compiled-in capture.
The twenty grade facets with the three buyer-side roll-ups marked unwritable, and the four column sizes that make the grid read down its bands.
The 133 visible subject areas, the 46 tag facets across theme, audience and language, and the 26 formats.
The five tax codes with the full descriptions TPT shows and the Avalara code carried separately.
The 23 teaching durations, the six answer keys in id order each carrying its menu position, the three thumbnail modes, the two copyright attestations quoted in full with their preamble, and the two status values.
The four standards jurisdictions with their button labels.
The measured caps, and the stated limits: the title cap, the description cap, the price floor in minor units, the additional-licence percentage, the free-resource page guidance and the four upload slots with their byte caps and extensions.

`POST /{version}/authoring/check` takes the draft as the form holds it and returns what the model refuses, each refusal naming the group whose heading holds it and the control's own label where the refusal is about one picker.
The client runs the same rules inline so a seller reads a message as they type; this endpoint is the authority, and a client that drifted still gets the same answer.

`GET /{version}/standards/search` takes a jurisdiction id and a query and returns either matches or the `not_ingested` state, with the framework's own attribution notice beside the results.
It returns `not_ingested` today for every framework, deliberately: the four catalogues and the code-to-node-id table TPT needs are a separate stream, and "nothing matched your words" and "nothing exists here yet" are different answers a seller acts on differently.

`GET /{version}/vocabulary/{inventory}` is unchanged and still serves one marketplace's own field table, which is what the destination tabs read.

The vocabulary generator emits two new unions into `web/src/lib/generated/vocab.ts`: `StandardsState`, and `FormGroup` with its `FORM_GROUPS` array, so the client's group vocabulary is the server's.

## 4. The form

`web/src/routes/listings/new/+page.svelte` renders the nine sections through `FormSection.svelte`, which carries the heading, the helper text and that section's own refusals.

Control by control, matching TPT and improving only where the improvement costs no structure.
Title is a text input with a hard cap and a live counter.
Files is the existing upload control plus the three-way thumbnail radio, whose four fixed slots appear only under "Upload thumbnails now", exactly as TPT's own conditional does.
Description is a textarea with a counter against the 45,000 cap.
Price is the Free Resource checkbox, which hides Price, Multiple Licenses, Bundle Discount Price and Tax Code when ticked, three separate money inputs rather than one price control with modifiers, and the tax code as a select of the five full descriptions with nothing pre-selected.
Categories is the grade grid, three capped multi-select pickers and the custom-category input.
Education Standards is a framework rail, a search box and the honest not-ingested panel.
Details is three controls in a row, the answer key ordered by id.
Copyright is a radio group with both attestations quoted in full, nothing pre-selected, and submission refused until one is chosen.
Product Status is the draft-or-live radio and the marketplace rail.

The grade grid is seventeen checkboxes in the four columns TPT's own grid reads down: primary grades, middle grades, high-school grades, then the three non-grade bands.
The arrangement is the segregation, so the column sizes come from the server and a re-polled vocabulary re-shapes the grid rather than overflowing one column.
The three roll-ups are served and named rather than omitted, so the form can say why a browse band has no checkbox.

Four things this form does that TPT does not, none of which changes the segregation.
Every capped picker carries a live counter reading "2 of 4", and at the cap the remaining options are disabled rather than left to be refused on submit.
Where no cap is measured the counter reads a plain count, because a counter against a number nobody holds would be an invented ceiling.
The three long pickers are searchable over both the label and the slug, which matters at 133 members.
Every refusal names its control and links to the section that holds it.

`web/src/lib/tpt-form.ts` is the pure client model: the draft, the counters, the cap enforcement, the local refusals, the per-marketplace projection and the two request bodies.
It is a mirror of the server's rules and never the authority; `POST /v1/authoring/check` runs on submit and its answer is shown alongside.

## 5. The per-marketplace tabs

The destination rail runs across the top: the canonical listing first, then one tab per selected marketplace, each marked where one of its values differs.

A marketplace tab shows that platform's value for the three fields every target carries — title, description and price — plus its disclosed losses, its attestation and its price floor read off `GET /vocabulary/{inventory}`.
A value follows the canonical one until the seller edits it.
Once it differs, "Update all" appears on the marketplace tab and promotes that value to the canonical one while clearing every other override of that field, and "Reset" drops the override so the field follows the canonical value again rather than being emptied.
Neither fires on its own, which is the Vendoo affordance the memo adopts.

What the API lacks for this, precisely.
There is no endpoint that renders one product's projection onto one marketplace field by field.
`GET /{version}/vocabulary/{inventory}` serves the registry's field table and `GET /{version}/mappings` serves mapping heads with their recorded losses, and neither answers "what will this listing's title be on Tes".
So the divergence above is computed client-side from the seller's own overrides rather than from a server-side diff, and the tab shows three fields rather than every field a platform carries.
Closing that needs a projection endpoint taking a draft or a product and an inventory and returning the per-field projected value with its loss, which would also let the tab show what a platform drops before the create rather than after it.

The client contract is shaped so that endpoint can feed it without a rewrite.
`MarketplaceProjection` is a list of `ProjectedRow`, each naming a canonical field or an equivalence axis, its values as a list, what decided it, and the loss the projection recorded.
`DecidedBy` has four cases, of which the client produces two: `listing` and `listing_override` are this listing's own doing, while `relation` and `override` belong to the projection.
The second pair is exactly the `ResolvedBy` discrimination the per-organisation override slice designs at the end of `docs/notes/mapping/tpt-base-residue.md`, down to carrying the instant the standing decision was taken.
Values are a list rather than a scalar because an axis resolves to a set: a term can broaden onto several of a platform's own values, and an override resolving to an empty set means "drop this term for me", which is a different thing from a term nobody has decided.
When the endpoint lands, `projectionOf` stops constructing rows and the tab renders what the server sent.

## 6. Storage

`crates/tam-storage/migrations/0040_product_tpt_base.sql` declares the sidecar: one row per product, keyed on the product's own key, holding the thumbnail decision and its handles, the additional-licence and bundle-discount amounts, the tax code, the three picker arrays and the custom categories, the standards as jsonb, the three detail fields, the copyright declaration and the status.
Row-level security keyed on the tenant, matching every other tenant table, and `rls_matrix.rs` classifies it as one so the closed-world test keeps holding.
The CHECK constraints cover the closed wire vocabularies and the four thumbnail slots.
The cardinality caps are deliberately not constraints, because they are measured claims about a form and one of them has already been contradicted, so freezing them into DDL would make a re-poll a migration.

`crates/tam-storage/src/tpt_base.rs` is the repository: an upsert and a read, both pinning the tenant.
The upsert replaces the row rather than merging into it, because a control the seller cleared is cleared and a merge would make clearing one impossible to express.
The wire ids of the closed listbox vocabularies are decoded back through the domain's own `from_wire_id`, so a stored value outside the set names itself as corruption rather than arriving as a plausible neighbour.
The grades are not duplicated here: they live on `product.grades` as the verbatim declaration every other reader already uses.

`POST /{version}/products` and `PATCH /{version}/products/{id}` take an optional `tpt_base` block carrying only what `product` has no column for, so the title, description, price, payload handles and grades have one source rather than two copies that can disagree.
The create validates the assembled draft through the same `verdict` function `POST /{version}/authoring/check` answers with, so the endpoint that reports a refusal and the endpoint that acts on one cannot come to different conclusions; a create carrying no attestation is refused by name rather than writing a product nobody attested to.
A create with no block writes no row, which is what an operator import does, and is why every read of the table is an outer join.

Folding the sidecar into `product` is a scheduled step after the engine driver split, not a permanent shape.

## 7. What the form does not do

Bulk create from a spreadsheet; the form is one product.
Listing-copy generation, the one place a model is admitted by the decision record.
Per-slot thumbnail upload: the four slots render and the thumbnail mode is stored, but `POST /{version}/uploads` takes one file per request with no slot to name it, so no thumbnail bytes are collected and the form says so.
Per-inventory price overrides beyond the three overridable fields.
Bundles, Easel, Online Resource and Video, which stay out until each is captured.
Etsy, which has no adapter.
