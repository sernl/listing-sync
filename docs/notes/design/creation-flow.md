# Creation flow: an add-product form, a publish control, and CRUD

How a seller could author a product in the browser, choose which marketplaces carry it, and manage it afterwards.

This is a design note, not a decision.
Nothing below is built, and the investigation behind it was read-only.
No marketplace was contacted; every claim rests on this tree, cited with line numbers.
Where the tree is ambiguous the note says so rather than resolving it, and everything under "Founder questions" is a proposal for review.

## 1. What exists and what is missing

Products are created in exactly one place.
`import_one` reads a marketplace listing under the first-party-export capability, ingests file bytes, writes a product and one target mapping, and projects once (`crates/tam-import/src/lib.rs:422`, `:530`, `:558`, `:588`).
Its two callers are the operator binary (`crates/tam-import/src/main.rs`) and the sync drain (`crates/tam-sync-worker/src/lib.rs:247`).
Both start from a listing that already exists on some marketplace, so there is no path by which a seller authors a product that was never listed anywhere.

The storage layer offers no other path either.
`ProductRepo` has `insert`, `get`, `list` and `list_page`, and no update or delete (`crates/tam-storage/src/product.rs:53`, `:155`, `:284`, `:707`).
`MappingRepo` has `insert`, `get`, `list_for_product` and `list_heads`, and no update (`crates/tam-storage/src/mapping.rs:66`, `:194`, `:228`, `:940`).

The API surface is read-only over the catalogue.
`/{v}/products` and `/{v}/products/{product}` are both GET (`crates/tam-api/src/lib.rs:147`, `:148-151`); there is no POST, PATCH or DELETE for a product, no upload route, and no route serving a marketplace's vocabulary.
No handler in `tam-api` reads multipart, and no `DefaultBodyLimit` is applied anywhere in the crate, so axum's 2 MiB default stands.
`ProductView` carries title, body, price, files, subjects and grades (`crates/tam-api/src/resources.rs:124-135`); it omits `body_format`, `rights` and `native_residue`, and its `files` array is payload plus cover, so previews are dropped (`:227-246`).
`tam-server` takes no key-encryption key and no object-store root; only `tam-worker` and `tam-import` construct a `BlobRepo`.

Everything downstream of a product and its mappings already exists.
`POST /{v}/jobs` enqueues and the engine's pump executes (`crates/tam-api/src/jobs.rs:528`; `crates/tam-engine/src/driver.rs`).
`lower` turns a stated draft-or-live intent plus the mapping's binding into `Create`, `Create` then `Publish`, or `Revise`, reading no marketplace to do it (`crates/tam-storage/src/lowering.rs:46-88`).
`requires_bound_on` gates the publish behind the create that binds its subject (`:97-106`).
Idempotency is a mandatory `Idempotency-Key` header (`crates/tam-api/src/jobs.rs:137-163`) over a per-item key derived from org, inventory, product, intent version and content hash (`crates/tam-marketplace/src/idempotency.rs:32-48`).
Progress is the SSE stream at `/{v}/events/stream`, resumable on `Last-Event-ID` (`crates/tam-api/src/stream.rs`).
Delete legs exist as `ItemOperation::Remove` (`crates/tam-domain/src/lib.rs:499-502`), enqueued today only by the migrate drain (`crates/tam-sync-worker/src/lib.rs:418-427`).

The gap is authoring, and only authoring.

## 2. The API design

Seven new endpoints, and publishing reuses the existing job endpoint unchanged.

`POST /{v}/uploads` takes one `multipart/form-data` part and returns file handles.
It runs `tam_pipeline::ingest` against a scanner and a `TenantBlobSink`, exactly as the import does (`crates/tam-pipeline/src/pipeline.rs:90`; `crates/tam-import/src/lib.rs:442-470`), so bytes land content-addressed and sealed per tenant under `{org-hex}-{hash-hex}` (`crates/tam-storage/src/blobs.rs:68-118`, `:265-275`).
Its body limit is `tam_limits::http::UPLOAD_BODY_BYTES_MAX`, 256 MiB, which is declared and consumed today only by the broker gateway (`crates/tam-limits/src/lib.rs:104`; `crates/tam-session-broker/src/gateway.rs:393`).
It returns the payload handles, the generated cover and any previews; the client holds them and posts them with the create.
This is the only new byte path, and it requires `tam-server` to gain a key-encryption key and a store root.

