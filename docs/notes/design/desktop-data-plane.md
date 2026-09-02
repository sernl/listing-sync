# The desktop client as the data plane

What runs on the seller's own machine once the desktop client stops being a shell and starts being the process that composes marketplace requests.

- date: 2026-09-03
- status: built and green under `cargo clippy -p tam-desktop --all-targets --all-features -- --deny warnings` and `cargo nextest run -p tam-desktop` (107 tests), and cross-compiling to `x86_64-pc-windows-msvc`; two endpoints it depends on are specified here and not yet served
- decisions it implements: D1 (two-branch automation, and the interface rule that the seller can see which device did the work), D10 and D11 (the entitlement the tick is gated on, and its grace), D14 (the device registry this device checks in with), D27 (client-side file ingest, and the interim that precedes it)
- what it continues: `engine-driver-split.md` sections 3 to 5, `desktop-client.md`, `device-registry.md`

## The custody line, stated once

The line D1 draws is not the transport seam.
It is which process composes and issues a request to a marketplace with no official API.
After this change that process is the desktop client on the seller's machine, and no server binary composes or issues one.

Concretely, the following now happen inside `tam-desktop` and nowhere else for TeachersPayTeachers and Tes.
The marketplace adapter renders a projected listing into that marketplace's own field set.
The adapter composes every URL, header, form field and encoding.
The HTTP client that issues those requests is constructed from the seller's own cookies, which never leave the machine that captured them.
The interpreter decides which request to make next, and holds the budget it makes them under.

The following remain on the server, and each is a refusal rather than an omission.
The server decides what an item is: `prepare_item` writes the seller's decision surface and answers a preparation.
The server owns the lease, its epoch and its expiry, all computed in SQL against its own clock.
The server owns the ledger: every write the device asks for is keyed on a lease reference, and the server derives the organisation, the connection, the inventory and the epoch from the lease it issued rather than trusting an argument.
The server owns the rate ceiling, the halt scopes wider than one tenant's inventory, the seller's outbox, and the per-organisation event sequence.

The server never says now.
It answers what is due when asked, and the timer that asks runs on the seller's device.

## The sequence of one pull

The local timer fires on its deterministic cadence.

The device checks in first, because a check-in is what learns of a revocation, and a cycle that pulled first would spend a round of work under sessions it was about to forget.

For each marketplace the device works, the tick decides whether to ask at all, before any request is composed.
Four refusals are checked in this order: the seller signed this device out; nobody is signed in to the console on this device; the entitlement gate refuses this marketplace; nobody has signed in to this marketplace on this device.
Each is reported as itself rather than as a generic failure to sync, because the seller acts on each of them differently.

If none of the four holds, the device asks the control plane for work.
The answer is one of three: an envelope carrying an item, idle, or held by another of the seller's devices.
Idle and held both end the marketplace's turn; held carries the jittered delay the device backs off for, so the losing device stops polling a queue it cannot win.

An envelope carries the leased item, the server's preparation of it, a manifest per file the operation uploads, the server's own reading of now, the server's deadline, and the next-poll delay.
It is `tam_engine_driver::vocabulary::ClaimView`, defined once and read by both sides, so the envelope on the wire and the envelope in the interpreter cannot drift apart.
It carries no URL, no header, no form field name and no encoding.
That is the property that keeps the architecture at S1: if a field here could not be computed without composing a marketplace request, the boundary has moved.

The device turns `server_deadline_ms - server_now_ms` into a local monotonic instant at the moment the envelope arrives.
It never compares the server's absolute instant against its own clock, because those are two clocks, and a laptop's is frequently wrong.

The device renders the preparation through its own adapter, taking the intent hash over what it rendered.
The server does not render the field set, and could not without composing.

The interpreter then runs the item against the marketplace, under the seller's session, through the device's own transport, with the device's own file source, its own attempt-id source, and the cancellation described above.

