# The M1j founder runbook: link, probe, first run, rehearsal, and the charge

M1j draws a boundary: code wires, proves and rehearses the duplication flow, and five acts stay the founder's.
This file is the procedure for those five — linking the real Tes session through the broker, probing the New Zealand currency rule, the supervised first run on the founder's own catalogue, the ZZ-prefix rehearsal against the live account, and sending the Stripe payment link.
The plan is [`plans/2026-08-25-m1j-duplication-and-first-charge.md`](plans/2026-08-25-m1j-duplication-and-first-charge.md); its boundary and decisions sections govern and nothing here reopens them.

Every procedure below drives a binary that already exists, and none of them changes code.
The fail-closed states stay closed: the New Zealand currency gate and the two uncaptured Tes endpoints — list-own-resources and own-file download — are opened by a founder code decision taken after the capture that measures them, not by a step in this file.
The currency act below records an observation; it does not flip the gate.

## Linking the Tes session through the broker

The broker is the only process holding the key-encryption key and the only database role that can read `connection_secret`, so linking is a write to its unix socket rather than an HTTP call.
No API surface for linking exists: `crates/tam-api/src/lib.rs` routes `GET /{version}/connections` and `POST /{version}/connections/{connection}/revoke` and nothing else against connections.
The credential is a Tes cookie header taken from the founder's own browser session.

### Preconditions

The database is up and migrated, and the broker is serving: `tam-session-broker serve <socket-path> <broker-db-url> <kek-path> [upstream-base]`, where the upstream defaults to `https://www.tes.com`.
The key-encryption-key file exists and is escrowed; the loader accepts either 32 raw bytes or 64 hex characters, and the tested escrow-and-recovery procedure is a Stage A item in [`compliance-floor.md`](compliance-floor.md).
An application login exists for the founder's organisation: `just dev-session` runs `tam-mint-session <db-url> <org-uuid-hex> <email> [ttl-days] [--ensure-org <name>]` and prints the `tam_session=` line the `/login` page asks for.
The exported cookie jar is a secret, lives at the gitignored `probes/local/tes-cookies.jar`, and is never printed or committed.

### Steps

1. Log into Tes in a browser as the account that will be automated and export the cookie jar in Netscape format to `probes/local/tes-cookies.jar`.
   That is the ingestion shape the code already proves: `TesSession::from_netscape_jar` at `crates/tam-marketplace-tes/src/session.rs:44`, used by the live smoke example at `crates/tam-marketplace-tes/examples/live_smoke.rs:51` and by the canary at `crates/tam-canary/src/main.rs:43`.
2. Verify the jar before anything durable holds it, with a mode that creates nothing: `cargo run -p tam-marketplace-tes --example live_smoke -- probes/local/tes-cookies.jar preflight`.
   It prints the draft form schema fingerprint on success.
3. Choose a connection UUID and send the link request as one line of JSON on the broker socket.
   The wire shape is defined in `crates/tam-session-broker/src/protocol.rs`: an object with `op` set to `link`, plus `org` and `connection` as canonical hyphenated UUIDs, `marketplace` as `"Tes"`, and `cookie_header`.
   The `cookie_header` field is the `name=value; name=value` header rather than the jar file; no binary converts between the two today, because the jar-to-header step lives inside `from_netscape_jar` and is not exposed, so the founder assembles the header from the jar by hand.
4. Confirm from the application side that the connection reads as linked, either through `GET /v1/connections` with the founder's session or on the client's connections page.

### Success

The broker replies `{"status":"linked"}`, the connection reads as linked from the API, and a subsequent `lease` request returns a loopback endpoint and an expiry and no secret material at all.
The link is idempotent on the connection row: it updates an existing connection to linked, and inserts one if none exists.

### Abort and rollback

Any reply other than `linked` carries a `detail` field; read it and stop rather than retrying with a fresh jar, because a jar that fails preflight will fail again.
Undo one connection with the broker's `revoke` request, which tombstones its ciphertext and revokes it.
The global case is the operator drill, `tam-session-broker revoke-all <broker-db-url> <kek-path>`, and timing that drill is itself a Stage A compliance-floor item.

