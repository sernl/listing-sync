# Import reliability and guides repair: implementation plan

> The founder approved the linked design and requested implementation.
> Use the executing-plans or subagent-driven-development workflow, with RED-before-GREEN for the reported defects and independent review at integration points.

**Goal:** repair the actual APK import workflow, preserve catalogue-to-publishing viability, and deliver the requested guides workflow before Phase 7.

**Architecture:** a narrow native import supervisor advances fenced runs in the existing PostgreSQL ledger; the console presents authoritative progress; guides retain distinct revisioned draft and published snapshots through one Rust Markdown renderer.

**Tech stack:** existing Rust/Tauri/Kotlin, PostgreSQL/sqlx, Svelte/TanStack, pulldown-cmark, nextest and Vitest.
No new queue, search service, image proxy, editor framework or dependency is pre-authorised.

**Spec:** [approved design and incident evidence](../2026-09-12-import-reliability-and-guides-repair.md).
This file is an execution plan, not evidence that any implementation task has passed.

## Contracts and ownership

The design owns semantics; the following names identify the implementation seams, not extra public abstractions.

- **I1 owns the import protocol:** lease claim/renewal, attempt fence, reason codes, enumeration completion, frozen selection total, progress snapshot and idempotent receipt acknowledgement.
- **I2 consumes that protocol:** native worker lifecycle, cancellation, session readiness and durable replay.
- **I3 owns catalogue commit correctness:** short organisation-scoped transaction, match revalidation, product/mapping effects and cancellation ordering.
- **I4 consumes I1–I3:** independent starts, device work discovery, scheduler integration and truthful UI.
- **G1 owns guide content contracts:** complete draft/published snapshots, aggregate revision, taxonomy identity and read models.
- **G2 owns rendering policy:** same renderer and safe URL handling for preview and publication, plus both CSPs.
- **G3 and G4 consume G1–G2:** operator editing and reader discovery respectively.
- The integration owner alone merges shared `web/src/lib/api.ts`, `crates/tam-api/src/lib.rs`, `openapi.rs`, generated vocabularies and migration numbering.
- Reserve migration `0074_import_execution.sql` for I1 and `0075_guide_revisions_and_taxonomy.sql` for G1 against the currently observed latest migration, 0073.
Recheck the sequence before execution if another change has advanced it; never edit an applied migration.

Dependency order:

- I0 → I1 → I2.
- I1 → I3.
- I2 + I3 → I4 → I5.
- G1 → G2 → G3 and G4.
- I5 + G3 + G4 → V1 → V2.

I2 and I3 can run concurrently after I1's interface is fixed.
The guides chain is independent of the import chain after approval; G3 and G4 can work concurrently under the shared-file integration rule.
The per-source uniqueness cutover must not become active before I3 protects cross-source commits.
All agents skip validation while parallel edits are in flight; the integration owner runs the relevant checks after each convergence and the standard suite once at the end.

## I0 — Preserve the incident and expose native failures

- [ ] Capture the actual APK failure boundary

**Files to inspect:** `apps/desktop/src-tauri/src/{commands,import,heartbeat,state,control_plane,connect,device}.rs`, the existing session modules, `web/src/lib/desktop.ts`, and the import pages.

1. Preserve the design's read-only ledger/request timeline as the baseline, without production identifiers, cookies, file contents or signed URLs in committed fixtures.
2. Add a deterministic regression at the actual command/supervisor-to-ledger seam: the server accepts an import, native preflight/enumeration fails before its first page, and the observer must receive an actionable result instead of indefinite reading.
Run it against the old path and observe the wrong result before fixing it.
3. Exercise the same path with a controlled stalled enumeration, missing local session, lost submission acknowledgement and page navigation.
Do not test a new helper in isolation while bypassing the real command caller.
4. Add only the bounded native attempt diagnostics needed to identify run, source, phase, request category, elapsed time and failure class.
Capture no marketplace response bodies or credentials.
5. On the founder's authorised phone, record the failing stage and response category for the small TES shop.
If neither phone is accessible, proceed with all deterministic repairs and keep the physical-device diagnosis gate explicitly open.

