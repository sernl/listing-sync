# Decision record

Decisions settled in conversation on 2026-08-24 and 2026-08-25, before any code was written.
This record exists because these decisions are not derivable from the research documents, and several of them override those documents.
Research artefacts live in `../research/`.

## Product

The product bulk-uploads and cross-lists digital teaching resources across education marketplaces for teacher-authors, schools and agencies.
Phase-1 marketplaces are Tes Resources and TeachersPayTeachers, in that order.
Later candidates are Classful, Teach Simple and Made By Teachers.
Both sentences above are superseded by the wedge decision of 2026-08-25 below, which makes Tes GB-to-US duplication the first chargeable product, Etsy connector two, and TeachersPayTeachers a gated optional premium connector; the later candidates remain unscheduled rather than dropped.
The founder is an existing Tes author and is customer zero.

## Architecture

Automation runs server-side on infrastructure we operate.
This was a founder decision taken against the round-one research recommendation, which favoured local-first execution in the seller's own session on account-safety and credential-custody grounds.
The decision was made with those risks stated and is recorded here as deliberate rather than uninformed.

The client is thin.
Its job around sync is detailed progress reporting: files in sync, completed, failed, per marketplace.
Web client first, Android second.
Mobile is a full client rather than a read-only one, because the client performs no automation.

Rust is non-negotiable for the engine, all I/O, batch processing, the automation layer, and anything computationally heavy.
TypeScript is acceptable for the user interface only.

Sync is deterministic and cron-scheduled, never agent-driven.
Large language models are confined to listing-copy generation and to rediscovering a selector after a marketplace changes its markup.
No model decides what to sync, whether a sync succeeded, or how to recover.

## Credentials

Sellers supply their marketplace credentials, which we store encrypted with per-tenant data-encryption keys.
The founder recorded this as interim: "until we find a better way to have their creds."
Credential acquisition therefore sits behind a seam from the first commit, with one implementation today and room for two better ones later: an interactive remote browser the seller logs into themselves, and an official partner integration if either marketplace grants one.
The seam is cheap now and expensive to retrofit, which is the reason it exists.

## Hosting

The founder's own NixOS machine on a home connection initially, moving later to a dedicated NixOS box with production-grade server specifications.
This is load-bearing rather than a cost preference.
A probe on 2026-08-25 found TeachersPayTeachers returns HTTP 200 to a plain `curl` from the founder's consumer-ISP host, while earlier research measured HTTP 403 from Anthropic datacentre infrastructure.
TPT's `robots.txt` names `GPTBot`, `meta-externalagent`, `CCBot`, `ImagesiftBot` and `Applebot-Extended` in explicit stanzas, so the 403 was an AI-vendor block rather than a generic datacentre block.
If the eventual production box sits in a commercial datacentre, TPT reachability must be re-probed before it is relied on.
Egress must be fixed and declared in either case.

## Marketplace permission

No permission enquiries are being sent yet.
The Tes path is proved first.
This defers the question of whether the product has one marketplace or two, at no cost, because milestones M0 through M3 are Tes-only regardless.

## Engineering discipline

Guided by TigerBeetle's TIGER_STYLE and Holzmann's Power of Ten, filtered for what transfers to an async allocating Rust web service.
The filtering is the point: an adversarial review of the first charter draft found its flagship artefact did not compile and violated its own lint table twelve times, so the revised charter is gated on compilation.

## Scope explicitly deferred

The node-graph mapping canvas, in favour of a virtualised product-by-marketplace table.
The public versioned developer API, until a real third-party consumer exists.
Desktop clients, which the server-side architecture removes the need for.

## Open

Operating entity and jurisdiction are undecided, which forks the privacy regime, the consumer-law regime, the insurance market and the customer terms.

## Wedge, settled 2026-08-25

The first chargeable product is Tes GB-to-US inventory duplication, not cross-marketplace listing.
Tes runs disjoint GB and US inventories, so an author wanting both markets must upload twice under two resource ids at two prices against two taxonomies.
That is a bulk-tool job entirely inside one marketplace: no cross-marketplace mapping, no TeachersPayTeachers exposure, no bot-management vendor, and no dependence on the small TPT-and-Tes intersection.
It is bounded by Tes's whole author base — 23,266 American-oriented resources in a 1,039,401-resource catalogue — rather than by an estimated 4,000-seller overlap.
Every component it requires (catalogue, file pipeline, taxonomy projection, listing-copy rewriting, job ledger) is reused verbatim when a second marketplace arrives.

The Etsy Personal App and Commercial Access request is filed in week one and never placed on the critical path, because approval is a manual review with no published service-level agreement.
Etsy's API Terms grant the licence TPT and Tes both withhold.

TeachersPayTeachers is demoted to a gated, optional, premium connector.
Every screen must be complete and worth paying for without it.

## Demand evidence, recorded 2026-08-25