## Probing the New Zealand currency rule

`InventoryId::TesNz` declares `CurrencyRule::Unmeasured` at `crates/tam-types/src/lib.rs:173`, from the enum declared at lines 136 to 148.
The projection consumes it at `crates/tam-taxonomy/src/listing.rs:97-108`: a `PriceIntent::Paid` product projected into an unmeasured inventory returns `ProjectionBlocked::CurrencyUnknown`.
A free price needs no currency, so free listings pass the gate today and priced ones block honestly.
The only measurement on record is US-only — fetching two US-inventory resources from a New Zealand client with `geoCurrency=AUD` cookies still returned USD offers, so currency follows the inventory rather than the viewer — and the GB-cookie case is untested.

### Preconditions

The link act is complete and preflight passes.
The founder is willing to create and delete one priced draft on their own account under the artefact invariants of the rehearsal act below.
The deferral stands until then: the plan's deferred list keeps this founder-supervised and dated before the first priced New Zealand listing, and free listings do not need it.

### Steps

1. On the account that will be automated, open the New Zealand upload flow for a new resource and record whether a currency control appears at all, what values it offers, and whether the choice is per-resource, per-account or fixed.
   Record it as a screenshot and a written note rather than from memory.
2. Set a price, save the resource as a draft, read the draft back, and record the currency the resource itself reports.
3. Record whether the same-account Curriculum-tag mechanism the wedge relies on changes any of the above, since that mechanism is what makes the answer non-obvious.
4. Delete the draft in the same sitting.
5. Write the observation into the decision record with its date and evidence.
   Replacing `Unmeasured` with `Fixed(_)` or `SellerScoped` is a founder code decision taken after that record exists.

### Success

An observation with evidence sufficient to name one `CurrencyRule` variant for TesNz without guessing, recorded and dated.
Until that exists the projection's refusal is correct behaviour rather than a defect.

### Abort and rollback

If the form's behaviour is ambiguous — the control appears but its scope cannot be settled from one resource — record the ambiguity and leave the gate closed.
The doc comment on the variant states the rule: it is removed by a probe, not a guess.
Deleting the priced draft is the whole rollback; nothing else in the system changed.

## The supervised first run

Three binaries carry this act, and their argument contracts are fixed in their own headers.
The broker serves as above.
`tam-import <db-url> <org-hex> <gateway-base> <kek-path> <store-root> <manifest.json>` turns operator-supplied resource ids and local file paths into canonical products, with the manifest a JSON array of `{ "resource": <numeric id>, "files": ["/path/to/original", ...] }`.
`tam-worker <engine-database-url> <worker-name> <broker-socket> <kek-path> <store-root> [poll-ms]` runs the item pump, polling every five seconds by default.
Both reach the marketplace through a broker lease, so neither process holds a credential: request a lease first and pass the endpoint the broker prints as the import's `<gateway-base>`.

### Preconditions

The link act is complete, the database is migrated, and the object-store root is writable.
Resource ids are supplied by the operator because the list-own-resources endpoint is uncaptured, and file bytes come from the founder's own disk because the own-file download endpoint is uncaptured; both stay that way until a founder-supervised capture lands.
The import's source and target inventories are fixed in `crates/tam-import/src/main.rs` as `TesGb` and `TesNz`.

### Steps

1. Assemble the manifest from the founder's own catalogue, one row per resource with its numeric Tes id and the local paths to its original files.
   Start with a handful of resources rather than the whole catalogue.
2. Request a lease from the broker for the founder's org and connection and note the endpoint and expiry it returns.
3. Run `tam-import` against that endpoint.
   It prints one line per row — terms mapped over terms seen, new reconciliation items, the already-open dedup count, and either `projectable` or `blocked: <gate>` — then the total, `imported N row(s); drain: M new item(s) over T term uses`.
4. Read the drain report before running anything else.
   The first-run share of terms reaching the reconciliation queue is the kill-gate measurement the plan compares across the first ten migrations.