**Acceptance:** each previously silent boundary is distinguishable; the investigation does not claim an unobserved TES timeout, session expiry, TLS panic or identity-reset cause.

**Runner:** existing `tam-desktop` import tests through `cargo nextest run -p tam-desktop -E 'test(import)'`; execute API-observer cases in the existing `tam-api/pg-tests` lane.
The production snapshot is incident evidence, not a substitute for this regression.

## I1 — Add durable ownership and progress protocol

- [x] Persist import attempts and replay-safe progress

**Modify:** `crates/tam-storage/src/import_runs.rs`, `crates/tam-api/src/{import_runs,import}.rs`, `crates/tam-engine-driver/src/import.rs`, existing API vocab/typegen definitions, import-flow tests and `crates/tam-storage/tests/rls_matrix.rs` for any new tenant receipt table.
**Create:** the numbered import execution migration only.

1. Add owner device, attempt fence, lease expiry, last contact, last progress, bounded reason, enumeration completion, frozen selection total and explicit commit-authorisation facts.
Use compound organisation/device relationships and database-time comparisons.
Add durable organisation-scoped start-key/source binding and replay lookup before open-run uniqueness checks; the existing start API always mints a fresh UUID.
Retain start receipts across run settlement so a lost acknowledgement cannot mint another run after expiry/completion.
2. Introduce claim, renewal, progress/failure and resume operations through the existing authenticated run transport.
All mutations validate owner/fence/lease/state; reporting a missing marketplace session must not itself require that session.
3. Persist logical receipt keys and content identity; identical replays return the original acknowledgement, and changed payloads with the same key conflict.
Commit receipts, item outcomes and counts atomically.
4. Expose durable run status sufficient for waiting, active, interrupted, reconnect, selecting, reading, reviewing, committing and terminal displays.
Keep logical stage and execution freshness consistent without a second independent UI state machine.
5. Implement manual activation expiry and active-lease interruption using the existing server maintenance/scheduling infrastructure.
Do not expire a scheduled run merely because it truthfully awaits a device.
6. Backfill existing open runs with no invented progress or ownership.
An old zero-page run becomes visibly unclaimed/interrupted, not falsely complete.
Backfill old manual runs as unconfirmed even if their state is `committing`; retain scheduled-rule authorisation separately.
7. Keep the old organisation-wide open-run fence until I3 and I4 are integrated.
Regenerate sqlx metadata and wire vocabulary at the convergence point.

**Severe RED cases:** unclaimed manual start reaches an actionable activation failure; a healthy SSE connection does not keep a dead lease active; an old device cannot renew or submit after takeover; a lost acknowledgement followed by replay does not increment counts; cancellation/failure of a terminal run cannot be overwritten by a late page; cross-tenant attempts fail.
Also replay a lost start acknowledgement after activation expiry and after completion, and reuse its key with a different source: no new run is minted, and conflicting reuse is refused.

**Acceptance:** the server can explain a run before any item exists, and ownership is enforced rather than advisory.

## I2 — Supervise native imports independently of pages

- [x] Run cancellable imports outside page promises

**Modify:** `apps/desktop/src-tauri/src/{commands,import,state,heartbeat,lib}.rs`, the existing local persistence/session seams only where required, `web/src/lib/desktop.ts`, and native command capabilities if the command set changes.

1. Replace the `HashSet` claim with run-owned cancellation handles and checkpoint/replay state.
Keep the supervisor small and import-specific.
2. Route manual, resumed and scheduled work through it; start/continue return after durable acceptance.
Remove the old awaited-start and separate unobserved scheduled execution paths rather than retaining aliases.
3. Run lease/cancel observation independently of marketplace work; do not hold a session mutex, runtime thread or server transaction across unrelated slow work.
4. Connect listing and per-item progress to the authoritative protocol, including partial-batch flushes; stop discarding `ImportProgress`.
Do not manufacture listing percentages or weaken existing request bounds.
5. Persist/report every early refusal; an idle non-owner device without a session does not fail somebody else's run.
6. Reattach on resume/restart using fenced ownership and acknowledged receipts.
If a marketplace cursor is not stable, re-enumerate safe reads instead of guessing the next offset.
7. Stop local work on cancellation and reject late results; distinguish local-stop/server-pending when offline.
Keep device revocation and source permission checks intact.
8. Read current-device readiness through the existing local session operation; preserve organisation-level connection information as a separate fact.
Do not reset or copy secure sessions between phones.

