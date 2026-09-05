# Form templates: the backend half, and where its data came from

Working note for the 2026-09-05 amendment recorded as "Best fit, pre-ticked on each marketplace tab" in `../../design/decisions.md` and as D33 in `vendoo-for-teachers-rethink.md`.
It records the provenance of the three data additions, which those two are too terse to carry, and the two places the implementation is narrower than the design.

## Where the labels and the placements came from

A native field now carries the words the platform's own form heads it with, and the section of that form which holds it.
Both are optional, and absent means no capture recorded one rather than a hole to fill.
The rule is the registry's existing admission rule applied to a field instead of to a value: where the uploader's own words were captured they are used, and where they were not the wire name stands in for itself.

Tes's placements are the five-step uploader wizard.
`../probes/02-upload-request-shape.md` names the steps — Description, Add Files, Categories, Licence, Publish — from the 2026-08-25 capture.
Tes's own author academy, quoted at `../../research/rethink/cross-marketplace-mapping-tpt-base.md:39`, says what each step holds: "title and description, file upload and resource type, tag and categorise, price or licence, preview and agree to the Author Code".
That sentence is what places `mainType` on the Add Files step and the category fields on Categories.
Tes's labels come from `../probes/05-uploader-vocabulary.md`, which names the controls in prose while cataloguing their options: Resource type, Main age range, Curriculum, Subjects and topics.
`yearGroups`, `mainAge`, `descriptionRawType` and `primaryCategory` carry no label, because no capture names a control for them; `primaryCategory` in particular has no control at all, since the publish request derives it from the categories the seller chose.

TPT's labels and placements come from `../../research/rethink/tpt-create-form-dom.md`, a full-page DOM snapshot of `GET /My-Products/New/Digital-Next` read on 2026-09-03, which records every section heading and every control label the create form renders.
The nine sections are `FormGroup`, which already existed; `FieldGroup::Tpt` carries that type rather than restating its values, so a section added there is a compile error in the registry instead of a second list to keep in step.
Four TPT fields carry no placement: the three read-only fields the form has no control for, and the two opaque server-issued upload handles.
`taxonomyTags` carries a placement and no label, because five separate pickers feed it and the snapshot's own finding 8 is that the form carries no native input for the array at all.

Etsy carries neither, because its entries come from a published API reference and no form of its has been captured.
The completeness test states that exemption as a derived condition rather than by name: an inventory that places no field at all is skipped, and the moment one of its fields is placed its required fields are held to the same bar.

## Where the implementation is narrower than the design

Two narrowings, both deliberate refusals to rank a set that is not the one the question was asked about.

`GET /{version}/elections/items` rebuilds an `elect_one` and an `over_cap` trigger and declines the other two.
A `supply` has no resolved set by construction, so best fit declines on it in the domain as well.
A `narrow` asks which values under one source band this listing means, and the durable row keys on the band's own native id rather than on the term it came from, so the band's candidates cannot be named from the row alone.
`best_fit` itself handles all four and is tested on all four; the narrowing is a property of what a durable row records, not of the ranking.

The resolved set that route ranks is recomputed by `project_terms` over the product's own canonical terms, and does not consult the seller's projection overrides.
An override only ever adds to a resolved set, so a set computed without them is a subset of what the projection resolves — short of the whole answer, never outside it — and best fit's own invariant, that it never names a value the resolved set did not hold, survives.
Consulting them would cost a further read per product on a route that already reads one.
