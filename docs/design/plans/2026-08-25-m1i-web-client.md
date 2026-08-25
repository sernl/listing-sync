# M1i: the web client

Goal: the thin client that renders progress and edits nothing behind the seller's back — the product-by-inventory table, per-inventory outcome bars with raw counts, the per-item step timeline with a downloadable result, the connections page with revoke and the blocked-on-seller state, the reconciliation queue, and the per-marketplace status page — in Svelte and SvelteKit per the founder's decision recorded in `decisions.md`, consuming exactly the API M1h serves.
The client performs no automation and holds no marketplace session; it is a progress reporter and catalogue viewer, which is what keeps it thin.

## Decisions this plan makes

- SvelteKit runs as a static single-page app: `adapter-static` with the SPA fallback, `ssr = false`, because the deployment is one axum process serving one static directory via `ServeDir` from a runtime path — `client-stack.md`'s crane-invalidation argument — and a Node server process would be a second runtime the architecture has no place for.
  `tam-server` gains `--ui-dir <path>` (tower-http's `ServeDir` as the router fallback; tower-http is already in the dependency tree), so the dev flow is Vite's dev server proxying `/v1` and `/healthz` to tam-server, and the deploy flow is the built directory behind the same origin — the cookie and the `EventSource` never cross origins in either flow.
- The dependency set is deliberately minimal: svelte, @sveltejs/kit, adapter-static, vite, typescript, svelte-check, vitest, tailwindcss.
  The windowed product table is hand-rolled (fixed row height, ~fifty lines), forms are plain bindings, toasts are a small store, and server state is a hand-rolled snapshot-plus-delta store: `load` fetches the snapshot, one `EventSource` per tab merges deltas keyed on `org_seq`, and a resync event refetches — the design's model, without the React-shaped cache dependency.
- The session cookie is HttpOnly, so the client cannot set it from JavaScript; the API gains the exchange the mint tool's output feeds: `POST /v1/session` takes the token, verifies it, and answers with the `Set-Cookie` (HttpOnly, SameSite=Lax, Secure, Path=/), and `DELETE /v1/session` expires the session and clears the cookie.
  The login page is a paste-the-token form until M5's real login; localhost is a secure context in the browsers that matter, so `Secure` is unconditional.
- Two more API enablers, both read shapes the client cannot do without: `GET /v1/mappings` (the flat org-scoped mapping listing that turns the products page into the product-by-inventory table) and `GET /v1/status` (public, no session: the per-inventory halt state from the fleet kill switch — a status page exists precisely for when logging in is what is broken).
  Connection delete stays unbuilt: the API has revoke only, deletion semantics for a connection row are undesigned, and shipping a destructive control without its design would be the exact corner M0's deletion probe warns about; recorded as a deferral rather than silently dropped.
- The domain vocabulary is generated, not retyped: `tam-typegen` (a small binary beside the OpenAPI constant, zero new dependencies) emits `web/src/lib/generated/vocab.ts` — `FailureCode`, `APIErrorCode`, `APIErrorKind`, `InventoryId`, `Marketplace`, the item state and outcome unions — from the same closed Rust enums the cross-layer tests pin.
  The web check lane regenerates and fails on drift, which is `client-stack.md`'s freshness condition discharged in the first lane it can be; the nix-sandboxed variant lands with the client's `importNpmLock` derivation, deferred below.
- A start-sync action ships on the products table: select mappings, one button, a client-minted idempotency key, the 201/200/409 statuses surfaced honestly.
  M1j still owns the pumps; starting a job that queues items is exactly what the operation-resource API is for, and the client exercising it is the support-cost containment this milestone is gated on.
- The devshell gains nodejs; the web lane is `just web-check` (install from the lockfile, typegen freshness, svelte-check, vitest, production build) beside the Rust gates rather than inside them, so a UI iteration never pays a cargo rebuild and vice versa.

## Tasks

### Task 1: API enablers

`tam-api`: `POST /{v}/session` (token exchange, Set-Cookie, structured 401 on a bad token), `DELETE /{v}/session` (expire plus clearing Set-Cookie), `GET /{v}/mappings` (org-scoped flat listing: mapping, product, inventory, binding state, lifecycle), `GET /{v}/status` (public: per-inventory halt rows from `inventory_halt`).
Storage: a mappings listing read and an inventory-halt read.
`tam-server`: `--ui-dir` serving the built client as the fallback service.
The OpenAPI table grows four routes and the parity test holds.
Tests: pg-lane session exchange round trip (exchange, whoami with the set cookie, logout, whoami refused), mappings listing under two tenants, status without a session.

### Task 2: the vocabulary generator

`tam-typegen`: emits `web/src/lib/generated/vocab.ts` with a generated-file header; `just web-typegen` regenerates; `just web-check` fails if regeneration changes the committed file.
Unit test pinning one union against the Rust constant so the generator itself cannot drift silently.

### Task 3: the SvelteKit scaffold

`web/`: SvelteKit static SPA, Tailwind, the typed API client (`fetch` wrapper parsing the structured `APIError`, cursor pagination helpers, credential-carrying defaults), the session store and login page (token paste, exchange, whoami echo, logout), the layout shell with navigation, the ledger store (snapshot plus `EventSource` deltas, `Last-Event-ID` resume, resync refetch).
Vitest covers the API client's error parsing, the cursor helpers, and the ledger store's merge and resync logic with a scripted fake `EventSource`.
Devshell nodejs; `web-install`, `web-dev`, `web-check` recipes.

### Task 4: the surfaces

Products: the windowed product-by-inventory table (products joined to mappings client-side), per-mapping state cells, selection, the start-sync action with its idempotency key and honest 409 copy.
Jobs: the jobs list; the job page with the outcome bar rendered from raw counts (every segment the ledger can store, no scalar verdict), live via the ledger store; the item timeline from the event rows; the downloadable per-item result (a client-side JSON blob of the item detail).
Connections: the list with link states, revoke behind a confirmation, the blocked-on-seller state (`needs_reauth`) surfaced as the page's loudest element.
Reconciliation: the open queue, resolve with a segments form, no-counterpart with a confirmation, the drain counters.
Status: the public per-marketplace page.
Component-level tests where logic lives (outcome-bar maths, timeline ordering, windowing bounds); page smoke via vitest with mocked fetch.

### Task 5: close

`just web-check` green beside all four Rust gates; the sandboxed flake check on the committed tree; a live smoke — the built client served by `tam-server --ui-dir`, the session exchanged from the real mint output, the products table against the seeded dev database; the phase table.

## Deferred, with owners

- The client's nix derivation (`importNpmLock`) and the nix-sandboxed typegen freshness check: with the deploy work, alongside the systemd units.
- Connection delete: needs its API and its deletion-semantics design first; revoke ships now.
- Virtualisation beyond fixed-row windowing, optimistic mutations, and offline behaviour: when catalogue scale demands them.
- The PWA manifest, service worker and Web Push: the mobile milestone `client-stack.md` scopes; nothing here forecloses it.
- Playwright end-to-end: M1j's wiring milestone, where the whole flow exists to drive.
- Real login, signup and password flows: M5, unchanged.