`POST /{v}/products` writes a draft product plus one mapping per selected platform, both `Binding::Unbound` and `PublishMode::DryRun`, as the import does (`crates/tam-import/src/lib.rs:558-582`).
It enqueues nothing.
It must run after the upload, because the deferred `assert_product_has_payload` trigger refuses a product with no live payload at commit (`crates/tam-storage/migrations/0003_catalogue.sql:126-170`).
It should take the same mandatory `Idempotency-Key` header the job endpoint takes, recorded beside the product: `import_one` mints a fresh product id on every pass and nothing refuses the duplicate, which `crates/tam-storage/src/sync_requests.rs:9-14` already names as the defect a redrained request had to work around.

`PATCH /{v}/products/{id}` edits canonical fields; `DELETE /{v}/products/{id}` sets `product.deleted_at`, a column that exists and that every read already filters on (`crates/tam-storage/migrations/0001_product.sql:22`; `crates/tam-storage/src/product.rs:172`, `:189`, `:293`, `:728`).
`POST /{v}/products/{id}/mappings` and `DELETE /{v}/products/{id}/mappings/{inventory}` add or drop a platform later; `mapping_one_per_inventory` already makes one-per-platform a database fact (`crates/tam-storage/migrations/0004_mapping.sql:63`).
`GET /{v}/vocabulary/{inventory}` serves the registry — canonical field specs, native fields, axis bindings, absent axes (`crates/tam-domain/src/registry/mod.rs:211-217`) — so the form is driven by the registry rather than by a second copy of it in TypeScript.

Publishing reuses `POST /{v}/jobs` unchanged, one job per inventory because a job carries exactly one (`crates/tam-storage/migrations/0005_job_ledger.sql:11`; `crates/tam-api/src/jobs.rs:582`).
The client does not currently send `intent`, so it can only enqueue drafts today (`web/src/lib/api.ts:393-398`; `crates/tam-api/src/jobs.rs:502-508`).

Server-side validation is the registry and the adapters, not a new rulebook: length caps through `LengthCap` and `truncate` (`crates/tam-domain/src/registry/mod.rs:265`), TPT's 95-minor-unit floor (`crates/tam-marketplace-tpt/src/write_model.rs:106`, `:176-179`), and Tes's free-versus-paid licence gate (`crates/tam-domain/src/equivalence.rs:19-23`).
Client-side validation mirrors the same registry through `GET /vocabulary` and is a convenience, never the gate.
Adding routes also means extending `ROUTES`, whose parity test probes every documented route against the mounted router (`crates/tam-api/src/openapi.rs:26`).

The role model does not move: `tam_app` owns every table in `public` under forced row-level security (`db/init/01-app-role.sql:31`), so no grant changes.

Three limits are founder-gated and are therefore proposals rather than choices.
Reuse `UPLOAD_BODY_BYTES_MAX` for the upload ceiling rather than adding a constant.
A per-product payload-file count is bounded by nothing today.
`Tier::quota`'s `listings_max` and `storage_bytes_max` are declared and enforced nowhere in the tree (`crates/tam-limits/src/lib.rs:51-54`, `:68-88`), and a creation flow is the first place they can be.

## 3. The form

One form, a shared core, and a section per selected platform.

The core is `CanonicalProduct` (`crates/tam-domain/src/lib.rs:324-344`): title, body with its declared format, price, payload files, cover, previews, subjects, grades and rights.
Genuinely shared across both marketplaces are title, description, price, files, and the four axes both bind — subject, topic, resource type and phase (`crates/tam-domain/src/registry/tes.rs:203-236`; `crates/tam-domain/src/registry/tpt.rs:76-110`).