Ten measured TPT-and-Tes dual-listers hold 173 Tes listings against 7,644 TPT listings, 2.3% of their combined catalogue.
That figure is ambiguous rather than adverse: it measures behaviour under manual friction and cannot predict behaviour once the friction is removed.
The founder's own catalogue is the counter-evidence and the reason the premise is being pursued — most of it is on Tes only, because cross-listing by hand is not worth the time.
The founder is therefore customer zero for the exact problem, and M0's payoff is directly measurable against their own catalogue.

## Repository setup, settled 2026-08-25

The repository is `sernl/listing-sync`, private, on GitHub under the founder's personal account, transferable to an organisation later without history loss.
The licence is proprietary with all rights reserved and the copyright holder is `sernl`, recorded as interim and to be reassigned to a legal entity once the jurisdiction question is decided.
The crate prefix is `tam-`, kept as written across the design set; it was coined during research and carries no meaning, and a rename remains cheap while few crates exist.

Version control is jujutsu in colocated mode, with SSH-signed commits under `gist@tuta.com`, honouring the founder's global `signing.behavior = "own"` and `git.sign-on-push`.
The build skeleton is a crane-based Nix flake pinned to Rust 1.97.1, with the verified `tam-limits` crate as a green anchor.
The enforcement configuration is activated incrementally: the workspace-lints table ships at the first commit, and the `disallowed-methods` layer with its `ban-probe` lands in M0 alongside the dependencies it governs, because those bans emit unreachable-path warnings until the crates they name are present.
A signed `v0.0.0` prerelease tags this design-and-scaffold baseline and proves the tag-and-release pipeline before any product code exists.

## M-1 outcomes (interim), 2026-08-25

The M-1 kill gate passed: no express anti-automation clause binds a Tes author in the en-au or en-gb terms.
The Tes uploader is a cookie-authenticated JSON REST API plus a presigned direct-to-S3 file POST, so `tam-browser` is not built for the Tes adapter and `tam-marketplace-tes` becomes a `reqwest` client against the internal JSON API; the per-session browser isolation and forced-upgrade cadence are removed for Tes.
Drafts are the native resource state, so the create strategy is draft-then-publish and is idempotent, and the correlation-marker fallback is not needed for Tes.
Descriptions are Markdown, the durable key is the numeric resource id whose URL is fixed at creation, supported files are PDF, Word, Smartboards, JPEG, Powerpoint, Excel and ePub up to 200 MB, and the Bronze seller tier pays a 0.6 royalty banded by gross merchandise value.
The taxonomy is numeric ids with a publicly crawlable tree at `GET /taxonomy/v4/{country}/{id}`, so the vocabulary work needs no founder session.
The wedge is reframed: market targeting is a Curriculum field value on one account rather than a separate US login or inventory, and whether Curriculum American alone populates the US inventory is the open question that finalises the wedge.
Two write-path calls remain to capture, create-draft and publish, and the session-longevity window has not yet started.

## Wedge reorientation, 2026-08-25

The wedge target is duplication into English (UK) and New Zealand, not US, decided by the founder after M-1 showed market targeting is a Curriculum field value on one account rather than a separate inventory.
The source is mixed and is read per resource from the catalogue's existing Curriculum tags via the API, rather than assumed.
The mechanism is unchanged from the reframed wedge: the same cookie-authenticated JSON API and the same Curriculum-tag duplication, so nothing in the engine changes with the target market.
The commercial and localisation profile improves relative to the research's US premise, because English (UK) and New Zealand are British-English variants close to the founder's International base, so the AI layer changes spelling, curriculum and key-stage framing and year levels rather than performing a full US-English and Common Core rewrite.
The taxonomy crawl targets the country codes for these markets, GB and NZ, against the public `GET /taxonomy/v4/{country}/{id}` tree.
The research's US-centric market sizing is therefore no longer the operative commercial case; the founder is customer zero validating the UK and New Zealand targets against their own maths catalogue directly.

## M0 outcomes, 2026-08-25

The M0 spike proved the entire Tes draft write path server-side in Rust with no browser: create via `POST /api/v2/resources`, metadata via `POST /api/v2/resources/{id}/draft`, a three-step AWS presigned S3 file upload, read-back via `GET /api/v2/resources/{id}/draft`, and delete via `DELETE /api/v2/resources/{id}/draft`.
Both milestone kill gates passed: the server-side session authenticates from Rust, and an interrupted write classifies as ambiguous through a tested three-valued `WriteOutcome` type.
The decision is go for M1: the confirmed endpoints, the S3 handshake and the classifier promote into the production `tam-marketplace-tes` crate under the enforcement gate, and the throwaway `spikes/tes-spike/` is deleted at that point.
Publish is deferred to a later supervised step, so M0 deviated from the milestone document's live-state condition by the founder's decision not to publish, and both kill gates that do not require publishing were met in full.

## Publish and delete captured, 2026-08-25

