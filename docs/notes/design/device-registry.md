# The device registry and "Your devices"

Decision D14's server-side half: which of the seller's own machines exist, what each one holds, and how the seller signs one out.

- date: 2026-09-03
- status: built and green under `just db-test`, `just web-check` and the desktop crate's own tests; the desktop client is wired end to end, reading the console's own session from its window and checking in on a timer
- decisions it implements: D14 (per-surface marketplace login, the device registry, the "Your devices" page with per-device sign-out), D1 (the two-branch automation rule the registry's contents obey), D10 and D11 (the entitlement the check-in closes on revocation), D30 (the wording the page uses about where a login lives)

## Why a registry of our own

better-auth already lists sessions, and it is not enough.
Its session row carries `ipAddress` and `userAgent` and nothing else, it has no device-name concept, and its `multiSession` plugin is several accounts in one browser rather than a registry of machines; all three were verified against better-auth 1.7.2 on 2026-09-02 and are recorded in the "Verified since the first draft" section of `vendoo-for-teachers-rethink.md`.
So a page that says "founder-pc, Windows, holding a TeachersPayTeachers login" needs columns that exist nowhere until they exist here.

The two planes stay separate, which is the charter's boundary rather than a convention.
`tam_auth` is confined to the `auth` schema; nothing in `public` reads it, and `tam-api` never does.
The page joins the two halves in the browser instead, and the join is a heuristic that says so.

## The tables

Migration 0042 adds two tenant tables, both under enabled and forced row-level security with a policy keyed on `app.current_org`, and both listed in the RLS matrix.

`device` holds one row per machine: `org_id`, `id`, `name`, `os`, `arch`, `app_version`, `first_seen_at`, `last_seen_at`, `revoked_at`.
The identifier is `text` rather than `uuid` because the device mints it itself — D14 records that `machine-uid` covers neither Android nor iOS — and it is bounded to 64 characters so a client cannot register an arbitrarily long key.
It is scoped by organisation, so a collision is only ever between one seller's own machines.
`name`, `os`, `arch` and `app_version` are bounded too, and the API refuses an overrun as a validation error rather than letting a constraint violation surface as a fault.

`device_marketplace_session` holds one row per marketplace a device reports holding: `org_id`, `device_id`, `marketplace`, `account_label`, `linked_at`, `last_used_at`, `status`, with the device as a cascading foreign key.

Both tables are metadata only.
Neither has a column a cookie, a token or any other credential could travel in, and that is D1 made structural: every no-API marketplace session lives on the seller's device, so a server-side column able to hold one would reintroduce the custody the whole architecture removes.
`account_label` is the storefront name the marketplace already shows the seller, not something that authenticates anything.

`status` is closed, and every value is one the device can distinguish without making a marketplace request.
`connected` is a session the device captured and has not discarded, `signed_out` is one the seller disconnected on the device itself, and `wiped` is one the device discarded because a heartbeat told it that it had been revoked.
There is deliberately no `expired`: nothing on the device can tell a live session from a dead one without contacting the marketplace, so a status meaning that would be a claim the data does not support.

No role but `tam_app` is granted anything on either table.
The engine has no reason to read the registry, the broker holds no session for a no-API marketplace to begin with, and `tam_auth` must hold no privilege on any table in `public`.

## The endpoints

Four operations under `/v1/devices`, every one org-scoped through `OrgContext`, so the request carries no organisation identifier a caller could substitute and a device id names a row only within the tenant the session speaks for.

`POST /v1/devices` registers a device or refreshes what a known one says about itself.
It is an upsert that never clears `revoked_at`: a revoked device re-registering is the exact case the mark exists for, and a registration that lifted it would let any device undo its own revocation by restarting.

`POST /v1/devices/{device}/heartbeat` is the device's check-in.
It stamps the device seen, replaces what the device holds, and answers whether the device has been signed out.
The report is the device's whole session set rather than a delta, so a marketplace absent from it is deleted; a delta would leave a stale row standing after a disconnect the device performed while offline, and the page's whole job is saying what a machine holds now.
`last_used_at` advances only for a session reported as `connected`, because a session the device no longer holds was not used at the heartbeat instant.
An unregistered id is not-found: a heartbeat is not a registration, so a client that lost its registration re-registers rather than silently appearing in the seller's list under whatever id it now holds.
A report naming a marketplace with an official API is refused outright, because its automation runs server-side under a sanctioned token and no device holds a session for it.

`GET /v1/devices` lists every device this organisation has registered, revoked ones included: a device the seller signed out is part of the record, and hiding it would hide the one row whose wipe is still outstanding.

`POST /v1/devices/{device}/revoke` signs one machine out.
Revoking twice keeps the first instant rather than restamping, because the first is when the seller decided and the wipe is measured against it.

The view adds one derived field, `wipe_outstanding`: the device was signed out and has not been heard from since, so it may still hold the marketplace logins listed beside it.
It is derived rather than stored because it is a statement about what we know, not a fact anybody wrote down, and it is computed in Rust rather than in SQL so it can be tested without a row.

## Wipe on next contact, and its honest limit

Revocation is a decision, not an effect.
Marking a device revoked stops the server answering it and closes its entitlement gate, and the device forgets its own marketplace sessions when it next reaches the heartbeat endpoint.

A device that never reconnects keeps its marketplace cookies until the marketplace itself expires them.
Nothing can do better: the sessions are on the seller's machine by design and the server has never held them, so there is no request we could make that would remove one.
The page states this rather than implying an immediate wipe, both in the sign-out confirmation and beside any row whose wipe is still outstanding.
That is D14's own framing — two facts recorded rather than engineered away — and this is the second of them.

## The page, and how the join works

`/settings/devices` renders two panels.

The first is the machines: each device with its operating system, architecture, application version, when it was last seen, and every marketplace it holds with the account label and the status.
A sign-out control on each row performs both acts, because the two planes are separate and neither implies the other: it calls our revoke endpoint, and it ends the browser sign-in matched to that machine through better-auth's `revokeSession`.
Both are started before either is awaited, so one failing cannot skip the other, and the result says which parts actually happened.

The second panel is the browser sign-ins the join could not attribute to a machine, each with its own control to end it.
The sign-in this browser is using is marked in whichever panel it lands in, so the seller can tell which control signs them out of the page they are reading.

The join itself is a heuristic and the page says so on every row.
There is no shared key between the two planes: better-auth stores `ipAddress` and `userAgent`, and the registry stores a name and an operating system.
So the page reads a platform token off the user agent — `windows`, `macos`, `linux`, `android`, `ios`, which is the same vocabulary `std::env::consts::OS` gives the device — and attributes a sign-in to a machine only when exactly one device and exactly one sign-in claim that platform.
Two machines running the same operating system, or two browsers on one machine, leave every row on that platform `ambiguous`: no sign-in is attached, the rows say we cannot tell which belongs to which, and each unattributed sign-in gets its own row to be ended from.
A platform no device reports leaves the row `unmatched`.
The order of the user-agent tests is itself load-bearing, because an Android agent contains "Linux" and an iPad's contains "Mac OS X"; the tests cover both.

`revokeSession` takes the session's `token` rather than its `id`, and both are on the row, so the client type carries the one that works.

## The cookie cache

`auth/src/auth.ts` now sets `session: { cookieCache: { enabled: false } }`, explicitly rather than by default.

With the cache on, `getSession` answers out of a signed cookie for `maxAge` seconds — 300 by default — without reading the session table, so a session ended through `revokeSession` stays usable until that cache expires.
This is the revocation latency D14 says the page must state rather than hide, and switching it off removes it instead: a sign-out takes effect on the next request that browser makes.
Turning the cache on is now a founder decision that has to answer for the latency it reintroduces, which is what the comment beside the setting records.

## The device's half

`apps/desktop/src-tauri/src/heartbeat.rs` is the client side.

`report_of` walks the session store over the seller-device marketplaces and returns what the machine holds, as metadata: the marketplace, the account label the marketplace already showed the seller, and the status.
Nothing in the report type can carry a cookie, for the same reason `SessionStatus` cannot, and a test asserts that neither a cookie name nor a value appears in the report's `Debug` output.

Amended 2026-09-04: the check-in also carries decision D10's entitlement.
`POST /v1/devices/{device}/heartbeat` answers an optional `entitlement` field holding a signed token, minted only when the deployment has a signing key, the device is not revoked, and `DeviceRepo::entitled_marketplaces` returns a non-empty set — the work claim's own predicate asked as a question rather than embedded in a claim.
`check_in` verifies it against the key compiled into the binary and installs the gate; a token that does not verify, and an answer carrying none, both close it, which is what keeps a lapsed plan to one revalidation window rather than one grace window.
A check-in that could not reach the server leaves the gate alone, for the same reason it wipes nothing: an offline period is not a lapse.
The field is optional and skipped when absent, and neither `HeartbeatView` nor the client's own reply type denies unknown fields, so the published 0.1.3 client sees bytes identical to what it saw before.

`check_in` reports, acts on the answer, and records the state.
A revoked answer forgets every marketplace session through the existing `SessionStore` and closes the entitlement gate before it returns, so a caller that ignores the return value has still had the sessions removed.
It forgets every marketplace rather than only the seller-device ones, because this is the removal and skipping a key on the strength of what we believe about its transport class would leave a stored session behind on the one path whose job is leaving none.
A check-in that could not reach the server is not evidence of revocation: it wipes nothing, which is what stops every offline period becoming a disconnect.

`cycle` is the scheduled path: check in, then pull work.
The check-in comes first because it is what learns of a revocation, and a cycle that pulled first would spend a round of work under sessions it was about to forget.
`first_run` registers and then checks in, and is reached from the `device_check_in` command the console calls when it loads.
`connect_marketplace` checks in after a capture and refuses to report success if the answer says this device has been signed out.

`DesktopState` gained a `revoked` flag beside the gate, kept apart from it because the two answer different questions: the gate says whether work may run, and this says why it may not, which is what the interface shows.
`device_check_in` answers `reached_server` alongside `revoked`, so the interface never reads a `false` from a check-in that never happened as permission.

`ControlPlane` is the trait, exactly as `WorkSource` is for work, and it has two implementations.
`Offline` returns `NotConfigured` for every call and is what a build with no configured base URL gets; an `Offline` that answered success instead of an error would be the one failure the registry exists to prevent, so it answers an error.
`HttpControlPlane` in `control_plane.rs` is the wire implementation, added on 2026-09-03 with the founder-disclosed `reqwest` edge.

That module is split in two so the protocol is testable without a socket.
`Transport` is one POST reduced to a path, a body and a reply; `HttpControlPlane` is the protocol above it, deciding which path, what JSON, and what each status means.
The unit tests substitute a fake at that seam, so the two paths, the body keys and the status mapping are pinned without binding a port, and `crates/tam-api/tests/devices_flow.rs` pins the same paths and keys on the server side against a real database.
`404` on either path means only that this device is not registered, so the client re-registers rather than retrying; every other non-success is a refusal carrying the status and an excerpt of what the server said.
A `200` whose body does not parse is a refusal too, and deliberately not a default: the only default available would be `revoked: false`, which is the answer that keeps working, so guessing it would turn a broken proxy into a device that never learns it was signed out.

Two custody properties are enforced there rather than remembered.
The session token this client speaks under never reaches a formatter, because `HttpTransport` writes its own `Debug`, the same reason `CookieJar` does.
And a test drives a real captured session with a real cookie through `check_in` and asserts that neither the cookie name nor its value appears in the bytes that would go out, which is the property the whole architecture rests on asserted at the one place bytes leave the machine for us.

The client is now wired end to end.
`lib.rs` constructs `HttpControlPlane::against(base_url(), WebviewSession)` during setup, so a shipped build reaches the real registry.

The base URL is `control_plane::base_url()`: the compiled-in `https://api.teachouse.io`, which is the origin `tauri.conf.json`'s content-security policy already names, overridable by `TAM_CONTROL_PLANE` for development.
In development that override points at the vite origin, where `just web-dev` serves the console and proxies `/v1` to a local `tam-server`, so the session cookie and the control plane are the same origin there.

The session is resolved per request rather than captured, and that is not a refinement.
The cookie is HttpOnly, so the console's own JavaScript can neither read it nor hand it over; `WebviewSession` reads it from the main window's cookie store from Rust, the same route `connect_marketplace` uses for a marketplace login and for the same reason.
Per request rather than once, because the application starts before the seller signs in and a session captured at construction would be missing forever, and because the device stops speaking the moment the seller signs out without anything having to notice and clear a field.
Nothing retains it: `HttpTransport` has no session field at all, so there is no place a credential could be read out of and none to invalidate.

Not being signed in is its own state, distinct from both revocation and an unreachable server.
`SessionSource` answering `None` produces `ControlPlaneError::NoSession` before the transport is reached, so a machine sitting at a sign-in screen makes no request at all rather than one the server would answer with the same blank 401 it gives a forged cookie — an answer this client could not tell from a revocation.
It wipes nothing and revokes nothing; it records only that nobody is signed in, which `DesktopState::signed_in` holds and the `device_check_in` command reports beside `revoked` and `reached_server`.
A cookie store that cannot be read is a refusal rather than an absence, because a fault is not a sign-out.

The schedule runs.
`run_schedule` ticks at the scheduler's cadence for the life of the process and calls `heartbeat::cycle`, which checks in and then pulls work.
The check-in is the half that matters today: it is how a device the seller signed out learns to wipe between console loads, and the work pull reaches `NoWork` until the engine driver split lands a real source.
The first tick of an interval completes immediately, so there is a check-in at start-up too; it races the window's creation and loses harmlessly, because no window means no session, which is exactly `NoSession`.

## What is verified

`crates/tam-storage/tests/device.rs` drives the repository against a real database: the register-and-refresh upsert, the heartbeat replacing rather than merging what a device holds, revocation standing across a re-registration and reaching the next heartbeat, and a two-tenant case where one seller can neither list, heartbeat as, nor sign out another seller's machine.
`crates/tam-storage/tests/rls_matrix.rs` carries both tables, so neither could have been added without a tenancy decision.
`crates/tam-api/tests/devices_flow.rs` drives the four endpoints over the wire, including the two-tenant case at the surface, the refusals, and the unauthenticated case.
`web/src/routes/settings/devices/merge.test.ts` covers the join, including the two agent strings that would fool a naive platform reader and every case where the join declines to guess.
The desktop crate's own tests cover the report, the wipe, the cycle ordering, the no-transport case, and the wire client: the paths and body keys it sends, the status mapping, the revoked answer reaching the session store, an unparseable answer refusing rather than guessing, and that no captured cookie reaches the bytes it would send.

## Sources

`docs/notes/design/vendoo-for-teachers-rethink.md`, decisions D1, D10, D11, D14 and D30, and the "Verified since the first draft" section.
`docs/notes/design/desktop-client.md`, for the device identity, the session record and the command surface this extends.
`docs/notes/design/engine-driver-split.md`, sections 4 and 5, for the pull model that binds a lease to a device id and the `Device` actor component that will name it.
better-auth 1.7.2, `packages/better-auth/src/cookies/index.ts` and `packages/better-auth/src/api/routes/session.ts`, for the cookie cache and the session endpoints.
