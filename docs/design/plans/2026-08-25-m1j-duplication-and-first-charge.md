# M1j: duplication and the first charge

Goal: the GB-to-NZ duplication flow wired end to end through everything M1a–M1i built — import, projection, the pumps, the write path, the client — instrumented for the reconciliation-drain kill gate, with the first charge taken by manual Stripe payment link.
This is the milestone that closes M1: "a bulk job of realistic size completes with per-item honesty rather than a green bar", proven first against an in-process fake Tes, then live under founder supervision.

## The boundary this plan draws

Code can wire, prove and rehearse the flow; three acts are the founder's and are runbook items rather than tasks: linking the real Tes session through the broker, the first live duplication run (supervised, on the founder's own catalogue, publish gated), and sending the Stripe payment link.
Two marketplace facts are uncaptured and stay fail-closed until a founder-supervised capture lands: the list-own-resources endpoint (the HAR holds only the upload flow) and the own-file download endpoint.
The import therefore runs per resource over the proven `GET /api/v2/resources/{id}/draft` read under the existing `FetchReason::FirstPartyExport` capability, with resource ids operator-supplied, and file bytes for customer zero come from the founder's own disk through the ingest pipeline that already exists — the marketplace-download leg is a recorded capture step for the first external customer, not a guess shipped now.

## Decisions this plan makes

- The composed projection lives in `tam-taxonomy` as `project_listing`: the four publish gates in one pure function — `Taxonomy { items }` from `project_terms`, `CurrencyUnknown` from the inventory's `CurrencyRule` (a `Free` price needs no currency, so free listings pass the NZ gate the currency probe has not yet opened; a priced one blocks honestly), `CoverMissing`, and `ScanIncomplete { file }` for any payload not scanned clean.
  Copy is projected verbatim in M1: title and body pass through unchanged (localisation is M4's), and grade paths pass native ids through unchanged because Tes's age-range vocabulary is account-scoped, not inventory-scoped.
- The worker pump is the `SyncMachine` driven for real: `tam-worker` scans leases on the engine role, and per item runs the projection, opens the fenced `write_attempt`, drives the Tes adapter's create-metadata-files-publish flows through a broker lease, read-back verifies under `VerifyAttempt`, and settles the three-valued outcome into the ledger the client already renders.
  `PublishMode::DryRun` stops before publish and settles what it proved; the live mode is the same code path with the gate open.
- The pipeline pump drains ingest work the API enqueues; the import command turns operator-supplied resource ids and file paths into canonical products: metadata and taxonomy from the per-id read (inbound `ingest` over the seeded crosswalk, unmapped paths retained verbatim and raising inbound items), files through `tam-pipeline`'s existing ingest.
- The service loops turn on where the design put them: the outbox drainer inside the API process, and the job-event pruner advancing the watermark the stream's resync already reads.
- Drain instrumentation is a per-import report: terms seen, terms already covered, new items raised — the first-run share the kill gate compares across the first ten migrations — recorded as a job event and surfaced in the client's reconciliation page.
- The end-to-end proof is an in-process fake Tes shaped by the cassettes: create, metadata, presigned S3, publish, read-back, each shape the spike proved, driven from enqueue to settled through the real worker loop, twice — a clean run settling `Committed`, and a gauntlet run proving per-item honesty (one item taxonomy-blocked into the queue, one rejected, one ambiguous halting the mutex).
- Stripe stays manual by design: the runbook records the payment-link flow and what is charged for; automated billing is M5's.

## Tasks

Task 1 — `project_listing` in `tam-taxonomy`: the four gates, verbatim copy, pass-through grades, file manifest from the payload set; unit tests per gate and a green projection over the seeded crosswalk.
Task 2 — import: `tam-marketplace` gains the import read shape under `FirstPartyExport`; `tam-marketplace-tes` maps the per-id draft read into a canonical import; a `tam-import` one-shot (ids plus file paths in, products plus mappings plus drain report out) exercising inbound taxonomy ingest; pg tests over a fake read.
Task 3 — the worker pump: the lease-scan loop, the machine driven against the adapter with the broker lease, fenced attempts, read-back, settle; the pipeline pump; both behind a supervised loop with clean shutdown.
Task 4 — the service loops: outbox drainer in the API process, the pruner, both interval-driven and tested.
Task 5 — the end-to-end gauntlet against the fake Tes, clean and hostile runs, client-visible.
Task 6 — the close: runbook (link, currency probe, supervised first run, ZZ-prefix rehearsal, Stripe link), instrumentation wired to the client, gates, sandbox, the phase table's last row.

## Deferred, with owners

- List-own-resources and own-file download endpoint captures: founder-supervised, before the first external customer's import.
- The NZ currency probe: founder-supervised, before the first priced NZ listing; the gate stays closed and free listings do not need it.
- Localised copy (spelling, curriculum framing): M4.
- Automated billing, tiers, signup: M5.
- Scheduled sync (cron-driven re-runs): after the first manual-run cohort proves the shape.
- Browser end-to-end (Playwright), deferred to M1j by the M1i plan: re-deferred, owner founder, before the first external customer's onboarding; the in-process Rust gauntlet covers the flow end to end for M1j.
- The xtask-dependent checks in the enforcement toolchain's thirteen-check table — lint-configuration scan, crate-root attributes, dependency closure, migration lint: owner founder, when the xtask crate first lands; the no-new-code subset of that table is wired in this close.
- The pg_notify lease-kill fan-out, named for M1j by the M1e plan and not built: owner founder, before the first external customer; revocation kills gateways and the linked-connection gate holds every new lease, but an in-flight lease runs to completion, so revocation latency is bounded by one item's write.
- Resource-type vocabulary seeding, named for M1j by the M1g plan as "when the duplicator consumes it" and shipped unconsumed: owner founder, at the first external customer or when NZ listing quality demands it; duplicated listings carry no resource-type mapping in M1 and the projection maps Subject and Topic only.