Seven things diverge, and the per-platform sections exist for exactly these.
Licence: Tes binds it, declares it required, and refuses delegation over seven `RefdataStore` values (`tes.rs:48-64`), while TPT holds no licence field anywhere and declares the axis measured-absent (`tpt.rs:112`) — so the TPT section shows a disclosed loss where the Tes section shows a selector.
Resource type: Tes takes exactly one of nine writable ids (`tes.rs:107-118`, `:217-222`); TPT folds it into the same flat tag array as everything else (`tpt.rs:91-96`).
Grades: Tes GB takes seven `ageRanges` ids and every other Tes inventory takes thirty `yearGroups` ids, a country fork (`tes.rs:138-148`, `:165-175`, `:238-245`); TPT's nineteen grade values are facets in the tag namespace (`tpt.rs:53-64`).
Files: Tes uploads every payload file (`crates/tam-marketplace-tes/src/flows.rs:635-636`), TPT takes exactly one (`crates/tam-marketplace-tpt/src/flows.rs:634-646`).
Body format: TPT's wire is HTML and renders a markdown body into it (`crates/tam-marketplace-tpt/src/write_model.rs:822-823`); Tes carries the declared format through `descriptionRawType`.
Caps and floors: TPT states 45,000 UTF-16 code units for description (`tpt.rs:34-45`) and a 0.95 minimum price (`tpt.rs:46-52`); no Tes cap or floor has been measured (`tes.rs:22-25`).
Attestation: TPT's create posts `copyright_declaration`, a legal attestation that refuses delegation (`tpt.rs:240-250`), already held per connection (`crates/tam-storage/migrations/0031_connection_custody.sql:56`; `crates/tam-worker/src/main.rs:388-397`) — so the form displays which attestation the write will carry and does not re-ask.

Requiredness must be reported as the registry has it and not invented.
Tes `licence` is the only field declared required anywhere (`tes.rs:50`); every TPT field records `required: false` because no capture contains a refused write, which its own test pins (`tpt.rs:21-23`, `:366`).
So the form marks the Tes licence required, marks nothing else required, and lets the server's refusals speak.

Seller-decides versus best-fit surfaces as one control per delegable axis.
`resolution_for` reads the registry's `Delegation` against a per-tenant opt-in, and `Never` wins over any opt-in (`crates/tam-domain/src/equivalence.rs:264-280`).
The tenant opt-in is not modelled: the elections handler passes `false` unconditionally, so every axis reads `seller_decides` today (`crates/tam-api/src/resources.rs:786-790`).
The control is therefore "I choose" by default and "choose for me" where the axis permits it, disabled with a reason on licence and on the TPT attestation.
An answer given on the form becomes an already-answered election item: `election_item.raised_by` is nullable precisely so an answer authored on a create form can precede every mapping, and the provenance CHECK admits exactly that shape (`crates/tam-storage/migrations/0023_elections.sql:86-90`, `:115-117`).
The projection then finds it settled through `answered_for` (`crates/tam-storage/src/elections.rs:307`), and an "apply to future" tick promotes it to a standing rule (`crates/tam-api/src/resources.rs:831-846`).

## 4. The publish control

Three candidates were weighed: the founder's dropdown of tick boxes, platform toggle chips inline on the form, and a publish dialog listing each platform with a per-platform readiness line.

What the data model can honestly say about readiness, with no marketplace contacted, is a short and complete list.
Whether a connection is linked and carrying work (`crates/tam-api/src/resources.rs:296`).
Whether the inventory is halted (`:695`).
Whether `lower` refuses — create in flight, lifecycle unknown, or an uncaptured transition (`crates/tam-storage/src/lowering.rs:46-88`, `:118-135`).
Whether the projection would block, and on which of five named causes: taxonomy gap, election, unknown currency, missing cover, incomplete scan (`crates/tam-domain/src/lib.rs:369-401`; gate order at `crates/tam-taxonomy/src/listing.rs:263-285`).
Which values this platform loses whatever the seller picks (`crates/tam-storage/src/mapping.rs:1089`).

That list is already a readiness sentence per platform, so the dialog is recommended, with the founder's tick boxes kept inside it.
Each row is a checkbox for one inventory, a readiness line beneath it — "TES: ready", "TPT: needs a licence election", "TES: this listing is live and Tes edit-published is uncaptured" — and one draft-or-live radio for the whole dialog.
An unready platform stays checkable and warned rather than disabled, because the seller may be about to fix it and a disabled control explains nothing.
The reason to prefer it over a bare dropdown is that a bare dropdown lets a seller select a platform whose item will park at the first pump with a cause the seller could have been told before clicking.

The dialog must not overstate itself.
Readiness is a prediction computed at click time; the engine re-projects at seed time and can still park on an election raised there (`crates/tam-engine/src/seed.rs`).
"Ready" is claimed only where a projection dry-run returned `Ok`.

