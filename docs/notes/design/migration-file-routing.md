# Routing a migration's files through the seller's own sessions

How a seller's Tes catalogue reaches TeachersPayTeachers with no file upload by the seller, and where the bytes are at every moment.

- date: 2026-09-04
- status: design accepted as the plan of record; S1 and the redirect fix are being built, S3 and S4 wait on the founder decisions in the last section
- decisions it implements: D1 (the seller's device is the only thing that opens a connection to a no-API marketplace), D27 (file ingest moves to the device so the bytes are on the seller's machine at upload time and never on our servers)
- what it continues: `desktop-data-plane.md`'s interim payload fetch, which this ends; `engine-driver-split.md` steps 10a, 10b, 14 and 15; `tpt-vocabulary-rebase.md`, whose mapping work this consumes unchanged

The founder's statement of the problem is the shortest one: migration between marketplaces must route the files directly through the seller's own sessions with no manual upload, otherwise seller-login access has no point.
TeachBuySell in Australia ships the manual version — a CSV of public details, and the seller drags the files in — and its own help text says the one thing it cannot do is transfer the files.
That is the gap this note closes.

## 1. Whether Tes lets a logged-in seller download their own files

Yes, and it is proven live rather than inferred.
The flow is two steps, both implemented and both gated on `FetchReason::FirstPartyExport`.
`GET https://www.tes.com/resource-detail/api/download/{id}` answers `{"zipUrls":{"{id}":{"url":"/teaching-resource/download/{id}/bundle"}}}`, built by `crates/tam-marketplace-tes/src/endpoints.rs:864` and parsed by `parse_download_manifest` at `:902`, which refuses any url that is not origin-relative.
`GET {ORIGIN}{that path}` answers a 302 to a signed CloudFront url, and the bytes behind it are a ZIP of the seller's originals; the request is built at `endpoints.rs:872` and the pair is driven by `TesAdapter::download_resource_bundle` at `crates/tam-marketplace-tes/src/flows.rs:891`.

The ground truth is `docs/design/decisions.md`, "The two uncaptured endpoints, resolved by a founder-supervised capture, 2026-08-28".
Resource 13549126 returned a 471 KB zip wrapping a 493 KB source pdf and resource 13436008 returned a 12.5 MB zip, both fetched as real bytes rather than assumed from a contract.
A draft has no published bundle and the first step redirects to an HTML `?error=notfound` page, which the adapter classifies as `NoPublishedBundle` rather than as a read failure.
The first pass of that research concluded no download existed; it had probed the resource `attachments` array and missed the download flow the resource-detail page's own button drives, and the correction is recorded in the same section.

Catalogue enumeration is equally settled.
`GET /api/v2/dashboard/getAllResources?page=N&limit=M` and `GET /api/v2/dashboard/getAllDrafts?page=N&limit=M` are page-walked by `TesAdapter::list_own_resources` (`flows.rs:810`), which refuses a walk that never reaches an empty page rather than returning a truncation an importer would mistake for the whole catalogue.

Previews and thumbnails are not a download path and do not need to be.
The `attachments` array carries a signed preview-image url and metadata only, and no fetch of it has been captured.
TPT generates its own thumbnails from the product file — `data[Item][generate_thumbnail]` is a three-way radio whose value `1` is pre-selected on a blank form — so nothing about a Tes preview needs to cross.

## 2. The file path end to end

### The locator

A source locator is a per-file row saying which marketplace resource the bytes live in.
It never holds bytes and it never holds a url that could be fetched without the seller's session.

    product_file_source(
      org_id, file_id references product_file,
      marketplace, connection_id,
      resource_locator, entry_path null,
      file_name, content_type,
      observed_hash null, observed_byte_len null, observed_kind null,
      observed_at, observed_by_device
    )

Two existing behaviours are what make this a real change rather than a column.
`describe_files` (`crates/tam-storage/src/blobs.rs:313`) joins `blob` on the hash to find a length, so a file with no blob row is invisible to the manifest builder.
`POST /{version}/products` refuses a file handle whose hash the tenant has never stored (`crates/tam-api/src/catalogue.rs:629-657`), which is a deliberate guard against a fabricated hash minting a row that points at no object.
Both are correct for an uploaded file and both must fork for a sourced one.

### The manifest, when the server cannot commit to a digest

