# Device coordination and bounded UI repair

**Goal:** answer the founder's multi-machine storage and execution questions, repair the reported device and thumbnail failures, and make Guides, Imports, Templates, Sync and Migrations usable as their data grows.

**Authority:** the founder requested planning followed by implementation in this session. Targeted tests and local interface checks are authorised; the full suite, deep end-to-end marketplace validation, deployment and releases are not.

**Architecture:** retain Rust/Tauri execution, organisation-scoped PostgreSQL state and the existing Svelte/TanStack console. TES/TPT requests and original resource files remain on seller devices. No new dependencies, queue, marketplace proxy or design system.

## Selected design

- Use the existing PageHead, Field, Button, Banner and TabBar components and palette tokens. Search is a visible labelled input; dropdowns share its baseline and control height. Filters stack at phone widths.
- Runs show 10 records per page; resource and activity lists show bounded pages, normally 25. Guides and marketplace-import search/filter/order controls apply before pagination. Sync/Migration filters that operate on the displayed page explicitly say so. Page navigation replaces rows rather than growing a list. Background refresh must not discard selection or jump pages.
- Main owns one shared `web/src/lib/Pagination.svelte`: `page` (one-based), `hasNext`, optional `busy`, `label` and `summary`, plus `onprevious` and `onnext` callbacks. Pages own their cursor/offset state; the component does not fetch data or invent totals.
- Current device, recent contact, saved marketplace login and current execution ownership are separate facts. Current-device identity comes from the installation ID, never a display-name match. Older compatible installations remain eligible; version differences alone do not justify hiding them.
- Keep old installation records in explicit history, not the main ready-device list. Do not destructively merge or delete devices that merely share a name. Existing revocation remains explicit.
- Present teacher-facing error text and a recovery action. Technical UUIDs, attempts and transport diagnostics belong in a disclosure. An observer connection is not proof that an execution device is available.
- New Template is the first/default tab, with editable TES and TPT examples. Examples never silently overwrite an edited draft or create saved records merely by opening the page.
- Android owns system-bar clearance natively where WebView safe-area reporting is insufficient. Browser layouts retain normal safe-area handling.

## Implementation tasks

- [x] Trace and explain shared catalogue metadata, server thumbnails and per-device original-file storage, including lifetime and cross-device availability.
- [x] Trace and explain installation identity, organisation ownership, nomination/leases, contact cadence and work discovery.
- [x] Repair revoked-device recovery, distinguish readiness from saved login, make current device prominent and move historical records out of the main list. Keep work discovery responsive without increasing heavy marketplace sync frequency.
- [x] Align Guides search/topic/tag controls and implement bounded searchable pages.
- [x] Bound import history, selection, review and spreadsheet surfaces; provide meaningful title/filter/order controls and preserve selection across page changes.
- [x] Repair missing TES thumbnail propagation from seller-side extraction through import commit to catalogue reads, including existing imported rows where stored cover evidence exists.
- [x] Put New Template first and provide usable editable TES/TPT examples.
- [x] Simplify Sync and Migrations run/activity layouts and bound their lists.
- [x] Correct Android top/bottom system-bar clearance and verify shared pagination at desktop and phone widths.
- [x] Resolve focused independent review findings, then run only affected tests, compilation checks and local UI smoke scenarios.

## Ownership and integration

Discovery readers map devices, storage/covers, imports, other UI and all seven screenshots independently. A slow design advisor supplies the shared layout decisions; source evidence decides implementation details.

Implementation owners work on file-disjoint slices. Main owns shared API-client integration, route registration/OpenAPI/type generation, the shared pager, Android shell insets and the plan/evidence record unless an exact shared-file mutation is explicitly assigned. Migration numbers are reserved only after inspecting the current sequence. Workers do not run builds, tests, linters or formatters while peers are editing.

## Validation limits

Target the uncertain boundaries: revoked devices cannot resume work; saved login cannot fabricate presence; pagination cannot lose selected resources or conceal older history; tenant scoping survives query changes; imported covers remain visible after import cleanup; examples cannot overwrite user drafts without an explicit action. Verify affected pages at desktop and 390px phone widths. No production marketplace requests or release claims.

Implementation and local verification are complete. The evidence below does not establish deployed behavior or physical-device layout.