**Severe RED cases:** TES stalls while TPT still advances; a page reload loses no outcome; a phone process stops and resumes without duplicate catalogue effects; stopping one run does not revoke the device or stop the other source; old-fence callbacks cannot write; a missing local session produces the correct local-versus-other-device explanation.

**Acceptance:** both sources have independent native task ownership, truthful failure delivery and bounded cancellation under foreground-safe Android semantics.

## I3 — Serialize catalogue decisions without serializing reads

- [x] Revalidate duplicate matches inside atomic commits

**Modify:** `crates/tam-api/src/{import_runs,duplicates}.rs`, existing matcher/fingerprint/storage seams, `crates/tam-api/src/import_batch/*`, and scheduler commit callers.
Retain existing identity/mapping constraints.

1. Establish one short organisation-scoped catalogue decision/commit transaction used by marketplace, spreadsheet and scheduled imports.
Make the confirmation command durably request committing and return acceptance; extend the existing server worker to drain manual as well as scheduled runs in bounded chunks.
Persist manual confirmation actor/time separately and require that fact in the manual drain predicate; description completion alone, including an old `committing` row, authorises no addition.
Scheduled imports use their existing explicitly approved rule rather than fabricated manual consent.
Remove the browser-owned commit continuation loop.
Device lease expiry must not stop an already-authorised server catalogue commit; run state and the cancellation guard remain authoritative.
2. Re-run/revalidate matching against currently committed products within that transaction before creating or binding anything.
A previously calculated “new” decision cannot bypass a product committed by another source in the meantime.
3. Keep metadata, source binding, fingerprint, marketplace label, receipt and item outcome consistent in the transaction.
4. Define cancellation ordering through the run guard: cancellation acknowledged first rejects the commit; commit first remains visible and cancellation stops later work.
Include import-associated duplicate `decide`, `merge`, `unblock` and `undo` in that transaction and guard.
Keep the existing 30-day undo as a separate audited compensation for committed effects without reopening the historical terminal run.
Pending review/undo may advance items only while its run guard permits it.
5. Preserve seller review for moderate evidence and existing field-winner decisions.
A TPT metadata-only candidate is not made auto-mergeable by relaxing the matcher.
6. Cover duplicate-review resumption in scheduled imports so resolving/parking a question cannot leave the scheduler on an unexplained dead end.

**Severe RED cases:** concurrent TES/TPT commits for a resource with sufficient identity evidence yield one catalogue product with both bindings; weak evidence yields a visible review, not a false auto-merge; spreadsheet/marketplace overlap obeys the same rule; lost commit responses do not create another product; cancellation races have the defined ordering.
Also navigate away immediately after confirming addition and restart the server mid-commit: both runs resume to the same reconciled outcome without a page driving them.
Finishing descriptions without confirmation creates no products; cancellation versus review and drain versus review obey the same ordering, and post-completion undo leaves the old run settled.

**Acceptance:** concurrency cannot trade the original blocking bug for duplicate products or overwrite an unrelated live listing.
No lock spans marketplace/file/fingerprint I/O.

## I4 — Cut over independent runs and truthful console state

- [x] Enable independent imports and accurate status

**Modify:** import storage open-run queries, `crates/tam-api/src/{import_runs,scheduler}.rs`, native open-work discovery, `web/src/lib/pages/import/*`, `web/src/lib/ledger.ts` and affected ledger consumers, `web/src/lib/{api,unreachable}.ts`, and their existing regression tests.