`PayloadManifest` today is a commitment rather than a description.
The server states the hash and the length before the bytes move, the device checks what arrived against them, and the payload route deliberately carries no digest of its own, because a response that restates its own digest proves nothing.
A Tes-sourced file has no such commitment on its first observation, so the manifest gains a source arm.

    enum PayloadSource {
        ControlPlane { hash: ContentHash, byte_len: i64 },
        Marketplace {
            marketplace: Marketplace,
            resource: String,
            entry: Option<String>,
            expected: Option<Committed>,
        },
    }

What integrity survives is stated here rather than softened, because the answer differs by arm.
Under `ControlPlane` nothing changes: the server committed beforehand and the device proves the transfer.
Under `Marketplace` with `expected: Some(_)` the check is a transfer proof against a previously observed value rather than against an independently known one; it catches a changed or truncated re-fetch and it makes a retry idempotent, which is what it is for, but it does not prove the bytes are the seller's original, because if the first fetch got the wrong bytes every later one agrees with it.
Under `Marketplace` with `expected: None` there is no integrity guarantee at all.
What is lost against today is exactly the server-committed digest, and nothing replaces it, because the server has no independent view of bytes it is forbidden to hold.
What remains is TLS to Tes, the origin pin, the seller's own session, and the fact that one run performs both the fetch and the upload with no third party between them.

The design removes the uncommitted case from every upload that matters.
The import pass in section 3 observes each file once and records what it saw, so a publish always fetches against a committed value and only the first observation is uncommitted.

Two properties are kept cheaply and are worth keeping.
The device reports and the server records, and the server never derives: the reported hash lands under the same rule migration 0045 established for ledger calls, where the device's asserted instant sits beside our receipt as two facts rather than one.
A second device reporting a different hash for the same resource is a recorded disagreement surfaced on the item, never a silent overwrite.

### The device side

`DevicePayloads` (`apps/desktop/src-tauri/src/payload.rs`) keeps its whole shape.
The per-item directory under the application data directory, the length-then-digest check in `verified`, `discard` on settle, the `Drop` backstop for a run that ended by an error, and `sweep` at start-up for a process that was killed are all unchanged and all still the reason the promise "nothing is kept" holds.
Only `load` forks on the source: `ControlPlane` keeps `payload_path` over the control plane, and `Marketplace` drives the Tes adapter under the seller's own session, applies the unwrap rule below, and then runs the same `verified` against the manifest.

`desktop-data-plane.md` says the interim ends with this endpoint and this cache deleted rather than optimised.
That is half right and the correction belongs here.
The route survives for files a seller uploaded by hand and becomes the minority path; the cache is not deleted at all, because it is exactly what the marketplace fetch writes into.

One structural change follows and is not optional.
`DevicePayloads` today holds one transport to our control plane, built in `DeviceWork::execute`.
A marketplace-backed source needs the seller's `SessionTransport`, which exists only inside `LiveMarketplaces::drive`'s per-marketplace arm, so the file source moves into that arm.
The consequence is that a TPT run needs a Tes session, which is architecturally fine because both are the seller's and both are on the seller's own device, but it has two gate consequences.
The tick's four refusals gain a fifth, the source marketplace's session, checked before the claim so a missing Tes login costs a refusal rather than a lease expiry.
And the entitlement grant for the source marketplace must also stand, because the run makes a request to it and the kill switch has to be able to stop that.

### The redirect, which is the one real hazard

`crates/tam-marketplace-tes/src/live.rs` puts the seller's cookie in `default_headers` on the session client, and the module's own first three lines say default headers apply to every host a client reaches.
The client is built with reqwest's default redirect policy, so the bundle request's 302 to CloudFront is followed with the seller's Tes cookie attached.
The host assertion in `route` (`live.rs:105`) and the one in `SessionTransport::send` each run once, against the initial url, and neither runs again on the redirect.

This does not bite today, because the download has only ever run through the broker gateway, whose transport carries no cookie of its own.
It bites the moment the device performs this fetch.

The fix is small and in one place.
The session client takes a redirect policy that follows same-origin and stops at anything else; a blanket refusal to follow would break the `?error=notfound` classification, which depends on a same-origin redirect being followed.
The adapter then sees the 3xx, reads `Location`, asserts the scheme and the host, and re-issues on the existing bare client.
That vehicle already exists and is already proven by `the_bare_client_sends_no_cookie` (`live.rs:420`), and this is the read-side mirror of the exemption the direct-to-S3 upload already relies on.
The broker gateway's own redirect policy deserves one look on the same grounds before the crate is deleted.

