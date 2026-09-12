# Import reliability and guides repair

- Status: approved by the founder; implementation in progress.
- Created: 2026-09-12.
- Scope: repair the reported APK imports and guides before Phase 7.
- Implementation plan: [bounded execution tasks](plans/2026-09-12-import-reliability-and-guides-repair.md).
- Governing design: [seller workflows](../notes/design/2026-09-12-one-marketplace-per-site-and-the-seller-workflows.md), especially import, device origin, publishing consent and both client layouts.

This document is for an implementer or reviewer with no incident context.
It separates observed failures from mechanisms that can cause them, then specifies the repair and its release gates.
It does not assert that any repair has shipped.

## Evidence and diagnosis

### Observed on the deployed 0.8.0 clients

A read-only production transaction sampled at 2026-09-12 15:43 UTC found six recent marketplace imports: five TES runs abandoned by the seller and one TPT run still `reading`.
Every run had zero items, `read_total = NULL`, and no recorded device failure.
Their events were `JobQueued`, followed only by the seller's abandonment where applicable.
The recent TES attempt ran from 14:00:22 until the seller stopped it at 14:07:44; TPT was opened at 14:08:01.
The catalogue contained one existing resource, not six newly imported resources.

The origin access log for 13:55–14:15 UTC contained no device import submissions.
Run creation returned 201 in approximately 6–7 ms; duplicate starts returned 409; run-detail reads returned 200 in approximately 2–5 ms.
This rules out a loop of import pages being rejected by the origin in that window, but does not rule out a request failing before it reached the origin.

The unrevoked Samsung SM-F926B reported both TES and TPT sessions.
The unrevoked Samsung SM-N975F reported neither.
These are different phones, not evidence that the application spontaneously regenerated one device identity.
The screenshot's local-session refusal is consistent with the latter phone showing the former phone's organisation-wide connection standing.
There is no evidence here that either marketplace session expired.

The screenshots show an import with only “Live” and “Under way”, an unrelated-source conflict, a local-session refusal beside a connected tile, and an HTML-as-JSON error beside `healthz 200`.
They also show misaligned guide creation fields, a saved-only preview, and a guides index without search or taxonomy.

### Confirmed mechanisms in the current source

| Finding | Source and consequence |
|---|---|
| Accepted is displayed as running | `ImportRunRepo::create` in `crates/tam-storage/src/import_runs.rs` inserts `reading`; `runBadge` in `web/src/lib/pages/import/run-view.ts` labels it “Under way” before any worker acknowledges it. |
| Startup failures are not durable | `source_of`, `ready_to_import` and `start_import` in `apps/desktop/src-tauri/src/commands.rs` return errors to the invoking page without recording failure on the already-created run. |
| The first half depends on a long-lived UI promise | `start_import` awaits enumeration and listing submission; navigation can lose its eventual error. |
| Progress is delayed or discarded | `ImportPass` in `apps/desktop/src-tauri/src/import.rs` posts the whole enumeration only after it finishes; its five-minute enumeration budget is a ceiling, not a normal duration. Description batches hold up to 25 items, and both live callers discard `ImportProgress`. |
| Nothing expires ownership | `0070_import_runs.sql` and `0071_schedules_and_sync.sql` contain no worker lease, attempt fence or run heartbeat; the scheduler does not settle silent runs. |
| Stop does not stop device work | `DesktopState.imports` is an in-memory claim set, not cancellation ownership; `abandon` records server state without interrupting the device pass. |
| TES blocks TPT by construction | `import_run_one_open_per_org` permits only one open import of any source per organisation. `device_open_run` and scheduler callers also assume a single open run. |
| Connected is not local readiness | The import tile uses organisation-wide connection standing; `ready_to_import` reads the current device's sealed session store. |
| “Live” measures the wrong thing | `RunPage.svelte` derives it from EventSource connectivity, not worker contact or useful progress. |
| Opening a run causes redundant reads | `createLedger` starts at cursor zero; subscribers refetch for every replayed event while testing the permanently nonempty event array. The production bursts match this code path. |
| HTML is misclassified as no network | `request` in `web/src/lib/api.ts` lets successful-response JSON parsing throw; `describeUnreachable` classifies that exception as a network failure and loses the HTTP status. |
| Guide fields align their outer bottoms | `.gd-head` in `web/src/lib/pages/guides/guides.css` uses `align-items:flex-end`; only Address has a trailing `Field` hint. |
| Preview is saved-only | The guide editor renders `saved.html`; it deliberately refreshes only after saving. |
| Naive autosave would lose text or publish drafts | The save callback replaces the edit buffer, storage is last-write-wins, and the same body currently serves editor and reader. |
| Requested Markdown has two security dependencies | `guides::render` escapes raw HTML but does not allowlist URL schemes; both console and packaged-client CSP currently reject external image hosts. |