5. Work the reconciliation queue in the client at `/queue` until the rows intended for this run are projectable.
   A blocked row is honest output rather than something to route around.
6. Enqueue the job, start `tam-worker`, and watch it.
   A projection-blocked item parks for a day and un-parks into a clean retry on the first steal pass after expiry.
7. Review the settled ledger per item in the client at `/jobs`, then the job view and the per-item detail.
   The outcome vocabulary is `Succeeded`, `Degraded`, `Failed`, `Ambiguous`, `Skipped` and `Blocked`; an ambiguous item halts the tenant mutex by design and is the founder's to adjudicate.
8. Repeat from step one for the rest of the catalogue only after the first pass has been reviewed item by item.

Both passes run the same publish-gated code path, and the gate is closed in a stronger sense than the plan's prose implies.
Every mapping the import writes carries `PublishMode::DryRun` at `crates/tam-import/src/lib.rs:302`, no repository method updates a mapping's publish mode after insert, and the driver never calls the adapter's publish — the only publish call sites in the workspace are the adapter's own tests and the live smoke example.
So the worker creates the draft, uploads the files, sets the metadata, verifies by read-back and settles, and making a resource public stays the founder's act in the Tes dashboard after reviewing the verified draft.
Opening the worker's publish gate is a founder code decision on the same footing as the currency gate.

### Success

Every item in the run reaches a settled outcome with a per-item record a seller could check.
The drain report's new-item count is recorded against the kill gate.
The drafts the worker created are present on the Tes account and match intent on read-back.

### Abort and rollback

Stop the worker with ctrl-c; it shuts down through its cancellation token, in-flight leases expire, and the stealer requeues with the epoch bumped, which is the stall bias the driver was built with.
If the account or the marketplace looks wrong, revoke the connection through the broker rather than merely stopping the worker, and use `revoke-all` for the global case.
A degraded item is terminal and is not a retry class: the listing is live and provably lossier than intended, and resending makes it worse.

## The ZZ-prefix rehearsal

The vehicle is `crates/tam-marketplace-tes/examples/live_smoke.rs`, the operator's supervised live check that replaced the deleted spike.
It runs as `cargo run -p tam-marketplace-tes --example live_smoke -- <jar-path> <mode> [args]` with three modes: `preflight`, the default, which creates nothing and asserts the draft form schema; `draft <pdf-path>`, the full draft flow followed by a verified delete; and `publish <pdf-path> --yes-publish-live`, which goes live.
Without the flag the publish mode refuses and keeps the draft, at line 107.
Every artefact carries the title literal `ZZ-SMOKE-DELETE-ME`, at line 32.

The M0 spike's account-safety invariants live only in that plan's prose, at [`plans/2026-08-25-m0-tes-spike.md`](plans/2026-08-25-m0-tes-spike.md) lines 21 and 22, and they bind this act unchanged in substance: at most three ZZ artefacts exist at once, each is deleted in the same run that created it, nothing is published except through the explicit gated mode, and the founder verifies the deletion in the dashboard rather than trusting an exit status.

### Preconditions

A cookie jar that passes preflight, a small throwaway PDF, and the founder present at the keyboard for the whole run.
Nothing else writes to the account concurrently: the per-tenant mutex stops two workers racing session-bound form tokens and does not know about a human in the dashboard.

### Steps

1. Run `preflight` first, every time.
   It creates nothing and fails closed if the form schema has drifted.
2. Run `draft <pdf>` for the rehearsal proper.
   It creates the draft, uploads the file, deletes the draft and verifies it is gone, printing `draft <id> deleted and verified gone`.
3. Open the Tes dashboard and confirm no `ZZ-` titled resource remains.
4. Use `publish <pdf> --yes-publish-live` only when the intent is to prove the publish leg itself, and delete the published resource immediately after verifying it.

### Success

Preflight prints a schema fingerprint, the draft run prints both the creation and the verified deletion, the dashboard shows no ZZ artefact, and the count of live ZZ artefacts is back to zero in the same sitting.