When the run is over the device settles: one envelope fenced on the item and the lease epoch, which the server refuses if a different device holds that lease.
The settle is not literally the last request of the run, and it is not meant to be: the interpreter notifies the seller once the item is terminal, so a notification and a journal append follow it, and nothing after the settle writes to the item.

Every step of that sequence is recorded as a typed event naming the device that produced it, and the console reads them back.
That is D1's interface rule rather than a convenience: with the work on the seller's own machines, "which device did this" becomes a question the seller can ask, and one they must be able to answer.

## The ledger, and what the device is not allowed to decide

Every ledger write the interpreter makes is a round trip to our control plane, and none of them is answered on the device.
That is not caution about correctness; it is the whole point of the port's shape.
`open_attempt` answering `AttemptInFlight` is the duplicate-create fence, and a device that answered it locally would have no fence.
`request_grant` is the rate ceiling, and a governed party that answered it would be setting its own limit; neither the window nor the ceiling appears in the call.
`settle_attempt` answers the binding decision, `preflight_failed` answers the streak that decides a give-up, and `connection_for` decides whether the run may proceed at all.
Every call carries the lease reference the server issued, so the server derives the organisation, the connection, the inventory and the epoch rather than trusting an argument.
The one halt reachable from here is fixed to the tenant's own inventory and takes no scope argument, so the wider halts remain the breaker's and the canary's.
The notification takes the closed seller-event value and never a topic string, so no relay name crosses the boundary.

Eleven of the twelve go to one endpoint that does not exist yet, and this is its contract.

`POST /v1/devices/{device}/ledger` under the console session cookie, taking a `LedgerCall` and answering a `LedgerAnswer`, both from the driver crate's vocabulary.
It fences every call on the lease the call names, refusing one whose holder is not this device, and it answers `Refused` carrying a `LedgerError` rather than an HTTP fault for the two conditions the interpreter branches on: `AttemptInFlight` and `StaleLease`.
A device that saw those as transport failures would retry the two things it must not retry.

The twelfth is the settle, which keeps its own existing path because that path is already fenced on the holder's device id as well as on the epoch.

Lease renewal is not on this surface yet.
The device drives its budgets off a duration the server vouches for, so renewal will move that deadline rather than change the shape of anything here.

## The session, and how it reaches the transport

The seller signs in to a marketplace in a webview this application owns, and the cookies are filed in the operating system's keychain.
Nothing sends them anywhere; the check-in reports that a session is held and never what is in it.

`src/marketplace.rs` is the one join between that store and the transport the adapters send through.
It reads the stored record per request rather than capturing it once, so a seller who signs in again is picked up at the next request instead of at the next restart.
It rebuilds the underlying client only when the stored record changed, because a client built per request would discard its connection pool and its TLS session cache.
It exposes an explicit invalidation for the other direction, where the record is unchanged and the cookies inside it have stopped working.

A session-authenticated request may reach exactly one host: the origin whose cookies it carries, taken from the same login target the login window captured them for.
A request to any other host is refused before a client carrying the jar is built at all.
A request that is not session-authenticated is exempt, because the direct-to-S3 upload is signed in its own body and must reach the bucket without our session attached; the adapter's own transport asserts that half.

The live clients are the adapter crates' own rather than reimplemented here.
That is deliberate: TPT's request shaping is measured rather than incidental, and the double-submit CSRF header, the per-hop `sec-fetch` envelope and the browser identity are the difference between the marketplace answering with the form and answering with a Cloudflare interstitial.
The cost is a second reqwest major in the bundle beside the one the check-in and the updater share; both resolve to one rustls over ring, so it adds no second TLS stack and no C toolchain to the Windows cross-compile.

## The interim payload fetch, and what it owes

D27 puts file ingest on the seller's device: if the upload is itself a marketplace request that must originate here, the bytes must be here at upload time, and the file never reaches our servers.
That is the target and it is not what holds today.
The catalogue's files were ingested through `POST /v1/uploads` and live in our object store, so for now the device fetches them back for the length of one run.