1. Replace every single-open-run assumption with source-specific or explicit-list access.
Manual starts, device discovery, scheduled pulls, scheduled finish and spreadsheet handoff must agree.
2. Activate per-source marketplace uniqueness only now; spreadsheet deduplication remains per batch.
Repeated starts link to the existing run, while TES and TPT can advance together.
Retain the new start key across client retries and native handoff; only an explicit new import intent creates another key.
3. Render authoritative stage, counts, frozen stage denominator, last meaningful progress, current owner/contact and next action.
“Live” must not imply worker activity; interrupted, waiting, reconnect and completed-with-errors are explicit.
4. Show current-device versus other-device connections and refresh them on connect, resume, sign-out and revocation.
5. Change ledger invalidation to new relevant event/resync signals, coalesce replay bursts, and resume from an organisation-scoped cursor without losing the snapshot/stream race.
Migrate `RunPage.svelte`, `RequestPage.svelte`, sync detail, resource detail and reconciliation wherever the shared interface changes.
6. Preserve status/content type/safe route context for unexpected API pages; distinguish HTML/JSON contract failures from `fetch` network errors.
Remove false DNS/VPN certainty; do not expand this into an infrastructure rewrite.

**Severe RED cases:** a live event stream plus dead worker shows interruption; stream failure plus healthy worker does not falsely fail the run; mounting a run with historical events does not refetch once per old event; genuine new progress still arrives after reconnect; HTTP 200 HTML retains its status and is not called unreachable network; another source remains startable while the first is in review.

**Surface proof:** actual browser and APK at 1280 and 390 logical widths, including selection, stop, resume, offline state, duplicate-start navigation and both active sources.
Measure network requests and observe rendered states; do not use source-string tests for the replay defect.

## I5 — Prove imports can feed explicit publishing

- [x] Verify imported resources through publishing readiness

The physical-phone proof reconciled both marketplace imports. The TPT run committed one selected resource, skipped 153 unselected rows, and acquired its 14,110,742-byte ZIP through the captured device-local source path. A renewed TES session created one uniquely marked TES draft from that imported resource, the device adapter completed its destination read-back, and an independent TES catalogue read found the marker. The cleanup removed only that draft; a fresh catalogue read returned the original four resources and no marker. Importing alone issued no marketplace write or deletion, the source TPT listing was left untouched, and the proof never published a live listing.

**Modify only where the exercised path requires:** existing catalogue import/source-binding, file-source resolution, `apps/desktop/src-tauri/src/{marketplace,work}.rs`, publish readiness/migration preview and the TPT/TES adapters.
Reuse the existing marketplace-source descriptors and fingerprint pipeline; the manual upload path stores originals on the server and is not a device-local acquisition fallback.

1. Exercise imported resources through the real destination readiness calculation, not a metadata-only API fixture.
Verify catalogue fields, source mapping, file descriptors and execution-device availability.
2. Add explicit readiness blockers for missing files, inaccessible source bundles, changed digests, missing target fields, unsupported verbs, absent sessions and permissions.
Do not mark every successful metadata row publish-ready.
3. Obtain the founder-supervised TPT own-file capture named in the governing design; implement only the captured operation, sanitize its fixture, and verify acquisition on the device.
If it remains unavailable, keep this gate blocked; do not substitute the server-upload path or silently add a new local-original storage mode.
4. Exercise TES imported-file resolution and captured TPT own-file resolution through the same publishing path.
Invalidate readiness when bytes or source version differ from the recorded evidence.
5. With separately authorised test resources and destination operations, perform explicit publication and independently verify the listing and file outcome.
Reconcile an ambiguous write before retry; never publish or delete a real seller resource merely to test the repair.
6. Verify that importing alone issued no marketplace write or deletion.

**Severe cases:** lost file, changed source content, another device lacking bytes, stale destination session and ambiguous publish response each prevent false success; rerunning an import does not enqueue a duplicate publish.

**Acceptance:** actual TES and TPT catalogue imports have reconciled results, and each claimed crosslisting path has on-device file and destination evidence.
Physical-phone access, the TPT capture and live-write authorisation are explicit external gates, not reasons to leave reachable work unfinished or to claim metadata-only completion.

