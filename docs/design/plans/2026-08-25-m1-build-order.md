# M1 build order

M1 (Tes GB-to-NZ inventory duplication, 8-11 founder-weeks) is decomposed into build-ordered sub-plans, each producing independently-testable software.
The order respects the charter's "expensive boundaries first" rule and the dependency graph, and it front-loads the correctness core so everything above it is testable.
Much of the early work promotes the compile-clean `sketches/domain.rs` into production crates under the enforcement gate rather than designing anew.

| Sub-plan | Ships | Depends on | Kill-gate relevance |
|---|---|---|---|
| M1a foundation | production workspace, full enforcement gate, `tam-types`, `tam-marketplace` seam, `tam-domain` `SyncMachine`, `tam-storage` skeleton, `tam-api` library, thin binaries | M0 | draws the expensive boundaries |
| M1b storage and tenancy | full schema, org-first repositories, forced row-level security, two-tenant negative test, migration discipline | M1a | tenancy is the costliest retrofit |
| M1c Tes adapter | promote the spike into `tam-marketplace-tes`: create, metadata, presigned S3, publish, delete, read-back behind the fetch-reason capability, cassette harness | M1a, M1b | selector-fallback and maintenance gates instrument here |
| M1d job engine | job ledger, per-item leases and fencing via `write_attempt`, outbox drainer, automation worker, per-tenant mutex, durable circuit breaker, rate governance, pre-flight schema assertion, hourly canary | M1b, M1c | account-safety and correctness gates live here |
| M1e credential broker | `tam-secrets`, session broker as a separate process, per-connection envelope encryption, tested key escrow and recovery | M1a, M1d | breach-surface floor |
| M1f file pipeline | streamed ZIP with bomb and symlink guards, PDF and PPTX probing, cover and preview generation, malware scan, per-tenant content-hash dedup | M1b | none |
| M1g taxonomy hub | canonical taxonomy, GB and NZ projections via the deterministic prefix crosswalk, reconciliation queue, grade provenance, first-run drain instrumentation | M1b | the reconciliation-drain gate |
| M1h API surface | versioned nested router, idempotency keys on every sync-starting request, long-running operations as resources, cursor pagination, progress SSE as a projection of the ledger | M1a, M1d | none |
| M1i web client | React thin client: virtualised product-by-inventory table, per-inventory outcome bars with raw counts, per-item step timeline, downloadable per-item result, connections page with revoke and delete, blocked-on-seller state; per-marketplace status page | M1h | support-cost containment |
| M1j duplication and first charge | wire the GB-to-NZ duplication flow end to end, instrument the reconciliation drain across the first ten migrations, charge the first cohort by manual Stripe payment link | all above | proves somebody pays |

Each sub-plan is a separate spec-review-plan-implement cycle.
This document is the index; sub-plan files are `2026-08-25-m1a-...` onward.