### Multi-file resources, which decide the file model

`TptAdapter::sole_file` (`crates/tam-marketplace-tpt/src/flows.rs:661`) refuses any projection carrying other than exactly one file, because a TPT create takes exactly one file into the product slot and the preview, video and thumbnail slots are uncaptured.
`import_one` ingests the Tes bundle under `ArchiveMode::Explode`, so a Tes resource whose bundle holds two files becomes a two-payload product, and every TPT create for it settles `Failed` and terminal today.
That is a live defect for the migration story rather than a new constraint this design introduces.

The answer for a migration is to stop exploding.
One payload file per Tes resource, the bundle whole, `entry_path` null; TPT accepts a ZIP, and the bundle is exactly what the seller's Tes buyers already receive.
One refinement is decided on the device at fetch time: a bundle holding exactly one entry resolves to that entry, because a buyer expects the pdf rather than a zip wrapping a pdf.
That refinement never changes the file count, which stays one, so the server-side projection is stable and the manifest's shape is decided at import rather than at upload.
Per-entry locators are the later generalisation for a seller who wants their files separated, and `entry_path` exists in the schema for it.

### Size and type

Tes caps a file at 200 MB across 58 extensions; TPT caps at 4 GiB across 49.
Size therefore cannot block a Tes-to-TPT migration, and saying so is more useful than a gate that never fires.
The gate is still written generically, because it fires constantly in the reverse direction and on Etsy, whose limit is five files of twenty megabytes.

Type can block, because 58 against 49 is a real intersection and TPT's list sits behind a modal mount that is empty until it is clicked.
The gate must be a seller-visible publish gate in the same shape the taxonomy gap and the currency gate already use: a blocked projection plus a queue item naming the file and its extension, offering to replace the file or skip the product.
It must never be a `Failed` at submit time, because that is terminal and the seller cannot act on it afterwards.

### The cover

`project_listing` blocks on a missing cover for every inventory (`crates/tam-taxonomy/src/listing.rs:292-294`), and the comment says the requirement is Tes's while the code is unconditional.
A device-ingested product has no cover on our side unless the device sends one.
The recommendation is that the device generates it with `tam_pipeline::render::cover` — a fixed 512 by 384 PNG, tens of kilobytes — and uploads only that through the existing `POST /{version}/uploads`, which already returns exactly the file handle the create path wants.
The sellable bytes stay off our servers and a derived thumbnail does not, which is a deliberate exception to the rule rather than an oversight, and therefore a founder decision.

### The scan

The projection requires every payload scanned clean, and the server scans at ingest today.
With the server never seeing the bytes, the scan moves to the device — `EicarScanner` is in `tam-pipeline`, which is already in `portable_crates_64` — and becomes a device assertion.
That is a downgrade and is recorded as one.
The mitigating fact is that the bytes travel from the seller's Tes account to the seller's TPT account and we are at no point a distribution point for them, which is a different risk posture from an upload into a store we serve.

## 3. Moving the catalogue read to the device

The device enumerates with `list_own_resources`, reads each listing with `fetch_for_import`, and streams each bundle through the pipeline with a discarding sink: fetch, probe the kind, scan, render the cover, hash, keep the cover, throw the payload bytes away, report the descriptor.
One bundle sits on disk at a time and nothing accumulates.

Downloading during the import rather than deferring it is a deliberate choice.
It costs one pass over the catalogue, which for a teacher's shop is minutes and happens once with visible progress, and it buys a complete projectable catalogue immediately: real kinds, sizes, hashes and covers, no new gate state anywhere, and a committed digest for every upload that follows.
The cost is that the bytes cross the seller's network twice, which is what the caching question in the last section asks about.

The wire is one route.
`POST /{version}/devices/{device}/import`, cookie-session authenticated like the rest of the device surface, taking `{ request, page: [ImportedResource], complete }`, where an `ImportedResource` is the seller's locator, the verbatim `ImportedListing` the adapter read, the observed file descriptor, and the cover handle.
`ImportedListing` and its parts need serde, which is the same move step 10 already made for the driver vocabulary and therefore a known shape rather than a new question.

Resumability falls out of what already exists.
`sync_request_resource` carries a per-resource breadcrumb precisely because `import_one` commits four times internally and mints a fresh product id on every pass, so a second pass over an already-canonicalised resource is skipped rather than duplicated.
Posting a page at a time against that breadcrumb makes a several-hundred-resource import survive an application restart with no new machinery.