## 5. Full CRUD

Edit is a PATCH on the canonical product, and what reaches a live listing depends on the lowering table.
A bound mapping lowers to `Revise`, and `uncaptured_transition` refuses Tes live-to-live and live-to-draft while TPT serves all four (`crates/tam-storage/src/lowering.rs:118-135`).
So a Tes listing that is already live cannot be edited through us today, and the form must say that rather than accept an edit that will refuse.
`FieldPolicies` already record per field whether we own it, the seller owns it, or we propose (`crates/tam-domain/src/lib.rs:243-261`); a `Frozen` field renders read-only with its reason instead of being silently overwritten.
Beyond the transition table the registry records no constraint on which fields a platform refuses to change post-publish, so this note claims none.

Delete is two operations and the UI must not conflate them.
Removing the product from us is the soft delete above.
Removing the listing from a marketplace is `ItemOperation::Remove`, an engine leg with an adapter behind it (`crates/tam-domain/src/lib.rs:499-502`), enqueued today only by the migrate drain (`crates/tam-sync-worker/src/lib.rs:418-427`).
A local delete that leaves bound mappings behind leaves live listings nobody tracks, so the dialog asks explicitly and defaults to removing remotely first, locally only once each removal settles.
The warning it owes the seller is that both marketplace deletes are irreversible there and the sales history behind a listing belongs to the marketplace.

Errors need no new surface.
The ledger already carries `failure_code`, `failure_detail`, `blocked_on` and `evidence_ref` per item (`crates/tam-storage/migrations/0005_job_ledger.sql:29-32`), the stream carries every state change, and `/{v}/jobs/{job}/items/{item}` renders one (`crates/tam-api/src/jobs.rs:320`).
What the flow adds is the link from a product row to the item carrying it, and two distinct wordings: a refusal the seller can fix, and a fault they cannot.

## 6. Deliberately out of v1

Bulk create from a spreadsheet; the job mechanism has no cap, but the form is one product.
Listing-copy generation, the one place a model is admitted by the decision record.
Per-inventory price overrides: `PriceRule::Converted` exists and nothing writes it (`crates/tam-types/src/lib.rs:288-296`).
Seller-owned TPT shelves: the native field exists and `project_fields` posts an empty category array (`crates/tam-domain/src/registry/tpt.rs:141-148`; `crates/tam-marketplace-tpt/src/write_model.rs:869`).
TPT preview, video and thumbnail slots, which no capture exercises (`crates/tam-marketplace-tpt/src/flows.rs:634-636`).
Etsy, which has no adapter.
The tenant delegation opt-in, unmodelled.
Editing a live Tes listing, blocked by capture rather than by design.

## 7. Founder questions

1. Does the create form accept several payload files?
   A ZIP upload is extracted into one payload file per entry (`crates/tam-pipeline/src/pipeline.rs:103-110`) and a TPT create takes exactly one (`crates/tam-marketplace-tpt/src/flows.rs:634-646`), so a ZIP authored today is structurally unpublishable to TPT.
   Recommended: v1 takes one payload file, offers "keep this ZIP whole" so a bundle stays one file, and marks multi-file products TES-only on the publish dialog.
2. Should deleting a product default to removing the marketplace listings too?
   Recommended: yes, with a per-platform checklist, refusing a local-only delete while any mapping is bound unless the seller ticks "leave the live listings alone".
3. Which limits does the creation flow enforce?
   Recommended: `listings_max` at `POST /products` and `storage_bytes_max` at `POST /uploads`, which are declared and enforced nowhere today and are the two quantities a tenant's writes control.
4. May `tam-server` hold the key-encryption key and the object-store root, which only `tam-worker` and `tam-import` hold today?
   Recommended: yes — an upload must seal bytes somewhere, and the alternative is a second service; the API process already holds the `tam_app` credential.
5. May the create form write an already-answered election before any mapping exists?
   Recommended: yes; migration 0023 was written for exactly this shape and its provenance CHECK admits it.
6. Does Publish default to draft or to live?
   Recommended: draft, matching `parse_intent`'s own default (`crates/tam-api/src/jobs.rs:502-508`), with live a deliberate second action, because a live Tes publish cannot be reversed by us today.
