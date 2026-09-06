# Routing a migration's files through the seller's own sessions

How a seller's Tes catalogue reaches TeachersPayTeachers with no file upload by the seller, and where the bytes are at every moment.

- date: 2026-09-04
- status: design accepted as the plan of record, and partly landed. On `main` at 2108cd0f: S1 with its compatibility shim, the same-host redirect policy, S2's marketplace-backed file source, and the session broker's deletion with its role. Staged above it: the re-issued marketplace-named hop, and S3's C1 and C5. In flight: the two-armed `FileBytes` and the locator it is written through, which land together because the type and the writes that consume it are in different crates. S4 is unblocked and unstarted. Every founder decision in the last section is taken: Q-c to Q-h adopted by silence on 2026-09-04, Q-a and Q-b decided on 2026-09-04
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
Amended 2026-09-07: that page is a Tes page and carries the word the sign-in sniffer keys on, so the founder's first live import skipped both drafts in the shop as `SessionExpired` on a session that read the next listing fine.
The adapter now reads the state route when the manifest hop looks like a sign-in page, and answers `NoPublishedBundle` for a draft; the device skips a draft by the state it already read, before the manifest is asked for, with a sentence naming the draft and what changes it.
The first pass of that research concluded no download existed; it had probed the resource `attachments` array and missed the download flow the resource-detail page's own button drives, and the correction is recorded in the same section.

Catalogue enumeration is equally settled.
`GET /api/v2/dashboard/getAllResources?page=N&limit=M` and `GET /api/v2/dashboard/getAllDrafts?page=N&limit=M` are page-walked by `TesAdapter::list_own_resources` (`flows.rs:810`), which refuses a walk that never reaches an empty page rather than returning a truncation an importer would mistake for the whole catalogue.

Previews and thumbnails are not a download path and do not need to be.
The `attachments` array carries a signed preview-image url and metadata only, and no fetch of it has been captured.
TPT generates its own thumbnails from the product file — `data[Item][generate_thumbnail]` is a three-way radio whose value `1` is pre-selected on a blank form — so nothing about a Tes preview needs to cross.
Amended 2026-09-07: what the console shows for a migrated resource is therefore the device's own derived cover, and for anything but an image payload that is `tam_pipeline::render::placeholder_card`, a solid card coloured by file kind — PDF red, PPTX orange, DOCX blue, a bundle of several files grey.
The founder read those cards as wrong thumbnails; carrying the Tes preview instead needs a capture of the `attachments` shape on one resource state and of its preview url's answer, and is a founder decision because it fetches an image the seller did not upload through us.

The listing read carries the declared resource type as of 2026-09-07: `mainType` crosses as the `ResourceType` axis and the import maps it over the seeded crosswalk, so a migrated product is typed on the target rather than arriving typeless.
The same read carries the price as the wire's own integer of minor units; until then it multiplied that integer by a hundred, and the founder's £5.00 listings were imported as £500.00.

## 2. The file path end to end

### The locator

A source locator says which marketplace resource a file's bytes live in.
It never holds bytes and it never holds a url that could be fetched without the seller's session.

This note first drew it as a side table, `product_file_source`, keyed on the file it describes.
It landed as columns on `product_file` itself, in migration 0052, and the change is recorded here rather than quietly rewritten because the reason generalises to any side table describing a row that is constrained on write.
`product_payload_nonempty`, a deferred trigger in `0003_catalogue.sql`, requires a product and its first payload in one transaction, and `product_file_blob_or_source` is a row CHECK evaluated per statement.
A side table cannot be written by the statement that writes the row it completes, so the file row would exist for one statement as neither blob-backed nor sourced, which is precisely the state that CHECK exists to refuse.
Columns on the row make that state unconstructible rather than merely rejected, and `insert_sourced_file` (`crates/tam-storage/src/product.rs:587`) writes the whole group in one statement.

    product_file(
      org_id, id, product_id, position, role, kind,
      hash null, scan_state null,            -- the blob-backed arm, unchanged
      source_marketplace, source_connection,
      source_resource, source_entry null,
      payload_file_name, payload_content_type,
      observed_hash, observed_byte_len,
      asserted_scan_state, asserted_scan_signature,
      asserted_scan_failure_code, asserted_scanned_at,
      observed_by_device, observed_at, recorded_at
    )