What of the import is reused and what is deleted:

The `fetch_for_import` call goes, because the device made it.
The `ingest` call with its `BlobRepo`, `TenantBlobSink`, `LocalObjectStore` and key-encryption key goes, because no bytes reach us.
`ImportEntry` and `NamedBytes` go, because no bytes travel on the wire.
`NoImportFiles` goes, because there is nothing left for it to refuse.

Recorded 2026-09-04, ahead of that slice and owed to it: the operator import's `discover` and `measure` paths went with the Tes adapter when the broker's gateway left `crates/tam-import/src/main.rs`, because both reached the marketplace through it and the binary now takes a TPT source and a manifest only.
`measure_one` and `MeasureTotals` are still exported and still covered by `crates/tam-import/tests/import.rs`, so what is owed is a caller rather than the function.
Until one exists, the founder's `measure` kill-gate number — the drain coverage when the originals are not on the box — cannot be taken, and re-pointing it at the device-side import described here is the follow-up.

Everything else is reused unchanged, and it is the great majority and every hard part: `inbound_subjects` over the source vocabulary's edges, the verbatim grade declaration with its age-range labels and derived interval, `resolve_price`, `rights_from`, `residue_of`, the canonical product construction, the product and mapping inserts with their field policies and price rule, the immediate projection with the seller's overrides, `record_losses`, the taxonomy raise, and the whole row report.
What is new is a `product_file_source` row written beside each `product_file`, from the device's observation.
`ImportRun` shrinks to a pool, an organisation, a source, a target and an instant, and `tam-import` drops its dependencies on `tam-pipeline`, `tam-secrets` and the adapter crates.

`tam-sync-worker` exists to hold the broker socket for the Tes read.
With the read on the device, its canonicalisation leg goes and its enqueue half stays, because minting the create and removal jobs, lowering the intent and deriving the idempotency key are pure ledger work.
The recommendation is to fold that enqueue into the import route's own transaction, so the device saying the catalogue is complete is what mints the create job, which deletes the poller, its key, its store root and its broker lease outright.

That is the broker's last consumer that matters, and it answers open question 3 of `engine-driver-split.md` by removing the consumer rather than re-pointing it.
Nothing here blocks the deletion and the deletion blocks nothing here.
Step 15 already built the device-reported connection writer and the gate ran positive on 2026-09-04, so what remains for the deletion is the founder's words on the five items step 14 lists, not this work.

## 4. The seller's flow

Five screens, three of which exist.
The console decides and the desktop works, which is the product's existing shape and is unchanged.

Marketplaces exists, and both rows must read connected; the Tes row already carries the device-reported link from step 15, so the only new thing is that the migrate entry point appears once both are connected.

The declaration screen exists, and this flow makes the founder item step 14 raised acute.
An unattested TPT connection reaches the submit, the adapter refuses, and the item settles `Failed` and terminal, so a seller migrating two hundred products with no declaration burns all two hundred and declaring afterwards brings none of them back.
The recommendation is that the migrate flow refuses to start without the declaration, which is one gate a seller can act on and is far cheaper than the park-on-a-gate change the step contemplated.

The import screen is new, driven by the desktop and mirrored in the console, with per-resource progress, a running count, an explicit statement that nothing is being kept on our servers, and resumption after a restart.

The review screen mostly exists, as the inventory page with its marketplace chips plus the queue and reconciliation pages.
Per product it shows the mapped TPT category and grade, the taxonomy gaps the projection already raises, and the two new gates for file type and cover.
The mapping and vocabulary work behind it is done, so this is largely rendering gates that already exist.

Publish all exists: the sync request endpoint takes the whole set, and the sync and job views already stream per-item status.

Four classes of field Tes cannot supply, each in its own place.
The copyright holder is per connection and made once, on the declaration screen that exists.
The tax code is never defaulted, because D7 makes the designation the seller's under TPT's terms; the recommendation is to ask once with a per-product override, since a teacher's catalogue is usually one code, which is a new defaults panel.
Standards stay a disclosed loss on a first migration, because Tes carries a coarse framework only and TPT's leaf node ids are a different vocabulary, and the projection already emits a loss record naming the axis.
Grade and subject are already crosswalked, and what is unmapped raises the queue item it raises today.