### Executed diagnostic experiments

- Actual local guide creation page, 1280 × 900: Title input top 320.484 px; Address top 296.734 px; mismatch 23.75 px, matching the hint line plus field gap.
- Actual client request function with an intercepted 200 `text/html` response: the result was `kind: network`, `status: null`, with the same `Unexpected token '<'` error as the screenshot.
- Actual local run page, one mount: 34 run-detail GETs in a 1.8-second observation window and one stream opened with `cursor=0`.
- `adb devices -l` exposed only the local emulator; neither founder phone was available for a native trace.

The precise TES interruption remains unknown: failure before enumeration, an enumeration timeout, a blocked native call, or Android suspension can all leave the same empty ledger today.
Do not label any one of those the incident's proven root cause.
Do not change Cloudflare, DNS or HTTP/3 settings on the basis of the HTML parsing screenshot.
The first implementation gate must make the native failure observable and exercise the reported APK path.

## Requirements and terms

An import is a seller-requested read that adds or reconciles catalogue resources; it does not implicitly publish them.
A source is a marketplace account attached to the organisation; this repair retains the existing one-account-per-marketplace model.
A unique import means one logical open run per organisation and source, independent of which device owns its current attempt.
An attempt is one device's time-bounded authority to advance that run.
A publish-ready resource has the required metadata and files accessible on the executing device for the chosen destination; a metadata-only import is not publish-ready.
A guide topic is its optional primary help category; guide tags are cross-cutting help labels, unrelated to tenant-private product labels.

The environment can interrupt a phone process, invalidate a session, change marketplace content, or lose a response after a successful write.
Neither a live browser stream nor a historical device connection proves current marketplace access.
The design must expose these conditions rather than assume they cannot occur.

Required outcomes:

1. Small TES and TPT shops reach an explicit catalogue result, or a specific recoverable/terminal failure; no unexplained indefinite running state.
2. TES and TPT can run independently; repeated starts of the same source lead to the existing run, not another worker or another product.
3. The seller sees the stage, useful counts, last contact, last progress and available action; percentages have a real denominator.
4. Stop, page navigation, app suspension and restart cannot create hidden continuing writes or duplicate catalogue resources.
5. Import status distinguishes catalogue completion from destination publishing readiness.
6. Guides support aligned fields, topics, tags, the requested Markdown insertions, live preview, safe draft autosave, and searchable/filterable published content.
7. All changed surfaces work in the browser, Windows app and APK where applicable.

## Selected recommendation and alternatives

Recommend a narrow device import supervisor using the existing run ledger, plus a revision-safe guides editor using the existing renderer.
A timeout/progress patch alone is insufficient: it leaves ownership, cancellation and concurrent duplicate creation unresolved.
A general workflow engine, new queue, Android background service, rich-editor framework, search service or image proxy is unnecessary for this scope.

“Async” means independent jobs owned by the native process rather than a page promise.
On Android, this design guarantees explicit suspension and safe resumption, not uninterrupted work while the operating system suspends the app.
The UI tells the seller to keep the app open during reads.
If uninterrupted screen-off execution is required, that is a materially different design requiring explicit approval.

## Import contract

### Starts, readiness and ownership