One part did stay a table of its own, for a reason the column form cannot serve.
`product_file_observation` is append-only and holds every observation, while the file's own source group records the first and is never overwritten by a later one.
So a second device reporting a different digest for the same resource is a disagreement the item view derives from more than one distinct `observed_hash` there, rather than a write refused or a value silently replaced.
`observed_at` is the device's own instant and `recorded_at` is our receipt of it, two facts rather than one.
Both columns are created by 0052; what 0045 established is the discipline they follow, for ledger calls, and 0052's own comment cites it as though it created them.

Two existing behaviours are what make this a real change rather than a handful of nullable columns nobody reads.
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
Only `load` forks on the source: `ControlPlane` keeps `payload_path` over the control plane, and `Marketplace` drives the Tes adapter under the seller's own session and applies the unwrap rule below.
Whether what arrives is then checked is the manifest's to say rather than the fetch's: `checked` verifies against a commitment where one exists and returns the bytes where none does, so the uncommitted first observation is visible in the code as the absence it is rather than hidden behind a check against an invented value.

`desktop-data-plane.md` says the interim ends with this endpoint and this cache deleted rather than optimised.
That is half right and the correction belongs here.
The route survives for files a seller uploaded by hand and becomes the minority path; the cache is not deleted at all, because it is exactly what the marketplace fetch writes into.

One structural change follows and is not optional.
`DevicePayloads` today holds one transport to our control plane, built in `DeviceWork::execute`.
A marketplace-backed source needs the seller's `SessionTransport`, which only `LiveMarketplaces` can build, because only it holds the session store.
The consequence is that a TPT run builds a Tes session, which is architecturally fine because both are the seller's and both are on the seller's own device, and it is stated at the point it happens rather than left to be discovered.

Where the resulting refusal lives was got wrong in this note's first draft, and the correction is worth keeping visible because the reasoning generalises.
The draft said the tick's four refusals gain a fifth, the source marketplace's session, checked before the claim so that a missing login costs a refusal rather than a lease expiry.
That is not achievable.
The scheduler decides per marketplace before the work source is touched at all — which is the whole of that gate and is right — but which marketplace holds an item's *files* is knowable only from the order the claim returns, so the question cannot be asked before the claim exists.
The refusal therefore sits in `DeviceWork::execute`, after the claim and before anything is composed, and it costs a lease expiry.
That cost is not novel: the inventory-mismatch refusal already beside it pays exactly the same price, and `desktop-data-plane.md` records that paying it is what makes serving a server-side filter worth doing.
The server-side claim predicate is what removes the cost, using `DeviceRepo::holds_connected_session`, which already exists; it lands separately from the device half.
Both halves of the question are asked there — the session and the entitlement — because fetching the seller's own file is a request to that marketplace, so a revoked grant on the source must stop the run even when the marketplace being written to is entitled.

### The redirect, and a claim this note got wrong

This section first said the bundle download would send the seller's Tes session cookie to CloudFront, and that was wrong.
It is corrected here rather than quietly rewritten, because the corrected reasoning is what the policy now rests on and because the first version was the more alarming of the two.

reqwest strips `Cookie`, `Authorization`, `cookie2`, `Proxy-Authorization` and `WWW-Authenticate` from any followed redirect whose host or port changes.
`remove_sensitive_headers` in `redirect.rs` does it, and `on_request` calls it unconditionally after every follow, so under the default policy the cookie would have been dropped at the hop.
The same mechanism answers the open item this note carried about the broker gateway: its `Authorization` header was covered too, so the gateway leaked nothing and its deletion needs no such audit.
What would have travelled is the non-sensitive remainder of `default_headers`, which here is `User-Agent`.

Owning the policy is still right, for two reasons that survive the correction.
That stripping is a dependency's internal behaviour rather than a contract: nothing in reqwest's public API promises it, a minor release could narrow the list, and a custody rule this codebase states in its own module documentation should not rest on a list we do not own.
And the list is not everything a session client carries — it does not include `User-Agent`, nor whatever a future edit adds to `session_headers`, and the TPT adapter's session client already carries a browser identity and a CSRF header beside its cookie, which is the shape the Tes one drifts toward.
A rule about the destination stays true as the headers change; a rule about a header list does not.