A single free test resource was published live to the founder's store, confirmed by the founder, and deleted, capturing the two transitions M0 deferred.
Publish requires a valid `licence`; free resources use a Creative Commons licence (`CC-BY`, `CC-BY-SA`, `CC-BY-ND`), while `TES-PAID` requires a price.
The route this section originally recorded, `POST /api/v2/resources/{id}/publish`, is superseded by the 2026-08-28 browser capture of a draft-to-live publish: the route is `POST /api/v2/resources/{id}/draft/publish` and the body carries the full listing metadata rather than the licence alone — `title`, `descriptionRaw`, `descriptionRawType`, `mainType`, `mainAge`, `additionalAge`, `ageRanges`, `yearGroups`, `ages`, `categories`, `primaryCategory` and `customThumbnails` beside `licence` and, for a paid listing, `price`.
`price` is an integer in minor units, so `500` is GBP 5.00 under the fixed-GBP decision, and this publish is the one endpoint a paid listing's price reaches: the draft is created first by the unchanged create and `set_metadata` flow, and publishing re-posts the whole metadata with `licence: "TES-PAID"` and the price to take it live.
The authoritative published-resource delete is `DELETE /api/v2/resources/{id}`; the `/draft`-suffixed delete only removes the draft overlay and returns a misleading 204.
Verification of publish and delete must read the authoritative API, because the public resource URL soft-404s with HTTP 200, and a mutating call's own 2xx is not evidence the mutation took effect.
This is a live confirmation of the correctness discipline: the sync engine treats a marketplace's own success signal as untrusted and asserts state from an independent read.

## Client stack reorientation, 2026-08-25