## G1 — Separate guide drafts and published taxonomy

- [x] Add revisioned guide content and taxonomy

**Modify:** `crates/tam-storage/src/guide.rs`, `crates/tam-storage/tests/rls_matrix.rs`, `crates/tam-api/src/guides.rs`, guide routes/OpenAPI, web API types and existing guide API tests.
**Create:** the reserved guides migration.

1. Add guides-specific topic/tag records with stable identifiers and normalised unique slugs; implement operator list/create/rename/retire operations.
Use existing operator authorisation and name/request-size conventions.
2. Add working topic/tag assignments alongside the existing working title/body.
Add published title/body/topic/tag assignments, publication time and published source revision.
Choose a join table for guide/tag assignments with a draft/published discriminator; retain referential integrity and atomic snapshot replacement.
Register the topic, tag and assignment tables in the closed-world `GLOBAL_TABLES` registry.
Keep guide operations on the existing application pool behind operator authorisation; grant no additional backoffice database access.
3. Use the guide's existing immutable identifier together with one monotonically increasing aggregate revision.
Draft save, publish, unpublish and delete require `expected_id` and `expected_revision`; stale or replaced writes return a conflict with the current identity and revision without discarding the client's buffer.
4. Migrate currently published content into an identical published snapshot and leave never-published guides unavailable to readers.
Preserve slugs and current visible content; existing guides start without invented topics/tags.
5. Publish copies every field and assignment from the exact acknowledged draft in one transaction.
Reader updated time, search text, counts and taxonomy come from that snapshot, not from autosave timestamps.
6. Replace the old unconditional update path and every caller; do not keep a route that bypasses revision checks.
Retain the draft/published visibility invariant and make unpublish explicit.

**Severe RED cases:** a draft save on a published guide changes neither reader prose nor its tags/topic/search result; two operators cannot silently overwrite each other; deleting and recreating one slug cannot admit a stale editor's write; stale publish cannot publish somebody else's content; draft routes and search remain inaccessible; taxonomy retirement does not delete content; migration preserves an existing published page.

**Acceptance:** saving a guide is distinct from publishing it, for content and taxonomy alike.

## G2 — Share safe Markdown preview and reader rendering

- [x] Add safe preview and requested Markdown

**Modify:** `crates/tam-api/src/guides.rs`, route/OpenAPI definitions, `crates/tam-server/src/main.rs` console policy, `apps/desktop/src-tauri/tauri.conf.json`, and `web/src/lib/pages/guides/guides.css`.

1. Add a side-effect-free operator preview operation through the existing renderer, accepting unsaved Markdown and returning rendered output/validation information.
Use the existing body bound and authorisation.
2. Enable pulldown-cmark footnotes and retain supported tables/strikethrough.
Normalise the bounded parsed event stream so definition blocks render after main prose in reference order; the pinned renderer numbers definitions as well as references on first encounter.
Preserve repeated-reference identity and unreferenced definitions, and cover definitions-before-references in reversed source order with a literal expected numbering.
Do not add a second renderer or heading syntax beyond the request.
3. Filter unsafe destinations at the typed Markdown-event seam before trusted HTML emission; keep raw author HTML escaped.
Test link and image policies separately, including case/control-character and protocol-relative evasions.
4. Permit direct HTTPS image URLs and existing local uploaded images, with no server-side retrieval.
Align server and packaged-client image CSP and suppress image referrer disclosure.
Keep script/connect permissions unchanged.
5. Render identical Markdown through preview and published-reader paths, including footnote reference order and backlinks where supported.
Use independent expected output semantics, not one production renderer to compute another renderer's expected value.

**Severe RED cases:** unsafe Markdown cannot produce an executable navigation; URL images actually load in browser and APK under their CSP; preview does not save or publish; oversized input is rejected; a slow earlier preview cannot supersede newer content at the consumer.

**Acceptance:** the requested URL image and footnote features work safely on the actual surfaces, not just in an HTML string test.