## Source findings and implementation rulings

- `device.rs::load_or_create` preserves installation identity across updates and label changes. New app data creates another ID; a matching model name is not evidence that two IDs are interchangeable.
- The previous native hourly cycle coupled heartbeat, pending imports and scheduled work. The coordinator now separates cheap discovery from the hourly sweep: 10 seconds idle, 30 seconds when held, failed-discovery backoff capped at 60 seconds, and routine check-in every five minutes. Fast deadlines use a monotonic clock. Android discovery follows the actual activity lifecycle, not a timed grace window after resume.
- Keep explicit restore as the sole way to clear revocation. `DeviceRepo::restore` must not manufacture a fresh contact timestamp. A successful subsequent native check-in establishes contact.
- A spreadsheet report is already bounded at 500 rows and drives whole-report counts and filename matching. Keep that bounded working snapshot, but paginate displayed rows. Server-side paging of marketplace items and history remains necessary because those lists have no equivalent report bound.
- Before this repair, `tam_pipeline::render::cover` emitted a flat card for document/archive payloads. It now extracts genuine embedded previews, including one nested document level, under one cumulative inflation budget. Generated cards remain the honest fallback when no preview exists; stored digests alone cannot supply missing image content. Re-import can repair missing/generated catalogue covers without duplicating products or replacing genuine/manual covers.
- Blob storage is tenant-scoped and content-addressed, but currently has no garbage collection. Imported originals are read on a seller device and refetched there for later publishing; they are not replicated to every signed-in machine.

## Verification record

- `svelte-kit sync && svelte-check --fail-on-warnings`: 0 errors and 0 warnings after the final UI changes.
- Targeted Vitest: 17 files, 456 tests passed. Slices cover templates, guides, import models, Sync/Migrations, request pagination, API-client behavior and device/connection recovery. Incidental source-text, wording and field-copy assertions were removed rather than re-pinned.
- Targeted native/media/API unit checks: 119 passed, 456 skipped. The filter covered desktop scheduler, heartbeat, commands, control plane and revoked import reporting; pipeline archive/render behavior; and activity cursor/link behavior.
- PostgreSQL-backed storage checks: 35 passed across `guides`, `device`, `product_cover_repair`, `sync_activity` and `sync_requests`.
- PostgreSQL-backed import API checks: 47 passed across `import_runs_flow`, `imports_flow` and `import_cover_repair`. These exercise tenant isolation, paging/selection boundaries, repeat-import cover repair and manual-cover preservation.
- `cargo sqlx prepare` refreshed storage query metadata against local PostgreSQL. The local server rebuilt successfully.
- Android-only `:app:compileArm64DebugKotlin --offline --no-daemon` and `cargo check --locked -p tam-desktop --lib --target aarch64-linux-android` passed. The target-specific check exposed the missing Tokio `macros` feature, which was enabled. No APK was installed; these checks do not verify physical system-bar layout.
- Actual Chromium checks used the local API at desktop and 390px phone widths. Guides advanced from 1–25 to 26–28, then searching reset to the matching page. An import-history page-two failure retained page one and retried the original page-two request. A 154-resource import displayed 25 rows and advanced to 26–50. Its image endpoint loaded successfully.
- Templates preserved draft and pending category state across tabs, confirmed Undo after divergent edits, reset the category at explicit form boundaries, and updated the same saved template ID rather than creating another record.
- Sync history replaced pages and its activity tab displayed distinct recurring schedule ticks without key collisions. Run identifiers, attempts and failure references are disclosed inside run/resource details rather than used as normal labels; expanding a resource loaded its timeline.
- Migrations preserved 25 selections across pages. Selecting page two's eight resources raised the count to 33; clearing that page restored 25 off-page selections. Desktop and phone layouts showed no horizontal overflow in the exercised pages.
- Cleanup removed 27 guide fixtures, 26 resource fixtures, one template, and seven closed import fixtures with their job ledgers. API reads returned the pre-smoke counts: one guide, seven resources, one template and six import runs. No fixture-named content remained visible.
- The repository's Rust formatter ran on the 36 changed Rust files only. No full suite, deep marketplace end-to-end run, production marketplace request, deployment or release was performed.