The web client is written in Svelte and SvelteKit rather than React, decided by the founder at the start of M1i ("I want this written in svelte/sveltekit instead of react").
This supersedes the React-specific layer of `client-stack.md` — React, TanStack Router, TanStack Query, TanStack Table and Virtual, shadcn/ui on Radix, sonner, and react-hook-form with zod — none of which have load-bearing findings that survive the framework change; the document's stack-agnostic findings all carry forward unchanged.
What carries: Vite as the build tool (SvelteKit's own), a static output directory served by axum's `ServeDir` from a runtime path so a CSS tweak never invalidates crane's cargo artifacts, the one-flake-two-vendoring-mechanisms shape with `importNpmLock` over `package-lock.json`, the snapshot-plus-delta model with a single `EventSource` merging deltas into the client cache, the generated-vocabulary discipline with its freshness condition, and the entire PWA-over-native mobile analysis.
The dependency posture tightens rather than transfers: SvelteKit's filesystem router with generated route types replaces the typed-router dependency, a hand-rolled windowed list replaces TanStack Virtual, plain Svelte bindings replace the form stack, a hand-rolled store replaces the query cache (the cache-primitives argument was React-shaped), and Tailwind stays.
The vocabulary generator is a small in-house binary emitting TypeScript unions from the same closed Rust enums the cross-layer tests already pin, in place of ts-rs and utoipa, matching the hand-built OpenAPI decision M1h recorded; the freshness condition stands, enforced first in the web check lane and in `nix flake check` when the client's derivation lands.

## Limits calibration, 2026-08-28

Every `tam-limits` provenance marker was driven to a factual claim and the uncalibrated budget to zero before the first charge, per the crate's release blocker.
The browser module is deleted rather than calibrated: M-1 removed `tam-browser` from the design, nothing references the bound, and the crate's admission rule expels bounds on resources that do not exist; a future browser-driven connector re-admits one with its own evidence.
`http::UPLOAD_BODY_BYTES_MAX` stays 256 MB, measured against the Tes per-file ceiling of 200 MB recorded in the M-1 outcomes above.
`job::ATTEMPTS_MAX` stays 5, sized against `job::WALL_CLOCK_MAX` at the backoff base, over M0's tested fault taxonomy.
Four constants are held as decided operating points with named re-open triggers: `ingest::ARCHIVE_COMPRESSION_RATIO_MAX` 200 until the customer-zero import samples real bundle ratios, `job::CONCURRENT_JOBS_GLOBAL_MAX` 8 until the per-job RAM footprint is measured, the tier quotas until M5 sets pricing, and `marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX` 30 until the charter's written-terms answer arrives.
A `DECIDED` marker kind is added to the crate's provenance taxonomy for exactly this class: a deliberate operating point citing a dated entry here, neither a guess nor a measurement.

## Currency observation and charge postponement, 2026-08-28

Founder observation on the live uploader targeted at the New Zealand market: no currency control appears anywhere, price entry is GBP only, and the uploads dashboard displays prices in pounds; the priced-draft read-back and a screenshot of the form itself were not captured.
From one GB account that observation cannot distinguish a fixed-GBP marketplace from a seller-scoped currency, so `CurrencyRule::Unmeasured` stands for TesNz and the projection's refusal of priced items remains correct behaviour; independently, the adapter's write path carries no price field in M1, so priced duplication is unimplemented whatever the gate says.
The first charge is postponed by founder decision: no manual Stripe payment link will be sent, and charging waits for a self-serve checkout in a later milestone, so M1's somebody-pays proof is deferred to that point and the runbook's Stripe act is dormant, with both of its preconditions still binding whenever charging begins.

## The two uncaptured endpoints, resolved by a founder-supervised capture, 2026-08-28

A HAR capture of the author dashboard settled both endpoints the plan deferred, and both exist.
List-own-resources is a pair: `GET /api/v2/dashboard/getAllDrafts?page=N&limit=M` returns the author's drafts and `GET /api/v2/dashboard/getAllResources?page=N&limit=M` returns the published resources, each item carrying id, title, licence, price, the draft flag and the `/teaching-resource/...-{id}` url.
Own-file-download exists for a published resource as a two-step flow, confirmed by fetching real bytes rather than assumed: `GET /resource-detail/api/download/{id}` returns `{"zipUrls":{"{id}":{"url":"/teaching-resource/download/{id}/bundle"}}}`, and that bundle url 302-redirects to a signed CloudFront zip carrying the original files (verified: resource 13549126 returned a 471 KB zip wrapping the 493 KB source pdf, resource 13436008 a 12.5 MB zip).
A draft has no published bundle and the first step redirects to `?error=notfound`, which is correct because an unpublished draft is not a migration source.
The earlier finding in this section's first draft — that no download exists — was wrong: it probed the resource `attachments` array, which carries only a signed preview-image url and metadata, and missed the `/resource-detail/api/download` flow the resource-detail page's Download button drives.
The consequence reverses the constraint: the server can fetch a seller's published source files from Tes and re-upload them to build the NZ copy, so a full server-side migration needs no seller-supplied files, and both the listing and the file-fetch legs are now automatable.
The download returns a zip bundle, so the import fetches the bundle and extracts it through `tam-pipeline`'s existing zip ingest before re-upload.

## The NZ currency, fixed to GBP by observation, 2026-08-28

The founder observed on their own account that the New Zealand upload flow shows no currency control and that prices denominate in GBP.
`InventoryId::TesNz` is therefore set from `CurrencyRule::Unmeasured` to `CurrencyRule::Fixed(Currency::Gbp)`, which opens the projection's currency gate so a priced GB resource duplicates into the NZ inventory rather than blocking as `CurrencyUnknown`.
This supersedes the earlier deferral in "Currency observation and charge postponement, 2026-08-28", where the same observation was recorded but the gate deliberately left closed pending a priced-draft read; the founder has now decided the observation is sufficient to fix the currency.
The measurement is from one GB account, so the re-open trigger is a non-GBP seller whose NZ inventory prices in another currency; the gate is a founder code decision either way.
The gate's block path is unchanged for the still-unverified seller-scoped inventories (Etsy, Tpt), and a test pins that a priced listing into a seller-scoped inventory still refuses.

## The post-M1 sequence, reordered 2026-08-28

The founder ordered the post-M1 work as M6 first, then M7, then M2, ahead of M3, M4 and M5.
M6 is reframed from drift reconciliation alone to the cross-platform mapping core: the binding write, the per-inventory field registry, and the connector contract, with drift verification following on top of bindings rather than preceding them.
M2 is widened to include an operations dashboard: the analytics view over our own ledger data plus product operations such as delete and modify.
No marketplace exposes listing views through an API, so the dashboard's first tier is our own ledger, with platform-native sales metrics added only where an API exposes them.
This supersedes the milestone tail ordering in `milestones.md`; the milestone table itself is left as written and this entry is the record of the reordering.
The active plan for the first leg is `plans/2026-08-28-m6-mapping-core.md`.

## The TPT connector, unblocked 2026-08-28

The founder directed that the TPT connector proceed session-style like the Tes connector, without waiting for written permission from TPT.
This supersedes the milestone document's precondition that the connector not be built absent written permission.
The kill gate stands unchanged: any written objection from TPT or IXL kills the connector.
The consent model is seller delegation in the shape of TPT's own VA-login programme, where a seller grants store access deliberately, and the connector acts only on the seller's own listings under the first-party capability tier.
The four M7 measurements from the milestone prose — session longevity, bot-management posture, the real mandatory-field set, and the undocumented bounds — still open the milestone.

## M7 TPT, read-buildable and write-capture-gated, 2026-08-28

Six HAR captures of the founder's own TPT seller account were analysed to open M7.
They fully expose TPT's read surface and write architecture but contain no write: every capture records browsing (the add-product and bundle forms at load only, the product grid, the dashboard, the store, and statistics), and the only GraphQL mutation present is an automatic privacy-consent toggle.
Three facts separate TPT from TES.
The product write is a legacy CakePHP multipart form guarded by a SecurityComponent token hash over its field set, not a GraphQL mutation, so the connector must GET the form and replay its tokens before each write.
Files upload out of band through Evaporate-to-S3 and a Filestack workflow, with the returned handle written back into a hidden form field, and none of that sequence is captured.
A three-layer bot-management stack fronts the origin — Cloudflare Bot Management minting cf_clearance from obfuscated-JS fingerprinting, reCAPTCHA Enterprise v3, and Sift — all passive in the captures, but every capture rode a pre-existing cf_clearance cookie and none exercised a gated write.
The read slice is buildable now on the M6 seam and is not rework-prone: the cookie-plus-double-submit-CSRF envelope, the MyProductListings enumeration query, and TPT's vocabularies are captured and stable.
The write slice is gated on one further artefact: a capture of a product created and published from a browser with tracking protection disabled, which settles the reCAPTCHA-on-submit question, the upload sequence, the mandatory-field set, the price and length bounds, and whether the write is the CakePHP form or a newer GraphQL mutation.
Downstream of that capture is an architecture fork that this entry does not settle: if a broker-established cookie jar passes from a server egress IP the connector is the TES session shape, but if Cloudflare or reCAPTCHA rejects a non-browser client the connector needs a headless browser at the egress IP, which reopens the tam-browser removal recorded under "Limits calibration, 2026-08-28" and is a founder decision.
Correcting the M2 platform-API finding: TPT does expose per-resource views, sales and earnings for a seller's own resources through the gateway's storeResourceStatsNext, so the M2 dashboard has a real TPT analytics source.
The active plan is plans/2026-08-28-m7-tpt-connector.md.

## M7 TPT write path is plain-HTTP, no browser, 2026-08-28

Two founder-supervised write captures — one metadata edit of a live product, one new-draft creation with a file upload — settled the write path and the architecture fork left open by the read-buildable entry above.
No reCAPTCHA, Turnstile, Sift, or Cloudflare token is attached to either write; an exhaustive search of all 511 entries across both captures found no captcha field, header, or query parameter on any write, and the reCAPTCHA Enterprise token minted on page load is posted only to Google and never forwarded to TPT.
Both writes are legacy CakePHP multipart form POSTs authenticated by the cookie session plus the SecurityComponent token triple and the CSRF pair scraped from the form page, with the double-submit x-csrf-token header present only on the XHR upload endpoints and absent from the form navigations themselves; the tokens are not single-use.
The create flow is an eleven-hop chain: GET the form for its tokens and the AWS config, reserve an S3 key at /uploads/upload_file, obtain the server clock at /uploads/time, sign each S3 request through /uploads/sign_auth — a server-side AWS Signature V2 oracle that returns an HMAC without ever exposing the secret, so the connector needs no AWS credentials — run the S3 multipart upload against the live.digital.upload bucket, hand the object to /uploads/process_file whose x-queue-tracking-id response header is the job token, poll /queue/results until it returns the file handle, request thumbnails at /converter/generate_thumbs and poll again for the collection key, then POST the form with the handles filled in.
The edit flow re-uploads nothing: existing asset handles are echoed back verbatim and only metadata changes.
The verdict resolves the fork in favour of the TES shape: a server-side plain-HTTP connector carrying a broker-established cookie jar reproduces both writes, and the tam-browser removal stands.
The one residual browser-shaped dependency is not the write but the session: cf_clearance was carried into both captures rather than minted in them, and login was never captured, so whether a cold server IP is served the form directly or challenged, and whether login requires a reCAPTCHA token, are unproven — both are session-establishment concerns for the broker's link step, handled as TES's session is, and neither blocks building the write path against cassettes.
The active plan is plans/2026-08-28-m7-tpt-connector.md, whose Task 3 now carries the full recipe.

## The transport seam carries a closed header allow-list and a closed request auth, 2026-08-28

The M7 write path needs a response header (the 302 `Location` and the upload queue's tracking id) and a per-request S3 signature, neither of which the seam could carry.
A general header map on either side would have deleted the seam's no-secret invariant rather than extended it, because a map that can hold `Location` can hold `Set-Cookie` and a map that can hold an AWS signature can hold a session cookie.
`HttpResponse` therefore gains `headers: Vec<(ResponseHeader, String)>` over a closed `ResponseHeader` allow-list, projected from the real header map at the live boundary, and `HttpRequest` gains a closed `RequestAuth` rather than headers of its own.
`RequestAuth` has three variants: `Session`, naming the credential the live transport holds at construction and carrying none of it; `Anonymous`, for a request whose authorisation travels in its own body, which is what the Tes S3 POST-policy upload is; and `S3SigV2`, carrying an ephemeral signature over one object.
`Anonymous` is an addition to the two-variant enum the design named, because without it the Tes upload — a multipart POST whose policy and signature are form fields — could only be described as `Session`, and an auth-dispatched transport would have had to send the seller's cookie to Amazon to send it at all.
Both new fields serialise under `skip_serializing_if`, so every committed cassette fixture stays byte-identical and reads back unchanged.
`RequestBody` gains `Bytes` through the same serde helper a response body uses, `MarketplaceAdapter::submit` gains `now: Timestamp` mirroring `read_back`'s `observed_at` because `SystemTime::now` is disallowed and a write hop needs the caller's clock as data, and a `Pause` capability trait joins the seam so an adapter can wait between hops without this crate acquiring a runtime.

## The Tes session cookie no longer travels to Amazon, 2026-08-28

`ReqwestTransport` put the cookie jar in reqwest's `default_headers`, which apply to every host a client reaches, so the Tes upload's multipart POST to `*.s3.amazonaws.com` sent the seller's full Tes session to Amazon on every file.
The transport is now two clients: a session client holding the jar, and a bare client carrying nothing but a user agent, chosen by what a request declares about its own authentication.
A host assertion refuses the mismatches outright — a session-authenticated request to the bucket, or a signed one to the marketplace origin, is `NotSent` and never leaves — so the leak cannot return by a flow building the wrong request value.

## The lifecycle seam, the sever, and the interim TPT custody, settled 2026-08-29

M8 Phase 3 lifts the write seam from create-only to the full lifecycle, and the decisions below are the ones a reviewer should not have to re-derive from the diff.

Publishing is not a third trait method.
`MarketplaceAdapter` gains exactly two — `revise` and `remove` — and a publish is `revise(.., to: Live)`, which makes the four-cell transition table the whole publish capability rather than a capability beside it.
The transition travels whole, `from` and `to` together, because both marketplaces' state routes lag the write that produced them (Tes measured 2026-08-29, TPT 2026-08-28), so a probe run soon after a write reads a live listing as a draft; a caller that just wrote always knows what it wrote.
Every cell posts and classifies and none of them verifies, because only the driver holds a rate budget to poll with — which meant splitting Tes's self-verifying `publish` and `delete` into their unverified halves, leaving the inherent methods and their inline reads exactly as the live runners use them.

The machine asserts form schema on `Create` only.
On Tes that is close to free: `assert_form_schema` is write-bearing — it creates a probe draft, writes it, reads it and deletes it on the seller's real store — so asserting on every revise would multiply probe drafts by the size of a bulk revise, and `tam-canary` already fingerprints the same `set_metadata` surface a `Draft → Draft` revise posts, on a schedule.
On TPT it is not free, and Phase 3 accepts the gap rather than papering over it: the canary loops over the two Tes inventories and never TPT, and both adapters' `assert_form_schema` probe the *create* form, so `FormTarget::EditDigital` is fingerprinted by nothing, anywhere.
The mitigating fact is that every TPT revise re-scrapes the edit render at write time and classifies a drifted form as a loud rejection — a scrape failure or a `SubmitNoConfirmation` bounce — so drift cannot silently misfire; what is missing is early warning, not detection.
The recommended follow-up is to extend `tam-canary` to TPT with an edit-render probe, a single read-only GET needing a probe-subject strategy and the TPT session, alongside the existing Tes loop; it is founder-gated and not built in Phase 3.
Reversing the decision for TPT alone was considered and refused, because it would make the machine's preflight branch a function of the adapter rather than of the operation.

The item stores its transition and the listing it means to act on.
The transition is stored rather than derived because no column records which side of the draft line a listing sits on: `mapping.lifecycle_state` ranges over seven values, is written at insert and never again, and `mapping.publish_mode` is a sync policy about whether to publish at all.
The subject is stored as the enqueuer's assertion while the mapping's binding stays the authority, and the engine refuses an item whose stored subject diverges from it — which is what makes a rebind between enqueue and lease detectable rather than a silent retarget onto another listing.
With both ends stored, `publish` carries no information the columns do not, so the stored operation set is three — `create`, `revise`, `remove` — isomorphic to the domain's `ItemOperation`; a seller-facing publish action lowers to `revise { to: Live }` at the API boundary in Phase 4.

A committed removal severs the binding rather than binding to what it removed, with `sever_cause = 'removed_by_seller'`, and this is the first sever the engine writes.
The cause is the seller's because the seller's own job asked for it; the other two members name the marketplace and a failed verification.
The sever is coupled to a change on the create path: the bind's prior-state fence now admits `'severed'` and `prepare_item`'s `Create` gate admits `Binding::Severed`, because `mapping_one_per_inventory` is unpredicated and forces a re-create to reuse the same row — without both, a severed mapping could be taken down and never put back, and the migrate story the sever exists for would not work end to end.
What authorises the sever is `write_attempt_one_in_flight` rather than the attempt's `lease_epoch`, which is written and compared from the same `LeaseRef` and cannot mismatch within a run; the remaining exposure is a stalled worker whose lease was stolen severing anyway, and Phase 3 makes that visible with a `SeveredAfterSteal` bind anomaly rather than adding fencing that would change the create path too.

The idempotency key stops being purely content-addressed for non-creates.
A create still hashes its payload files and its key is byte-identical to the one it minted before, by delegation rather than by arithmetic that would have to be re-checked.
A revise carries no payload change at all — a price fix and a title fix hash identically — and a removal followed by a re-create reproduces the first create's digest, so under the old key the second of any such pair was refused by `job_item_idempotent` permanently, `job_item` rows never being deleted.
Revises and removals are therefore identified by the job that asked for them, which keeps duplicate protection within a job and deliberately drops it across jobs, where it was never wanted: a seller may legitimately ask twice, and a content-addressed key cannot express that.

TPT credential custody in Phase 3 is a founder-exported cookie jar read from configuration, and that is interim.
`TAM_TPT_COOKIE_JAR` and `TAM_TPT_AUTHORSHIP` are read once at the worker's process boundary; a TPT item still cannot lease without a `linked` TPT `connection` row, so the vault row functions as the queue gate and `gate_connection` still stops TPT items, while the credential actually sent never came from the vault.
This means the worker process holds a seller credential in plaintext on disk, which is precisely the invariant the broker exists to prevent, and it is a deviation from the M7 entry's broker-established jar rather than an implementation of it.
Three answers were considered.
Keep the file jar, which is what Phase 3 ships and what this entry exists to stop calcifying.
Give the broker a hand-me-the-cookie-header operation for direct-transport marketplaces, which deletes the no-secret-to-worker invariant outright rather than extending it, and is not recommended.
Give the broker a TPT gateway with its own allow-list and let only the bucket hops leave the worker directly, which preserves the invariant whole: the S3 leg does not need proxying, because those hops already declare `RequestAuth::S3SigV2`, the live transport's host assertion already refuses a session-authenticated request to the bucket and a signed one to the marketplace origin, and every marketplace-origin hop in the create chain is session-authenticated.
Option three is the recommended production answer and is out of Phase 3's scope.
The authorship attestation is configuration for the same interim reason; the durable answer is two columns on `connection` written by the broker's link step, in M2.
Because one process-global credential speaks for exactly one seller while the lease scan is cross-tenant, `TAM_TPT_ORG` pins the jar to the organisation it belongs to and the worker refuses every TPT item from any other, leaving its lease to expire; without the pin a second tenant's items would execute against the configured seller's account, and a removal would read absence from the wrong catalogue and sever a mapping whose listing is still live.

`OUTBOUND_REQUESTS_PER_MINUTE_MAX` bounds effects per connection per minute, not requests, and always has: one submit issues a create, a metadata write, a three-step upload per file and a state read, and consumes one grant for all of it.
The verification poll now consumes one grant per `read_back` call, which narrows the gap on the verify path and leaves it wide on `submit` and `assert_form_schema`, the expensive ones.
Moving consumption into the transport seam is the only place the constant can be made a true request ceiling; it re-tunes every existing flow against a bound they have never been measured against, so it is deferred and founder-gated, and the constant is not renamed in Phase 3 because that is founder-gated too.

The verification poll must fit inside the lease, and the inequality `submit_worst_case + tries × interval < LEASE_TTL_SECS` is asserted and documented rather than made true by raising a limit.
On today's numbers it holds for the measured TPT create — roughly 180s against a 300s lease, with 22s of poll on top — and fails at that platform's theoretical worst case, where two queue-job polls alone can spend 360s.
The failure is stall-biased: the attempt stays in flight and no duplicate listing is ever created.
Raising `LEASE_TTL_SECS` is a one-line founder decision that removes the worst case at the cost of widening the window a dead worker's item is stuck in; renewing the lease across long effects is the durable fix and goes on the M2 list.
Neither is implemented in Phase 3.

`sha2` is hoisted to `[workspace.dependencies]` and TPT's write evidence now carries a `response_body_digest` on every cell, including the pre-existing `submit`.
The digest is the write-evidence contract, TPT is the adapter most likely to produce ambiguous write evidence — a scraped form and bounce semantics — and an asymmetry where only one adapter answers the contract erodes it.
No new crate enters the closure: `sha2` was already a direct dependency of `tam-marketplace-tes`.

## Phase 4 mapping and orchestration, settled 2026-08-29

A projection that cannot carry a value fails in four ways with four remedies, so the outcome names four things apart rather than flattening them into one error.
A *gap* is a question about a vocabulary pair: answering it writes a durable edge and every later product finds it waiting, which is what makes that queue drain.
An *election* is a question about one product against one target — which licence this seller grants, which of the year groups a band covers this listing means — and it cannot deduplicate across products, so its reuse mechanism is a standing rule rather than an index.
A *loss* is not a question at all: it is disclosed and never blocks, because a Tes licence going to a platform with no licence field has no possible answer and raising it would create a queue that never drains.
An *unrecognised* source value is none of the three, because the reconciliation queue's own key references a canonical term, so a value the relation has never seen cannot become a queue item and calling it a loss would assert the target has no such field when the truth is that we do not know what the value is.

The two queues are two tables because they have two keys.
`reconciliation_item` deduplicates on the vocabulary pair while open, which collapses a five-hundred-product batch into one question; an election's key is the product, so the same index would collapse the wrong thing, and a supply or over-cap question names no single term to key on at all.
The reuse mechanism for elections is a standing rule the seller states once, consulted before anything is enqueued, which is what reconciles "anything unclear is the seller's decision" with "a decided equivalence does not re-ask on the next product".

Equivalence scope splits by kind.
Vocabulary-level equivalences stay global, because a TPT fourth grade being a Tes year group is a fact about the world, and making it tenant-scoped would have every seller re-answer the same crosswalk — which is the treadmill the M1 kill gate exists to detect.
Seller elections are tenant-scoped, because a legal and presentational choice is theirs alone.

The licence is never delegable.
Tes binds it and requires it, TPT declares no licence axis at all, and that single absence is the legal exemplar declared as data.
A rule delegating a licence to best fit is refused by the domain's own constructor and again by a database CHECK, because the domain check alone passes for anything that writes the row directly.
A Tes-to-TPT licence drop is a surfaced loss and never a silent one; a TPT-to-Tes create with no stated licence raises an election and blocks, where before it published under CC-BY — a perpetual irrevocable grant the seller never chose.

A migrate's removal waits on a predicate about the world rather than on another item.
`Binding::Bound` is written only from the driver's own verification read, so "the counterpart exists and we saw it" is exactly what the gate asks, and a dependency on a specific item would strand the removal the first time a failed create was re-run as a different job.
The unsafe direction — source gone, target absent — is therefore unreachable, and the failure mode is a duplicate.
The gate also has a terminal arm, because a parked item is invisible to every existing reaper and would otherwise cycle park to queue to park forever with the job reading active.

The read leg of a sync runs as `tam_app` in its own process.
It has forced row-level security, so it pins one organisation per request and cannot read two tenants' rows in one statement — a stronger tenancy posture than the item pump's cross-tenant scan.
The alternative was granting the engine authorship of the catalogue, which is what the grant enumeration exists to prevent.
The consequence is that Phase 4 adds no engine *write* grant on any catalogue table, and `sync_request` takes no engine grant at all: `native_residue` takes the same SELECT the rest of the product aggregate already has under 0016.
Of the milestone's two new engine write grants, `election_item` and `mapping_loss` both mirror the existing reconciliation one exactly, and `election_rule` is deliberately SELECT only — a standing rule is the seller's to write.
The cost is named rather than hidden: this process becomes the second holder of the TPT cookie jar, applying the same one-tenant pin the worker does, when the TPT seller-download capture lands.
Until then it holds no credential at all — only Tes has a captured source read, so `POST /{v}/sync` refuses any other source and the drain records a terminal failure for one written before that refusal existed, rather than leasing a gateway for a request it can never serve.

A publish is its own operation because the pair is otherwise unconstructible.
Both adapters create a draft, so reaching live from nothing is two writes, and the second cannot name its subject when the seller asks for it — the create binds that id minutes later.
This reopens a shape Phase 3 closed on purpose and the cost is stated there; the alternative was making the seller's one action two round trips for every new listing.

Three things implementation found that the plan had wrong, recorded so they are not rediscovered.
Narrowing `projection_edge`'s primary key would have forbidden the very relation the grade axis needs — one band covers eight year groups — so the single-valued rule is a partial unique index excluding narrower edges, which delivers the stated purpose exactly and over-delivers nothing.
Two CHECKs govern `job_item.operation`, not one, and widening only the shape constraint admits a value the value list still refuses.
A grade path the relation does not recognise is now carried out as unrecognised rather than relabelled into the target vocabulary and posted, which means a product whose grades were never seeded publishes with no grades until the seeder has run: the one place the fix trades a wrong value for an absent one.

## The TPT currency, fixed to USD by the founder, 2026-08-29

The founder checked their own TPT seller account and confirmed the marketplace sells in USD and offers no other currency to select.
`InventoryId::Tpt` moves from `CurrencyRule::SellerScoped` to `CurrencyRule::Fixed(Currency::Usd)`, which supersedes the Tpt half of "The NZ currency, fixed to GBP by observation, 2026-08-28"; Etsy stays seller-scoped and unverified.
This settles G-O6 and unblocks paid TES-to-TPT sync, which the projection's currency gate had been blocking as `CurrencyUnknown`.

Two refusals stand in place of a conversion.
A TPT read whose amount is rendered with anything but a dollar sign is refused rather than redenominated, because the rule and the wire would then disagree and picking one is not a resolution.
A projection into TPT carrying a price in another currency is refused at `project_fields`, exactly as the Tes adapter refuses a dollar price into its GB inventory: TPT's wire carries a bare amount, so a pound price would sell the resource at that many dollars.
A seller converting a GBP listing states the target USD price through the pricing election; no exchange rate is invented anywhere in this system.