The net recommendation is one defaults step before the import runs, asking for the two things nothing can supply, so the review screen carries a handful of gates rather than one per product.

## 5. What this reuses, and what it makes unnecessary

Reused entirely unchanged: the work order, claim view and settle envelope; the lease with its epoch fence and renewal; the duplicate-create fence that answers an attempt already in flight; the rate ceiling; the reconcile path with its subject and its stranded state; the reaper; the per-item payload directory with its discard, drop and sweep; the digest check; the projection and its gates; the taxonomy crosswalk and the per-seller overrides; the elections; the job and item ledger; the console's sync and job views; and the device registry and check-in.

Made unnecessary: server-side blob storage of migrated payloads, and with it the storage quota's dominant term for a migrating seller.
That is a billing consequence worth naming rather than discovering, because metering moves further onto connected marketplaces and catalogue size, which is the axis D4 already chose.
Also unnecessary: the import's refusing file source, the import crate's pipeline, secrets and adapter edges, the sync worker's credential and socket, and the broker's Tes allow-list entries for the dashboard, download-manifest and bundle routes.
The device payload route survives for hand-uploaded files and stops being the only path.

## 6. Slices, in order

The assumption behind every figure is stated so the estimate can be falsified rather than argued about: this repository's agent-driven cadence, the founder answering a gating question the same day, the working copy green under `just check` throughout, and no live marketplace contact except where a probe is named.
That is the same basis as the re-baselined column of `vendoo-for-teachers-rethink.md`.

S1, the manifest learns about sources, half a day to one day.
Add the source arm to the payload manifest, keep the control-plane arm as the only one constructed, and make the device's payload load dispatch on it; behaviourally inert.
Verified by the wire round-trip covering both arms, which fails under a serde shape that drops one, and by the existing desktop payload suite passing unchanged, which fails under any behaviour change.
No probe.

S2, the marketplace-backed file source, one and a half to two and a half days.
The marketplace arm driving the bundle download under the seller's session, the detached cross-origin redirect, the single-entry unwrap, verification against the committed value, and the source-session and source-entitlement refusals in the tick.
Verified by a scripted transport serving manifest, redirect and bundle and asserting the second request carries no cookie; by an off-origin location refused; by a single-entry bundle unwrapping while a two-entry bundle stays whole; by a digest mismatch stalling rather than uploading; and by the same-origin redirect still being followed so the not-found classification still holds.
Gated on a founder probe: the redirect leg on their own Windows machine, one published resource, read-only, recording the location's host and scheme, whether the signed url fetches with no cookies, and the byte count against the known size.

S3, the device imports the catalogue, two and a half to four days, and the first slice a seller can feel.
The desktop pass with its discarding sink and its screen, the import route, the import split, the locator migration, serde on the imported listing, and the console mirror.
Verified by the apply half reproducing the row report today's import produces for the same listing; by the same page posted twice creating one product; by a page naming another tenant's resource being refused; and by the discarding sink asserted to write no blob row, which is the decision's own property and the thing a wrong implementation gets wrong.
Gated on a founder probe: enumerate once from the device and compare the count against the Tes dashboard, because the live run of 2026-08-28 saw both dashboard routes answer an empty array with HTTP 200 on an authed session, suspected to be site-context scoping.
Whatever that probe answers, the interface must never render an empty enumeration as "no listings", because an empty shop and a failed read are indistinguishable and the wrong one of the two is far more expensive.

S4, the migrate flow, one and a half to two and a half days.
The defaults step, the import, the review with its two new gates, publish all, and the sync worker's read leg deleted with its enqueue folded into the import's completion.
Verified by an end-to-end route test from request through pages to a minted create job; by a migrate refused to start with no declaration; and by a file outside TPT's extension set raising a gate rather than a terminal failure.
Gated on one founder act: TPT's accepted extension list, which is one click of the supported-types modal or a confirmation that the captured upload configuration already carries the forty-nine.

S5, the live run, half a day of ours plus the founder's.
One resource, their own Tes shop to their own TPT store, watched.
The record of the first live Tes contact — four adapter defects, none caught by any test, every one of them a cassette diverging from the live API — is the standing evidence that this step is not ceremony.

The total is six and a half to ten and a half days, of which S1 to S3 is the part that removes the manual upload.

## 7. The other direction, and Etsy

What generalises is most of it.
The marketplace source arm is marketplace-agnostic, the locator names a marketplace, and the source-session readiness gate, the type and size gate, the cover-on-the-device rule, the import wire shape and the entire apply half carry across unchanged.