- Add a client-generated, client-retained start idempotency key; the current start API does not supply one.
  Persist its organisation/source binding and resolve replay before checking for another open run.
  The same key returns the original run even after settlement; conflicting source/payload reuse is refused.
- Replace organisation-wide uniqueness with one open marketplace run per `(org, source)`; spreadsheet uniqueness is per batch identity, not a shared null-source slot.
- A same-source conflict returns the existing run and the console opens or links it.
- Manual APK starts check actual local session availability; browser starts explicitly select or wait for an eligible device.
- Show “Connected on this device”, “Connected on another device”, “Reconnect here”, and “Waiting for a device” as different facts.
- Local presence is not a promise that the marketplace will accept the next request; expiry and challenge responses remain distinct actionable failures.
- Record run ownership before marketplace work: owner device, monotonically increasing attempt fence, lease expiry, last contact, last progress and bounded stage/reason information.
- Use database time for claims and expiry; proposed operating values are a renewal every 15 seconds and a 60-second lease.
- Every device mutation checks organisation, run, source, owner, current fence, unexpired lease and permitted phase.
- Renewals and cancellation observation execute independently of enumeration and publishing work.
- An explicit manual start that has not been claimed within two minutes fails with an activation reason and a retry action.
- A scheduled run waiting for a device is visibly waiting, not running; it need not fail merely because the phone is closed.
- An expired active lease becomes interrupted/waiting with its checkpoint intact; it never remains presented as active work.
- Reclaim requires fresh local readiness and a new fence; takeover by a different device is explicit.

Preserve the existing catalogue lifecycle states where practical.
Add explicit enumeration completion and a frozen selected total so partial listing pages cannot be mistaken for a completed selection list.
Derive the display stage from those facts and the catalogue lifecycle; derive waiting/interruption from ownership and reason, not from elapsed time alone.
Do not introduce independent, contradictory authoritative state machines in the console and server.

### Native execution and failure

Replace the import claim set with a small supervisor holding one cancellation handle per active run.
Both manual commands and resumed/scheduled work enter that same supervisor.
Start and continue return after durable acceptance; their results do not wait for the marketplace walk.
Use the existing device persistence pattern for checkpoints and unacknowledged metadata pages; no new database or runtime dependency is required by this design.

A failure reporter must work before a catalogue adapter/pass exists.
Persist preflight, unsupported-client, source lookup, enumeration, description, permission and submission failures through an authenticated run operation that does not require the missing marketplace session.
Only the nominated/current owner or initiating actor may affect that run; an unrelated idle device cannot fail another device's run because it lacks a session.
When the control plane is unreachable, retain the failure locally and report it on reconnection; lease expiry supplies truthful server-side interruption in the meantime.

Keep existing marketplace request and enumeration bounds unless evidence and founder approval justify changing them.
Make native blocking work cancellable or place it behind a bounded blocking seam; first isolate any actual JNI blocking fault rather than rewriting secure storage speculatively.
Do not add automatic retries for authentication, challenge, permission or unsupported-operation failures.
Retry only replay-safe reads/submissions under bounded policy.

### Replay, counts and cancellation

Use stable logical page/item receipt keys independent of attempt number.
Equal key and equal content returns the previous acknowledgement; equal key and different content is a conflict, not an overwrite.
Receipts, accepted catalogue effects and durable counters update transactionally.
Resume from a cursor only where the captured marketplace contract supports it; otherwise re-enumerate safely and reconcile stable source locators.
Never treat mutable page offsets as a marketplace snapshot.

Expose discovery pages/items, selected total, processed outcomes, additions, existing matches, unresolved review, failures, and file readiness separately.
Freeze the selected denominator when selection is accepted; do not use the remaining `selected` state count as a total.
Do not inflate counts on replay or count a failed read as successfully imported.
During discovery show counts found and activity, without a percentage unless the source supplied a trustworthy closed total.
During reading and committing show a stage-local percentage over a frozen denominator.
A run can finish with skipped/failed items, but its summary must name them rather than show unqualified success.