The policy follows a redirect only while the host does not change, and stops otherwise.
Stopping rather than erroring is the design half rather than the safety half: the caller gets the 3xx with its `Location`, which is a fact a flow can act on by re-issuing the hop on a client carrying nothing, and an error would leave it unable to tell a declined hop from a marketplace that is down.
A blanket refusal to follow would also have broken the `?error=notfound` classification, which depends on a same-origin redirect being followed.
Reaching the hop cap errors instead, matching `Policy::limited`'s own comparison so the constant means one bound on both clients, because a redirect loop is not a destination anybody can re-issue.

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
`POST /{version}/devices/{device}/import`, cookie-session authenticated like the rest of the device surface, taking `{ request, resources, skipped, complete }`.
An entry of `resources` is the seller's locator, the verbatim `ImportedListing` the adapter read, the observed file descriptor, and the cover.
Two corrections to this line as first drafted, both from building it.
The cover is the PNG itself, base64 in the page, rather than a handle from a prior upload: the page is then atomic, so a resource is described and its cover stored together or not at all, and the upload route would otherwise run the ingest pipeline over an image and derive a cover from the cover.
And `skipped` is a field of the page rather than something the device keeps, because completion is what mints the write jobs and a completing page that said nothing about its failures would start a publish for a partial catalogue while reporting success.
`ImportedListing` and its parts need serde, which is the same move step 10 already made for the driver vocabulary and therefore a known shape rather than a new question.
The vocabulary itself lives in `tam-engine-driver`, not on the device, so the producer and the consumer compile against one definition; every string it defines is bounded and validated on decode, because the server has never seen the machine that posts a page.

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
What is new is the source group written on each `product_file` from the device's observation, and the `product_file_observation` row that records the same observation as its own fact.
`ImportRun` shrinks to a pool, an organisation, a source, a target and an instant, and the apply half names no pipeline, no key, no object store and no adapter.
The crate keeps those dependencies, and an earlier draft of this line was wrong to say it drops them.
`crates/tam-import/src/main.rs` is the founder's operator import and stays: it reads a TPT listing through its own adapter and ingests the manifest's files from the operator's disk, so `tam-pipeline`, `tam-secrets` and `tam-marketplace-tpt` are the binary's edges rather than the library's.
That is the whole point of the split — one apply half serving a caller that holds bytes and a caller that never sees them — so the dependency staying is the design working rather than a leftover.

`tam-sync-worker` exists to hold the broker socket for the Tes read.
With the read on the device, its canonicalisation leg goes and its enqueue half stays, because minting the create and removal jobs, lowering the intent and deriving the idempotency key are pure ledger work.
The recommendation is to fold that enqueue into the import route's own transaction, so the device saying the catalogue is complete is what mints the create job, which deletes the poller, its key, its store root and its broker lease outright.

### How a file says where its bytes are

`tam_types::ProductFile` carried `hash`, `byte_len` and `scan` beside each other, which cannot describe a file whose bytes we do not hold.
It now carries one value, `FileBytes`, with two arms: `Held { hash, byte_len, scan }` for bytes in our object store, and `Sourced { marketplace, connection, resource, entry, payload_file_name, payload_content_type, observed }` for bytes the seller's marketplace holds, where `observed` is what one device reported — its device, digest, length, scan and its own instant.

Two arms rather than nullable fields because there are two claims, not one claim sometimes absent.
`hash` is a digest the server computed over bytes it holds; `observed_hash` is a digest a device asserted over bytes we never held; and a schema or a type that let one stand where the other is expected would let a device's word be read as our verification.
The database says the same thing in `product_file_blob_or_source`, which admits a row that is blob-backed or marketplace-sourced and neither both nor neither, so the type makes the rejected row unconstructible rather than merely rejected — the difference between learning at compile time and learning from a constraint violation inside a transaction.
`byte_len` and `scan` moved inside the arms for the same reason: the held length is the input to the blob assertion and the sourced one is the device's report, and the two verdicts have two authors.