What is Tes-specific is the two-step manifest-then-bundle flow, the `zipUrls` parsing, the ZIP-bundle shape with its single-entry unwrap, and the published-only rule.

TPT to Tes needs TPT's own-file download, and while `TptAdapter::download_resource_bundle` exists it is uncaptured: the registry refuses through it by name in `crates/tam-storage/src/lowering.rs:157`, and `crates/tam-api/tests/jobs_flow.rs:280` asserts that refusal.
So the reverse direction is gated on a TPT capture of the same kind that settled Tes's, and everything else will already be built.

Etsy is the opposite case, and it is the two-branch rule working rather than a contradiction.
D1 puts a sanctioned marketplace's automation server-side under its own token, so an Etsy upload is a server upload and the bytes must be on the server, which is why the control-plane source arm never goes away.
The awkward composition is Tes to Etsy, where the device would fetch from Tes and hand the bytes to us for the Etsy leg.
The recommendation is to refuse that composition in the first pass and name the refusal, rather than discover it in production.

## 8. Founder decisions

Open. Each row states the decision, the recommendation and what taking the alternative would cost.

| # | Decision | Recommendation | Status |
|---|---|---|---|
| Q-a | Is a digest reported by the device acceptable integrity on a first observation, where the server holds no independent commitment? | Accept, recording the device and its asserted instant beside our receipt, and surfacing a later disagreement rather than overwriting it. The alternative is two independent fetches whose agreement is the check, which doubles the transfer for a property the seller's own session already largely provides. | open |
| Q-b | Is a malware scan performed and asserted by the device acceptable, given the server never sees the bytes? | Accept. The bytes travel from the seller's own account to the seller's own account and we are never a distribution point for them. | open |
| Q-c | May the device upload a derived cover image, as the one exception to keeping bytes off our servers? | Accept: a fixed 512 by 384 PNG per product, so the console has an image and the existing cover gate stands unchanged. The alternative is to make that gate target-conditional and show no image in the console. | open |
| Q-d | May payload bytes be cached on the device between the import pass and the publish? | No for now. Discard on settle, the drop backstop and the start-up sweep are what make "the file is still the seller's" true after a crash. Revisit if the second fetch of a large resource is felt, with a seller-visible toggle and a size ceiling. | open |
| Q-e | Is a migration's payload the bundle whole rather than its exploded entries? | Accept, unwrapping only a bundle that holds exactly one entry. TPT takes exactly one file, and the bundle is what the seller's Tes buyers already receive. | open |
| Q-f | Does the migrate flow refuse to start without the TPT copyright declaration? | Accept. It converts step 14's open item from a whole catalogue of terminal failures into one gate the seller can act on. | open |
| Q-g | Does a Tes-sourced TPT upload require the Tes entitlement grant as well as TPT's? | Yes. The run makes a Tes request, and the kill switch must be able to stop it. | open |
| Q-h | Is a Tes-to-Etsy migration refused in the first pass? | Accept the refusal. Satisfying the sanctioned branch would mean routing the seller's bytes through us, which is the arrangement D27 removes. | open |

Two items are flagged rather than decided, because they belong to owners other than this note.
Whether the broker gateway leaks the seller's cookie on the same CloudFront redirect is worth one read of its redirect policy before the crate is deleted.
And whether the observed hash and kind columns are nullable-until-observed or provisionally filled with a later correction is a schema call to take with the migration in hand; the provisional form is the lighter of the two.

## Sources

- `docs/design/decisions.md`, "The two uncaptured endpoints, resolved by a founder-supervised capture, 2026-08-28".
- `docs/notes/design/vendoo-for-teachers-rethink.md`, decisions D1, D7, D27, and the re-baselined phase table.
- `docs/notes/design/desktop-data-plane.md`, "The interim payload fetch, and what it owes".
- `docs/notes/design/engine-driver-split.md`, steps 10a, 10b, 14 and 15, and open questions 3 and 4.
- `docs/notes/design/tpt-vocabulary-rebase.md`, for the mapping this consumes unchanged.
- `docs/research/rethink/tpt-create-form-dom.md` and `tpt-product-model.md`, for TPT's file slots, caps and extension count.
- `docs/research/rethink/cross-marketplace-mapping-tpt-base.md`, for the Tes file constraints and the projection table.