## G3 — Build the revision-safe guide editor

- [x] Align fields and add safe autosaving

**Modify:** both operator guide routes, `web/src/lib/pages/guides/{editor.ts,guides.css}`, existing Markdown insertion helpers where already shared, and web API bindings through the integration owner.

1. Fix the measured 23.75 px mismatch locally by aligning label/control rows independently of optional hints.
Keep the create action predictably aligned and stack the form on phone.
2. Add topic selection and tag association/management using existing controls; keep them guides-specific.
3. Add heading/subheading, link, URL image and footnote insertion while preserving the caret/selection and existing formatting/upload controls.
Use a fresh footnote identifier; visible numbering remains the renderer's responsibility.
4. Add independently debounced live preview with editor-generation fencing.
Show stale/loading/error state without presenting an old preview as current.
5. Add serialized draft autosave, queueing the latest unsaved edit while a request is in flight.
Adopt acknowledgements/revisions without replacing newer local text.
On an acknowledgement loss, pause queued writes and read back the authoritative draft/revision and published-source revision/visibility.
Compare the sent edit and expected revision transition without overwriting newer local text; resume only after reconciliation.
Otherwise expose conflict/unconfirmed publication and retain the buffer; no blind publish/unpublish retry.
6. Keep explicit Save; publish awaits the exact saved edit and checks its revision.
Conflicts retain the buffer and require a deliberate resolution; offline/failed saves show unsaved status and a navigation warning.
7. Make publish/unpublish explicit and show “published with unpublished changes” where applicable.

**Uncertain-edge regressions worth retaining:** typing during a delayed save; out-of-order preview; two-writer conflict; publish while a newer edit is queued; collision with an existing footnote identifier.
Also lose the response after a successful save, publish and unpublish, including a subsequent edit by another operator; no self-conflict, lost text or false visibility claim may be hidden.
Use real browser interaction for routine toolbar and layout proof rather than adding plumbing assertions.

**Surface proof:** 1280 and 390 widths, keyboard editing, caret placement, field geometry, uploaded/URL images, headings, footnotes, network interruption and public-page stability during autosave.

## G4 — Make published guides searchable and filterable

- [x] Add published-guide search and taxonomy filters

**Modify:** published guide list/detail handlers and storage queries, `web/src/routes/guides/+page.svelte`, `web/src/routes/guides/[slug]/+page.svelte`, guide styles and web API bindings.

1. Search published title/body/taxonomy names with case-insensitive literal text matching using the existing database.
Escape wildcard metacharacters; use no external service or speculative search index.
2. Apply search AND topic AND any selected tag in one authoritative query contract.
Do not mix unpublished working metadata into reader results or counts.
3. Mirror query/topic/tag identifiers in the URL and query-cache key.
Preserve back/reload/share behaviour and ignore obsolete responses.
4. Provide labelled topic/tag controls, result count, clear empty state and Reset filters.
Show guide topic/tags on results and detail; keep retired-but-referenced taxonomy intelligible.
5. Reuse the existing responsive controls and list layout, with keyboard and touch operation.

**Severe cases:** draft-only text/tags never appear in published search; literal `%`/`_` do not become SQL wildcards; combined filters obey the documented semantics; reload/back restores filters; no-result state resets correctly.

**Surface proof:** actual reader on desktop and phone with several published guides and an unpublished edit whose unique phrase must remain unsearchable.

## V1 — Resolve review findings and verify the assembled repair

- [ ] Review integrated behaviour and run standard verification

Independent reviews, standard gates and local browser checks passed; the physical-client and live-marketplace acceptance scenarios below remain open.

1. Obtain an independent correctness review of ownership/replay/cancellation/duplicate matching and a separate review of draft isolation/URL safety/editor races.
Use the slow-model advisor for unresolved or ambiguous decisions, not as a substitute for executed evidence.
2. Resolve findings before declaring completion; rerun the relevant failing scenario after each repair.
Retain regressions only where a plausible defect would fail the observable contract; replace touched wording/plumbing assertions rather than re-pinning them.
3. At final convergence run the repository's standard gate from its configured development environment:

```sh
just pre-push
env -u CC -u CXX -u NIX_CFLAGS_COMPILE -u NIX_LDFLAGS \
  PATH="$TAURI_WINDOWS_TOOLCHAIN_BIN:$PATH" \
  cargo xwin check -p tam-desktop --target x86_64-pc-windows-msvc
```

`just pre-push` already covers auth, Rust format/clippy/purity/tests, web typegen/svelte-check/Vitest/build, landing checks, sqlx metadata/database tests and portable targets.
The standard workspace lane includes desktop clippy and desktop tests; do not rerun them merely to restate the same proof.
The Windows command uses the unwrapped toolchain path exported by the default development shell, matching `just desktop-build-windows`.
Do not claim its later lanes ran if an earlier one failed.
Use the real development entitlement public key or the documented unset-key mode, never an all-zero Ed25519 key.
Run the Android recipe from the Android development shell:

```sh
just android-build-debug
```

Build and exercise the release APK with the existing signing procedure at V2; a debug emulator is not proof that the founder's installed release behaves correctly.

4. Exercise the actual flows: small TES shop; small TPT shop; simultaneous different sources; duplicate same source; stop/restart/offline; duplicate review; imported-resource publishing readiness; guides create/edit/autosave/publish/search/filter.
Record versions, device, input count, observed result and remaining blockers.
5. Test the UI at 1280 and 390 widths and verify actual installed-client CSP, session lifecycle and native commands.
6. Update the governing design/decision/release/user-facing material only for behaviour actually accepted and implemented.
Remove throwaway smoke artefacts after the evidence is captured.

**Acceptance:** every requested outcome has a real scenario result, all review findings are disposed, and no green test count is used to conceal an unexercised phone or marketplace path.

## V2 — Deliver the verified client without advancing Phase 7

- [ ] Package and hand over the repaired workflows

1. Use the existing desktop distribution and release-authorisation procedure; decide the version from the accepted scope rather than moving an old tag.
2. Update both desktop version fields, write the release note, run `just release-check`, build the signed Windows/Android artefacts and preserve the established Android signing identity.
3. Verify install/upgrade, current-device connections, import command compatibility and guide images on the shipped artefact.
A changed Rust command, capability or packaged CSP requires a new app build even when the web console updates independently.
4. Coordinate the server/protocol/client deployment as a clean cutover.
Old clients must be stopped with the existing actionable update-required behaviour before they can submit unfenced import work; do not retain an unsafe legacy mutation path.
Drain or explicitly interrupt old work, preserve its data, deploy the server gate and install the compatible app.
5. When deployment is authorised, follow the existing Teachouse production-host pin and download-refresh procedure and observe the published download set.
Do not perform production writes or tag/redeploy during this planning task.
6. Present the founder with evidence and any blocked gate.
Phase 7 stays blocked until the founder accepts the repaired import and guides workflows; a metadata-only TPT result cannot silently discharge the publishing requirement.

## Planning verification record

This plan was grounded in all seven supplied screenshots, read-only production run/session/event/access-log queries, three independent source traces, a slow-model advisory pass, and three actual browser diagnostics recorded in the design.
No source implementation, production import, session reset, marketplace write, release or deployment was performed by this planning task.
The configured `just pre-push` baseline passed: 37 auth tests, 1,444 workspace Rust tests including the desktop crate, 1,922 web tests, and 978 database-enabled tests, plus its format/clippy/purity/type/build/landing/portable gates.
The configured Windows client cross-check also passed.
Initial invocations needed the real development public key and the repository's Windows resource-compiler environment; those invocation errors were corrected without source changes.
These are baseline results against the unfixed implementation, not evidence that the proposed repair works.
The two independent plan reviews produced seven findings; all seven were addressed and both reviewers confirmed closure by targeted reread.
Both slices are ready for founder approval as a proposed design; implementation and actual-client/marketplace evidence remain unperformed gates.