The fetch is owed an endpoint that does not exist yet, and this is its contract.

`GET /v1/devices/{device}/payload/{file}` under the console session cookie.
It answers 200 with `application/octet-stream` and the bytes verbatim.
It answers 403 when that device holds no live lease on an item whose projection references that file.
It answers 404 when the file does not belong to the organisation the session speaks for.

It deliberately carries no digest, no name and no content type.
Those come from the envelope's payload manifest, which the server committed to before the transfer began.
A response that restated its own digest would prove nothing; a digest the server committed to beforehand proves the transfer.

On the device the bytes are verified before anything is written: the length against the manifest first, so a truncated transfer of a large file is named as one rather than as an unexplained digest mismatch, and then the blake3 hash, which is what actually decides.
Unverified bytes are never written to the cache, or the next read would take them back out and skip the check.

The bytes live in one directory per item under the application data directory.
They are removed when the item settles.
They are also removed when the run's file source drops, so a run that ended by an error rather than by a settle leaves nothing either.
A sweep at start-up removes what a killed process could run neither of those for, because a crash means there was no settle and the file is still the seller's.

The interim ends when D27 lands: at that point the ingest pipeline runs on the device, the bytes are already there, and this endpoint and this cache are deleted rather than optimised.

## What is stubbed, and what is owed

Two endpoints are specified here and served nowhere.
`GET /v1/devices/{device}/payload/{file}` above, without which an item whose operation uploads a file cannot run on the device.
`POST /v1/devices/{device}/ledger`, without which no item can run at all.

A third gap is smaller and sharper.
`POST /v1/devices/{device}/work` takes no body, so it cannot be asked for one marketplace.
The device sends the marketplace as a filter the server currently ignores, and refuses an order whose inventory is not the marketplace it gated on, because the readiness gate is per marketplace and running past it would bypass the session and entitlement checks that gate performed.
That refusal costs a lease expiry, which is what makes serving the filter worth doing.

The entitlement verifier still carries an all-zero public key, which is not a valid Ed25519 point, so every gate answers no until the founder supplies the real one.
That is the correct failure for a placeholder and is recorded in `desktop-client.md`.

Lease renewal is not implemented.
The split note's section 4 makes claims renewable so a device that goes offline mid-flow is reclaimed for having stopped heartbeating rather than for being slow, and the driver's own budget inequality already sits close to the lease TTL.
The seam is left for it; the schedule and the deadline arithmetic already assume a fixed deadline, so adding renewal moves the deadline rather than changing the shape.

A revocation mid-run does not behave as `engine-driver-split.md` section 5 requires, and the divergence is in the machine rather than in this client.
That section says a failed entitlement check must produce the shape `BudgetGrant::Exhausted` produces — the open attempt settled abandoned and the run `Abandoned` — and never a new terminal outcome, so that a revocation never settles an item on evidence the run does not have.
What happens is that `SyncMachine::exhaust_budget` maps the pre-submit states to a terminal `Outcome::Skipped` (`crates/tam-domain/src/lib.rs:1198`), so the interpreter settles the item and the seller's queued work is dropped rather than left for the reaper to requeue on an entitled device.
The safety half holds: no attempt is opened and no marketplace request is composed, which `a_revocation_in_flight_stops_the_run_before_it_opens_an_attempt` asserts directly.
The liveness half does not, and the test pins the current behaviour with that stated rather than endorsed.

The activity ring is an interface convenience and not a journal.
It is bounded, it is stamped with the tick's instant rather than each event's own, and it is lost when the application exits.
The record of what happened to an item is the ledger on the server, and it is the one an operator reads.

## Sources

- `docs/notes/design/engine-driver-split.md`, sections 3 to 5.
- `docs/notes/design/desktop-client.md`.
- `docs/notes/design/device-registry.md`.
- `docs/notes/design/vendoo-for-teachers-rethink.md`, decisions D1, D10, D11, D14 and D27.