### Abort and rollback

If a run dies between create and delete, delete the artefact by hand in the dashboard before starting another run; the three-at-once ceiling is the entire safety margin.
If the schema fingerprint changed, stop.
That is the drift the canary escalates as a fleet halt for the inventory, and rehearsing over drift corrupts listings.

## The Stripe payment link

Charging is manual by decision for M1: the plan records the payment-link flow and what is charged for, and automated billing is M5's, needed at roughly fifty customers.
No billing code and no billing tables exist in this repository, and that absence is the decision rather than an omission.

### Preconditions

The written pre-approval reply from Stripe under Services Agreement 1.2(a)(ix) has landed.
This is a hard gate, not a formality.
[`compliance-floor.md`](compliance-floor.md) lists the enquiry as a Stage A item owned by the founder at line 30 and gives the reasoning at lines 96 and 97: Stripe's restricted-businesses list contains no anti-automation, anti-scraping, terms-circumvention or credential clause, the only clause reaching this product is the general third-party intellectual-property one, and asking in writing before any billing code is written creates a record that a post-launch discovery cannot.
Two merchant-of-record candidates prohibit this product by name and are ruled out at lines 89 to 91; FastSpring is the documented fallback at line 98 and Lemon Squeezy is not to be built on at line 99.

The uncalibrated-constant budget in `tam-limits` is the second hard gate, and the payment link may not be sent while it is above zero.
`crates/tam-limits/src/lib.rs:246` pins `UNCALIBRATED_BUDGET` at seven, and the crate header at lines 17 to 20 states the rule: an `UNCALIBRATED` marker is a guess, it is a release blocker for the first paying deployment rather than a wish, and the number is driven to zero before taking money.
Calibrating those constants against measurements from real runs is its own founder session; this runbook reads the number and calibrates nothing, and `crates/tam-limits` is a founder-gated file either way.

### Steps

1. Confirm the pre-approval reply is on file, with its date, before anything else.
2. Confirm the uncalibrated budget is zero: run the crate's test suite with `cargo nextest run -p tam-limits` and read `UNCALIBRATED_BUDGET` at `crates/tam-limits/src/lib.rs:246`.
   The ratchet test `uncalibrated_markers_are_ratcheted` fails when the marker count and the budget disagree, so a green suite with a non-zero budget means the gate is closed rather than open; the number itself is the check.
3. Create the payment link in the Stripe dashboard for the agreed amount.
   The price structure is unsettled: [`commercial-model.md`](commercial-model.md) records a $29-a-month subscription floor and, as the structure to test against it, a one-time onboarding fee plus a lower recurring price modelled at $299 one-time plus $19 a month.
   Choosing between them is a founder decision recorded there rather than here.
4. State on the link what is being charged for: the migration, meaning the bulk duplication of the seller's catalogue from the GB inventory into the New Zealand one, run and supervised by the operator.
5. Send the link to the customer directly.
   There is no signup, no trial and no self-serve flow at M1.
6. Record the charge against the tenant by hand: the org id, the payment link and payment ids, the amount, the date, and the job id of the migration it paid for.
   That record is the only one that exists.

### Success

A pre-approval reply on file, an uncalibrated budget of zero, a charge paid by somebody other than the founder — one of the three things M1 exists to prove — and a hand-kept record tying that charge to the tenant and to the job it paid for.

### Abort and rollback

If the pre-approval reply has not landed, or the uncalibrated budget is above zero, do not charge, and do not work around either gate.
A refusal discovered after customers are subscribed is the kill risk the first gate exists to prevent, and a guessed resource bound met by a paying tenant is what the second one prevents.
To undo a charge, refund it in the Stripe dashboard and record the reversal in the same hand-kept record.
Assume refunds the founder does not control are possible for sixty days under Stripe Managed Payments, which makes a sync failure inside that window a billing-continuity risk as well as a trust one.
Never admit a marketplace terms breach in correspondence with a customer or a processor, because the insurance exclusion at 8.13(b) triggers on an admission alone without any adjudication.