A product with a sourced payload is unwritable without this, which is what settled it.
`PayloadSet` is non-empty by construction, so the product insert must be handed a payload file and will write whatever it is handed; and `product_payload_nonempty`, a deferred trigger in `0003_catalogue.sql`, closes the same invariant in SQL, so a product and its first payload must land in one transaction and no separate write path can create one.
A device-imported product is mixed — a sourced payload and a blob-backed cover — so one insert must write both, and only the type can tell it which is which.

### What must be settled before anything produces a sourced file

Five requirements, each with its decision, recorded here because a wrong reading of any of them is silent rather than loud.

What `expected` digests: the bytes handed onward after the unwrap decision — the sole entry where a bundle reduces to one file, the bundle otherwise — and never the container.
A marketplace that re-zips a bundle with different timestamps changes the container's digest while the entry's is unchanged, so digesting the container would stall a single-file resource on a mismatch that means nothing.

What an unwrapped file is called: the name and content type come from the entry, recorded by the producer at import time and carried in the manifest, never from the wrapper.
A device that unwraps and finds the manifest's name disagreeing with the entry's refuses by name rather than uploading a worksheet as a zip.

What `entry: Some(_)` means to a device today: nothing, and it must therefore be refused by name rather than ignored.
Ignoring it would upload the bundle while the manifest says an entry, which is the wrong-bytes case wearing the right digest's name.

What the two marketplace requests per sourced file cost: they are charged to the source marketplace's rate window, the same shape `driver.rs` corrected once already.
Fetching the seller's own file is a marketplace request like any other, and an unbilled one is a ceiling that does not hold.

Which devices may be handed a sourced item: only one whose `app_version` is at or past the S1 shim.
The rule is per claiming device rather than fleet-wide — the claim already names the device and reads its row — so a device below the shim is not handed such an item and takes the idle path that already exists, leaving the item for a device that can run it, rather than an up-to-date machine being penalised for a stale sibling.
The fleet-wide condition is real but it is the condition for retiring the shim, not for emitting.
A tenant whose only devices are old must not see an item wait in silence: the item's view says it is waiting for a device at or past that version, and the console says to update the desktop application.

### The cover travels inside the page

An earlier draft had the device upload the derived cover through the upload route and hand back a file handle.
It carries in the page instead, base64 beside the resource it belongs to, for two reasons found while building it.
The page becomes atomic, so a resource is described and its cover stored together or not at all rather than leaving an orphaned blob behind a failed page.
And the upload route runs the ingest pipeline over whatever it is given, which for an image this pass has already produced means generating a cover of a cover.

### A correction about the coverage number

This note said the device-side import was the natural caller for `measure_one`, and that was wrong about where the call goes though right about where the data comes from.
`measure_one` takes an import run, opens a taxonomy repository on its pool and computes coverage against the canonical terms and the projection edges, none of which a device has.
What is true is better: every field of a `MeasureReport` already falls out of `import_one`'s own row report when the server applies a page, so the founder's kill-gate number is an aggregation in the import route rather than a second walk of the seller's shop.

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
How the seller starts it was the gap C5 left and C5b closes: the pass existed with no trigger and no way to reach a marketplace, so the console's button invoked a command nothing registered and the seller read a developer's error as though it were a refusal.
The trigger is a `start_import` command taking a request id alone; the device reads that request from the control plane to learn which shop it names, so a console cannot ask a device to enumerate one the request does not name.
Inside the application the console invokes it directly; in a browser it says to open the application, because the pass runs on the device by construction.
The enumeration runs inside that command rather than behind it, so the seller waits for it and every failure that precedes the first posted page — the entitlement, a revocation, an unreadable catalogue — is a sentence at the click rather than a request view that never changes.
That wait is bounded at five minutes, because it is otherwise the walk's own limits that bound it: fifty resources a page and forty pages a walk, published and drafts walked separately, each request capped at thirty seconds, which is eighty requests and forty minutes of a button reading "Starting…".
At the bound the command answers a named refusal saying the read was stopped, nothing was imported, and starting again is safe.
A failure after the first page is the request's, and the device settles it there: a page carrying `failed` records the reason as the request's `failure_detail`, which is the page the seller is already watching.
Reaching a request the seller navigated away from needed a list, `GET /{version}/sync`, because a device-branch migrate mints no job until its completing page and so appeared in no list at all until then.