Report meaningful per-item/stage progress independently of payload batching.
Keep the bounded maximum batch size and flush partial work promptly; a six-item shop must not wait for a 25-item threshold to expose progress.
Worker contact, meaningful progress and browser-stream freshness have separate timestamps/labels.
Do not show a fake overall percentage or ETA.

Cancellation atomically invalidates the run's fence and is ordered against its catalogue commit transaction.
After cancellation is acknowledged, no later catalogue commit for that run may succeed.
Effects committed before that acknowledgement remain visible; cancellation is not rollback.
Stop native work promptly and reject its late pages.
If offline, say “Stopped on this device; server confirmation pending” until acknowledgement.
A terminal run is immutable; a deliberate retry creates a linked new run, while an interrupted run may resume under a new attempt.
The existing 30-day duplicate-merge undo remains an explicit audited compensating action against committed catalogue effects, not a resumption or mutation of the terminal import's historical outcome.

### Parallel reads without duplicate products

Do not land the per-source unique index alone.
The matcher currently sees committed products, so concurrent TES and TPT reads can both initially decide that the same resource is new.

Allow independent marketplace reads, but serialize the short catalogue decision/commit section per organisation.
Inside that transaction, re-read candidates and revalidate the existing duplicate verdict before creating or binding a product.
The second commit must see the first commit's product and either safely match it, ask for a decision, or establish that it is distinct under the existing rules.
Include spreadsheet and scheduled import commit callers in this same rule.
Import-associated duplicate `decide`, `merge`, `unblock` and `undo` paths participate in the same organisation transaction and run/cancellation guard.
A late review answer cannot change an abandoned run or race the drain outside that guard.
An undo of committed effects records a separate compensation and does not reopen the import; undo/decide against pending work must still satisfy its open-run guard.

After the seller confirms catalogue addition, the commit command durably transitions the run and returns acceptance.
Persist manual confirmation separately from description completion, including its actor and time.
The current `committing` state can be reached before the seller confirms addition, so that state alone is never drain authority.
Backfill existing manual runs as unconfirmed; preserve scheduled-rule authorisation as a distinct explicit fact.
The existing server worker drains all eligible manual and scheduled committing runs in bounded chunks, resuming after process restart.
Do not retain a browser-driven commit loop whose continuation depends on the page remaining open.
Device leases govern discovery/description submissions; server commit authority comes from the confirmed run state and transaction guard, not from a phone remaining online.
Publish scheduling still requires its existing separate authorisation and is not implied by catalogue commit.
Retain mapping uniqueness constraints as the final guard.
No database lock may span marketplace, filesystem or image/fingerprint I/O.

Do not broaden matching to speculative pending-product identities when a short commit lock and revalidation provide the needed guarantee.
Do not strengthen fuzzy matching to auto-merge TPT metadata just to make a duplicate count smaller.
Migrate every `open(org)` singleton caller, including scheduled pulls, scheduled completion and device work discovery.
A paused duplicate review for one source must not block a different source; its own source remains linked to the outstanding review rather than spawning duplicates.

### Console and response handling

Use the run snapshot as truth and ledger events as invalidation signals.
Coalesce event bursts, react only to new relevant event sequence/resync, and preserve an organisation-scoped cursor across remounts.
Invalidate that cursor on logout or organisation change.
Retain the existing missed-event/resync behaviour; do not skip the snapshot/stream race by starting arbitrarily at the newest event.
Migrate every caller of a changed shared ledger interface.

At the JSON response boundary, retain status, content type, requested route and safe redirect/final-route information.
A non-JSON response is a protocol/unexpected-page failure, not a no-network failure.
Do not log cookies, full HTML bodies, signed URLs or query-string credentials.
Neither `healthz 200` nor valid JSON alone establishes that a particular API response satisfied its contract.

## Import-to-publishing contract

