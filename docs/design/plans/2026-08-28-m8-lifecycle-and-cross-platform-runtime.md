# M8: the lifecycle and cross-platform runtime

Goal: turn the M6 framework and the M7/M1 adapters into runnable live operations — the full create/read/update/delete lifecycle on each platform, publish-to-live, and the cross-platform sync, migration, bulk and publish-to-both flows the founder's live-test battery exercises.
This milestone was opened when the founder's live-test request revealed that M6 delivered the mapping framework and M7 delivered a certified TPT adapter, but neither wired a runnable CRUD or cross-platform runtime; the single wired write path was TES-only, free-only, create-only, draft-only.
The live TPT read (commit 31bdd9ca) proved first-contact works: a browser cookie jar passes from our egress IP with no Cloudflare challenge, so live TPT runs from our infrastructure.

## The base standard

Every platform, current and future, gets the same lifecycle as a base standard: create, read, update, delete, publish, and participation in cross-platform sync/migration/bulk.
This is enforced by putting the lifecycle on the shared `MarketplaceAdapter` seam rather than in per-adapter inherent methods, so a new platform implements the trait and inherits every orchestration.

## Decisions this milestone makes

- The `MarketplaceAdapter` seam gains `update` (edit an existing listing), `delete` (remove a listing), and a publish capability (draft to live), joining `project_fields`/`assert_form_schema`/`submit`/`read_back`. The engine's closed `Effect` set gains the matching effects so the driver can interpret them.
- The worker becomes multi-platform: it routes each inventory to its adapter and transport — TES over the broker gateway (existing), TPT over a direct live transport (validated live, since the broker cannot proxy TPT's S3 upload). A new platform slots in by the same match.
- TPT delete is the captured `RemoveResource($id)` GraphQL mutation on `/graph/graphql` (`resourceDelete(input:{id}, sellerId)`), returning the deleted id. TPT update is the captured `editNext` form (the M7 `publish` path generalised to change fields, not only status). TPT paid create relaxes the M7 free-only refusal to carry a price, using the `price`/`free`/`license_price` fields the edit capture proved, gated on the founder's paid authorisation.
- TES delete is the documented `DELETE /api/v2/resources/{id}` for a published resource and the existing `/draft` delete for a draft, both verified by a follow-up read. TES update reuses the existing `set_metadata` step against an existing draft id. TES paid create was BLOCKED pending a founder capture of a paid TES create, because the price wire format had never been observed and would not be invented; the captures arrived as `tes-publish-product-from-draft-to-live.har` and `tes-delete-live-product.har`, and the gate is closed. Paid create, publish and delete are proven live: publish is `POST /api/v2/resources/{id}/draft/publish` re-posting the full listing metadata with `licence: "TES-PAID"` and `price` as an integer in GBP minor units, and the published-resource delete is the `DELETE /api/v2/resources/{id}` route this plan already documented.
- Cross-platform sync reads a product from platform A into the canonical model and creates it on platform B: TES-as-source uses the existing import; TPT-as-source needs `fetch_for_import` built (the M6-deferred import-run generalisation). A sync job seeds a target-B mapping from a source-A product.
- Migration is sync to the target followed by delete on the source, so the listing exists only on the target; it needs delete on the source platform.
- Bulk is N products in one job (the job mechanism already has no cap). Publish-to-both is two jobs, one per platform, each carrying its own draft-or-live intent.
- Live test products carry `ZZ-DELETE-ME`-class titles and either a generated one-page PDF or a small real file from `~/downloads/ebmc/TES/`, never colliding with the founder's real catalogue, always deleted immediately after verification. Paid-live tests use a minimal price and are removed the moment they are confirmed live.

## Phases

### Phase 1: TPT single-platform live ops (fastest live proof)

The TPT delete op (`RemoveResource`), the update/edit op (`editNext` generalised), and paid create (price-carrying), plus a self-cleaning `examples/live_write.rs` (create draft, optionally edit, optionally publish, then delete and verify gone) mirroring the TES `live_smoke`.
Cassette-green first; then a controlled live run proves create/paid/edit/delete on the founder's account.

### Phase 2: TES lifecycle gaps

TES published delete (the documented endpoint), TES draft update (via `set_metadata`), and the TES `live_smoke` extended with delete-by-id and edit modes.
TES paid folds in when the capture arrives.

### Phase 3: the seam and engine lifecycle

`update`/`delete`/publish on the `MarketplaceAdapter` seam; the matching engine effects and driver interpretation; the multi-platform worker routing; the M6 bind firing on live writes so mappings become bound.

### Phase 4: cross-platform orchestration

TPT mapping mint; `fetch_for_import` for TPT-as-source; the sync job (read canonical from A, project to B, create on B); migration (sync + delete-source); bulk; publish-to-both.

### Phase 5: the live battery

Run the founder's full list live with cleanup, reporting each result: the single-platform CRUD on both, the four cross-platform sync/bulk runs, publish-to-both, and the two migrations.

## Deferred / gated, with owners

- TES paid create: was gated on a founder capture of a paid TES create; the capture arrived and the gate is closed, with the wire format recorded in the TES decision above. Owner: founder, discharged.
- The 401-vs-403 ambiguity in TPT `classify_graphql_read` (a Cloudflare 403 currently reads as an expired session): fold a distinct classification into Phase 1 so a live run tells refresh-cookies apart from IP-blocked. Owner: Phase 1.
- Streaming large uploads, marker-based reconciliation, and the broker routing TPT: out of scope; TPT rides the direct transport.