The review screen mostly exists, as the inventory page with its marketplace chips plus the queue and reconciliation pages.
Per product it shows the mapped TPT category and grade, the taxonomy gaps the projection already raises, and the two new gates for file type and cover.
The mapping and vocabulary work behind it is done, so this is largely rendering gates that already exist.

It also carries a per-product choice of which file to send, added by founder decision on 2026-09-04.
Each product defaults to using the Tes file, which is the direct route this whole note exists to build, and offers "choose a file from this computer" beside it.
In S4 that second option is the upload path that already exists: the file goes through `POST /{version}/uploads` into our store and is fetched back at publish, which is the control-plane arm of the manifest and needs nothing new.
That it puts the seller's chosen file on our servers is the honest cost of reusing the built path, and it is bounded by being a per-product election rather than the default.
A device-resident variant that keeps a chosen file off our servers entirely is a later slice, and Q-a is what makes it possible: a file the seller picked has no commitment from anybody, so it is the uncommitted case with no import pass to observe it first, and the device's own report is now an accepted answer to that.

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

S1, the manifest learns about sources, built.
The source arm on the payload manifest, the control-plane arm the only one constructed, and the device's payload load dispatching on it; behaviourally inert.
Verified by the wire round-trip over both arms and both axes of the marketplace one, and by the existing desktop payload suite passing unchanged.

It carries a deprecation shim, and the shim is the part with a deadline.
Desktop 0.1.3 is published and auto-updating, and its own copy of the manifest declares `hash` and `byte_len` as required top-level fields with no serde attributes.
A server emitting only `source` would make every claimed item carrying a file fail to decode there — and fail after the claim, so the item sits leased until its lease expires while the device reports a failure every poll.
So the manifest's `Serialize` is written by hand and emits the old pair beside `source`, derived from the source rather than stored, which is why the two cannot drift; reading tolerates both shapes because the derived `Deserialize` ignores fields it does not know.

A marketplace source emits neither old field, and that is chosen rather than fallen into.
No value would let a 0.1.3 client succeed with one: it has no marketplace fetcher, and our object store holds no bytes for that file, so a synthesised hash would only send it to a payload route answering 404 — failing later, after a round trip, in a shape that reads as our server being broken rather than as a client being too old.
Failing at the envelope is louder and truer, and the cost is real: that item stays leased until its lease expires.
The window therefore has an end condition rather than a hope.
It closes when every registered device reports an `app_version` at or past this change, which `device` rows carry at registration and at every check-in.
That fleet-wide condition is what retires the shim, and this line previously read it as the condition for emitting a source at all, which contradicted the per-device rule stated under the five requirements above and is corrected here rather than quietly dropped.
Emitting is gated per claiming device: the manifest builder already emits a marketplace source for a sourced file, no import has yet written a sourced row for it to describe, and C6 decides which device is offered such an item, so a machine below the shim is passed over rather than handed an envelope it cannot decode.

S2, the marketplace-backed file source, built.
The marketplace arm driving the bundle download under the seller's own session, the single-entry unwrap, and the source-session and source-entitlement refusals — post-claim in `DeviceWork::execute` rather than in the tick, for the reason recorded above.
Verified by the seller's bytes coming from the marketplace with our control plane asked for nothing, which is D27's own property; by a marketplace refusal named as itself rather than read as our failure; by a manifest with no attached source refusing before any transfer; by an uncommitted first observation being accepted rather than checked against an invented value; and by the marketplaces holding an order's files being named once each.