Keep no-API marketplace requests, sessions and imported resource-file bytes on the seller's device.
For those imported files the server retains approved metadata, source locators, digests and sketches, not payload bytes.
The session store, `SessionTransport`, `HttpControlPlane`, entitlement gate and revocation path are shared with publishing; changes there require publishing-path regression coverage.

Catalogue completion must persist the source binding and the file-source descriptors required by the existing publish path.
Before publication, revalidate that the executing device can resolve the required bytes and that they still match the recorded digest/version.
A changed source file or unavailable file produces an explicit readiness blocker, not reuse of stale approval.
Missing target fields, permissions, session, file-download capability or destination support are separate blockers with a next action.

TPT own-file download remains explicitly uncaptured in the governing design.
The available `downloads.har` is a TES capture; inspection of `tpt-capture-options.har` found no download/bundle endpoint or resource-payload GET, only font GETs among its binary responses.
Those candidate captures do not discharge the TPT own-file gate.
Do not claim automatic TPT-to-TES publication from imported resources works until that capture is implemented and verified on-device.
The existing manual attachment path is not a local-only fallback: `FilesPanel` calls `api.upload`, which posts bytes to `/v1/uploads`, and native `payload.rs` retrieves those originals from server object storage.
Preserve that separately consented existing upload behaviour, but do not route marketplace-imported bytes through it to bypass this capture gate.
A genuinely device-local original-file attachment mode would be a separately approved contract, not an existing capability or a hidden addition to this repair.
The plan includes the capture gate; it is not silently deferred behind a metadata-only success label.
Import itself never performs a new marketplace publication or source deletion.
A real publish smoke requires separate seller authorisation and an independently observed destination result; an ambiguous external write must reconcile before retry.

## Guides contract

### Content, taxonomy and publication

Use the existing global guides module and operator permissions.
Add an optional single topic and multiple tags, managed in the guides backoffice and distinct from product labels.
Use stable identifiers, unique normalised slugs and existing bounded name/request validation conventions.
Do not silently assign existing guides to an invented topic.
Retire referenced taxonomy entries or refuse deletion with an explanation; never cascade-delete guides.
An explicit taxonomy rename changes its displayed name globally; an autosaved guide edit does not change published taxonomy assignments.

Preserve a complete working draft separately from a complete published snapshot: title, body, topic and tags all belong to both.
Use a monotonically increasing aggregate revision for optimistic concurrency, not timestamp equality or a short content hash.
Every draft save, publish and unpublish checks the expected revision and returns the new revision.
Backfill published guides from their existing content so URLs and visible text survive migration; draft guides remain unreadable to readers.
Do not add arbitrary revision-history retention: one working copy and one published snapshot meet this request.

Autosave writes only the working copy, after a short debounce, with one request in flight and the newest unsaved edit queued.
Never replace newer local text with a save response.
A conflict or failed save preserves the edit buffer and exposes an action; it never silently overwrites another operator.
A best-effort blur/visibility flush is not a guarantee against process termination; visibly distinguish saved, saving and unsaved work.
Keep an explicit Save action and a navigation warning for unacknowledged work.
Publish saves/awaits the exact current edit and publishes that acknowledged revision, not whichever revision happens to be latest when a request arrives.
Unpublish is explicit and makes the reader route unavailable while retaining the working copy.

A lost save/publish/unpublish response leaves the write outcome unconfirmed, not necessarily unchanged.
Pause queued mutations and perform a bounded authoritative read-back without replacing the local edit buffer.
Compare the sent draft, expected revision transition and intended published-source revision/visibility with that read-back before adopting its revision.
Resume queued edits only when the prior outcome is reconciled; otherwise show an explicit conflict or unconfirmed publication state.
If read-back is unavailable, retain the buffer and uncertainty rather than retrying blindly or claiming that publication failed.

### Editor and Markdown

Repair the guide form locally: align field labels/controls independently of optional hint height, with actions in their own predictable alignment.
Do not turn a two-field defect into a repository-wide form redesign.
Preserve desktop side-by-side editing and preview; stack or switch panes on phone without horizontal overflow.