The leg that was left unfinished is now built, and how that decision was taken is worth recording rather than smoothing over.
The bundle hop answers a redirect to a signed url on a content network, the session client declines to follow it, and the hop is re-issued on a client that carries nothing and follows nothing.
This note previously said the choice between a fourth request-authentication variant and a named host constant waited on the founder's redirect probe.
It did not wait: it was taken as the variant, before the probe, on the team lead's decision under the founder's direction to build the seller's path now.
The reasoning stands on its own and the probe would not have overturned it — a host constant has to be maintained against a host the marketplace can re-point without telling anybody, and fails closed in a way that reads as an outage, while the variant names what the request is rather than where it goes.
What the probe now supplies is confirmation rather than a decision, and the first live run is the probe: every way the hop can disappoint us fails closed with a distinct sentence naming which, so what comes back is a diagnosis rather than a failure.
What matters either way is that a declined hop is a named refusal and never an ambiguity, and that is a defect avoided rather than a detail.
`classify_read_bytes` has no 3xx arm, so a declined hop fell to its catch-all and became `AdapterError::Ambiguous`, which is the arm that halts a tenant's inventory and waits for an operator.
The first seller whose migration reached a bundle download would have had their whole inventory halted by a hop we deliberately decline, on a condition that is expected and permanent until the re-issue exists.
The named refusal lives in the download flow rather than in the shared classifier, because a 3xx is legitimate elsewhere on this adapter — the draft manifest's same-origin redirect to `?error=notfound` is followed by the client and never reaches a classifier — so a blanket arm would be a claim about routes nobody has measured.
The shape recurred inside the re-issue rather than past it: a failure of the *second* hop reaches the same catch-all by the same route, so that hop's status range and the archive's magic bytes are both decided before the shared classifier sees the body.
Still owed a founder run, though no longer a gate on a decision: the redirect leg on their own Windows machine, one published resource, read-only, recording the location's host and scheme, whether the signed url fetches with no cookies, and the byte count against the known size.

S3, the device imports the catalogue, and the first slice a seller can feel.
It is six commits rather than one, and where each stands is recorded here because the order between them turned out to matter.

C1, serde on the import read vocabulary, done.
C5, the device pass, done: enumerate, and per resource read the listing, fetch the bundle through the re-issued hop, decide what the payload is, probe, scan, render the cover, hash the bytes handed onward, and post the page, keeping nothing.
Its own property is asserted directly — everything posted is serialised and the seller's bytes are absent from it in raw and encoded form — and it is really the type that holds it, since the observation has no field those bytes could occupy.

C2, the locator and the type, joint with the storage stream, because the type is in one crate and the writes that consume it are in another and they cannot land apart without a broken build between them.
C3, the import split, follows it: the apply half loses the marketplace read and the ingest, and gains what a device observed.
C6, the per-device version gate, lands before C4, and that ordering is a correction rather than a preference: C2 makes the manifest builder able to emit a marketplace source, so a sourced row created before the gate exists could be handed to a device that cannot decode it, which is the compatibility defect the S1 shim exists to prevent, re-created one layer up.
C4, the route, last, and it carries the coverage aggregation.
Built, in four parts that are worth naming because two of them were not foreseen here.
C4a moved the page vocabulary into `tam-engine-driver` and bounded every string in it after a review found four unbounded ones behind a claim that said otherwise.
The route applies each page through C3's `import_one`, writing the payload as a marketplace-sourced file and the cover as the one blob Q-c allows, and its completing page settles the request and mints the create job in a single transaction — which required extracting `create_with_request_key`'s body into a transaction-taking helper, because the two-statement version leaves a window where the job exists and the request still reads `draining`, and a device that gets one answer to its page does not retry a 200.
The coverage aggregation landed per resource rather than per request, in migration 0053's four nullable columns: a request-level total can say a migration lost eleven terms but not which product lost them, and the seller's next action is always about a product.
Nullable rather than defaulted to zero, because a resource nothing measured and a resource measured at zero are different facts and this is the number the founder's kill gate compares.
`tam-import`'s operator binary went behind a default-on `operator` feature in the same slice, so `tam-api` can depend on the library without a marketplace adapter entering its graph; that is half of `engine-driver-split.md`'s open question 9, and the tree assertion it recommends is now possible for the server binaries.
C7, the console, follows C4's route.

Gated on a founder probe: enumerate once from the device and compare the count against the Tes dashboard, because the live run of 2026-08-28 saw both dashboard routes answer an empty array with HTTP 200 on an authed session, suspected to be site-context scoping.
Whatever that probe answers, the interface must never render an empty enumeration as "no listings", because an empty shop and a failed read are indistinguishable and the wrong one of the two is far more expensive.
The device pass already keeps them apart: a refused catalogue is its own failure that posts nothing, and an empty one still posts a completing page, so a seller is never left watching an import that cannot end.