Keep Markdown as the source and the existing Rust `pulldown-cmark` pipeline as the sole renderer.
Add toolbar actions for heading/subheading, link, URL image, and footnote alongside the existing formatting and uploaded-image controls.
Footnote insertion uses a fresh unused numeric reference/definition identifier.
The pinned pulldown-cmark 0.13.4 assigns numbers on first encounter of either a definition or a reference, so its option alone does not guarantee reference-order numbering.
Add a body-size-bounded event normalisation that moves definition blocks after the main prose in reference order before using the existing renderer.
Preserve reference identities, repeated references and unreferenced definitions; test definitions placed before prose and in reverse order.
Do not add heading-ID syntax, a WYSIWYG model or another Markdown parser for this request.

Provide a side-effect-free authenticated preview operation using the same rendering and validation policy as publication.
Debounce preview independently of autosave so invalid or unsaved work can still be inspected.
Tag requests with an editor generation; stale responses cannot replace a newer preview.
Do not hash the text with a collision-prone short hash as a correctness fence.
A preview response never saves or publishes anything.

Allow only explicitly safe link/image destinations in the parsed Markdown event stream; continue escaping author-supplied raw HTML.
Keep ordinary same-tab links; no custom raw-HTML anchor emitter is needed merely to open a new tab.
Support root-relative guide/image links, fragments, safe web links and appropriate mail links; reject script/data/file/protocol-relative URL tricks as applicable to each destination type.
URL images use HTTPS and load directly in the reader's browser, not through a server fetch/proxy.
Make the third-party image privacy consequence visible to the operator and suppress referrer disclosure.
Widen only `img-src` in both server and packaged-client CSP to support HTTPS images; do not widen script or connection permissions.
The renderer, preview, reader, CSP and installed APK must agree before this feature is accepted.

### Reader discovery

Search published title and body text and published taxonomy names; filter using published assignments only.
Implement this through the existing database/API, with no search service or new dependency.
Use case-insensitive literal text search for this bounded help corpus; escape SQL wildcard characters instead of treating the query as a pattern language.
The semantics are search AND selected topic AND any selected tag; no selected tags means no tag restriction.
Name the tag rule in the UI and preserve query/topic/tags in the URL for reload, back navigation and sharing.
Return a clear empty state with Reset filters, and keep draft content out of results, counts and suggestions.
Use existing query/cache/control patterns rather than mixing server text filtering with a second contradictory client-side filter implementation.

## Review dispositions

A slow-model advisory pass supports narrow supervision, fenced ownership, short commit revalidation, real file-readiness gates, and draft/published separation.
Independent source traces cover the device, ledger and guides slices.
Their suggestions are not implementation authority: this design deliberately rejects pending-item matching as a prerequisite, automatic identity-loss assumptions, timestamp/32-bit-hash revision fences, draft-only title/body separation that leaks tags, a global form abstraction, and upload-only images that fail the requested URL feature.
The advisor's earlier identity-change and SSE hypotheses were superseded by the device-name query and the executed replay experiment above.

The independent import and guides reviews raised seven actionable findings.
This revision addresses all seven: explicit manual commit confirmation/backfill, guarded duplicate-review and undo writes, removal of the nonexistent local-original fallback, durable start idempotency, footnote event ordering, registration of new guide tables in the closed-world tenancy matrix, and read-back reconciliation of lost guide-write acknowledgements.

## Approval and completion gates

Approval covers the bounded design, foreground-safe Android semantics, and direct HTTPS guide images.
It does not authorise production marketplace writes, account/session resets, infrastructure changes or a release tag.
A required new dependency, uncaptured marketplace operation or expanded background-execution promise returns to the founder rather than being silently substituted.

The repair is complete only when every acceptance gate in the implementation plan is evidenced, review findings are resolved, standard verification passes, and the actual changed client surfaces have been exercised.
If TPT file capture or physical-phone access is unavailable, report that gate as blocked and finish all reachable work; do not call the import-to-publishing requirement complete.
Phase 7 remains blocked until the founder accepts the repaired workflows and the applicable release evidence.