One dependency is worth stating plainly rather than leaving to be discovered.
S3's pass downloads every bundle in order to measure it, which is the leg the redirect work left failing closed until the marketplace-named hop could be re-issued.
That re-issue is built, so the dependency is discharged; before it was, S3 could be built and proven against cassettes but could not have imported a real catalogue at all.

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

All taken. Each row states the decision, what was decided, and what taking the alternative would have cost.

| # | Decision | Recommendation | Status |
|---|---|---|---|
| Q-a | Is a digest reported by the device acceptable integrity on a first observation, where the server holds no independent commitment? | Yes. The device's report is recorded with its asserted instant beside our receipt, and a later disagreement is surfaced rather than overwriting. The alternative considered and not taken was two independent fetches whose agreement is the check, which doubles the transfer for a property the seller's own session already largely provides. | decided yes, 2026-09-04 |
| Q-b | Is a malware scan performed and asserted by the device acceptable, given the server never sees the bytes? | Yes, and advisory rather than a guarantee we make. The bytes travel from the seller's own account to the seller's own account and we are never a distribution point for them, so what the device asserts is recorded as the device's assertion and is not restated by us as a clean bill. | decided yes, 2026-09-04 |
| Q-c | May the device upload a derived cover image, as the one exception to keeping bytes off our servers? | Accept: a fixed 512 by 384 PNG per product, so the console has an image and the existing cover gate stands unchanged. The alternative is to make that gate target-conditional and show no image in the console. | adopted by silence, 2026-09-04 |
| Q-d | May payload bytes be cached on the device between the import pass and the publish? | No for now. Discard on settle, the drop backstop and the start-up sweep are what make "the file is still the seller's" true after a crash. Revisit if the second fetch of a large resource is felt, with a seller-visible toggle and a size ceiling. | adopted by silence, 2026-09-04 |
| Q-e | Is a migration's payload the bundle whole rather than its exploded entries? | Accept, unwrapping only a bundle that holds exactly one entry. TPT takes exactly one file, and the bundle is what the seller's Tes buyers already receive. | adopted by silence, 2026-09-04 |
| Q-f | Does the migrate flow refuse to start without the TPT copyright declaration? | Accept. It converts step 14's open item from a whole catalogue of terminal failures into one gate the seller can act on. | adopted by silence, 2026-09-04 |
| Q-g | Does a Tes-sourced TPT upload require the Tes entitlement grant as well as TPT's? | Yes. The run makes a Tes request, and the kill switch must be able to stop it. | adopted by silence, 2026-09-04 |
| Q-h | Is a Tes-to-Etsy migration refused in the first pass? | Accept the refusal. Satisfying the sanctioned branch would mean routing the seller's bytes through us, which is the arrangement D27 removes. | adopted by silence, 2026-09-04 |

One item is flagged rather than decided, because it belongs to an owner other than this note.
Whether the observed hash and kind columns are nullable-until-observed or provisionally filled with a later correction is a schema call to take with the migration in hand; the provisional form is the lighter of the two.

A second item that stood here is now answered and is recorded rather than dropped.
It asked whether the broker gateway leaked the seller's cookie on the same CloudFront redirect.
It did not: reqwest strips `Cookie` and `Authorization` on a cross-host hop, so the gateway's own header was covered, and its deletion needs no audit on these grounds.

## Sources

- `docs/design/decisions.md`, "The two uncaptured endpoints, resolved by a founder-supervised capture, 2026-08-28".
- `docs/notes/design/vendoo-for-teachers-rethink.md`, decisions D1, D7, D27, and the re-baselined phase table.
- `docs/notes/design/desktop-data-plane.md`, "The interim payload fetch, and what it owes".
- `docs/notes/design/engine-driver-split.md`, steps 10a, 10b, 14 and 15, and open questions 3 and 4.
- `docs/notes/design/tpt-vocabulary-rebase.md`, for the mapping this consumes unchanged.
- `docs/research/rethink/tpt-create-form-dom.md` and `tpt-product-model.md`, for TPT's file slots, caps and extension count.
- `docs/research/rethink/cross-marketplace-mapping-tpt-base.md`, for the Tes file constraints and the projection table.
