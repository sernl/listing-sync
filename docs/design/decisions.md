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
This paragraph is superseded for marketplaces with no official API by the architecture reversal of 2026-09-03 below, which moves request origin to the seller's own device for those marketplaces and leaves it server-side for the marketplaces that publish an API and issue a token for the purpose; the round-one research recommendation it was taken against is the position now adopted for the first branch.

The client is thin.
Its job around sync is detailed progress reporting: files in sync, completed, failed, per marketplace.
Web client first, Android second.
Mobile is a full client rather than a read-only one, because the client performs no automation.
The last three sentences are superseded by the same 2026-09-03 entry.
The client is no longer thin for a no-API marketplace, because it composes and issues every request to one; the surface order becomes Windows desktop first, then Android, then iOS, with the web console retained and the browser extension deferred; and a phone is a full client for API-branch marketplaces and a start-it-yourself client for the rest, because no phone can run a deterministic schedule.

Rust is required for the engine, all I/O, batch processing, the automation layer, and anything computationally heavy.
TypeScript is for the user interface, and for one bounded identity service.
That service is better-auth, running as `tam-auth`, and it owns platform-user identity and browser session only: registration, sign-in, social and passkey credentials, email verification, password reset, and the keys for the tokens it issues.
It owns no domain data, performs no marketplace request, and holds no marketplace credential.
It reaches Postgres only as the `tam_auth` role, whose grants are confined to the `auth` schema; it never reads or writes a table in `public`, never sets `app.current_org`, and never contacts the session broker.
Authorisation — which organisation a request speaks for and what it may do there — is decided in Rust from Postgres, and is never asserted by a token claim.
Any extension of this service beyond identity and session is a new founder decision, not an application of this one.

Sync is deterministic and cron-scheduled, never agent-driven.
Large language models are confined to listing-copy generation and to rediscovering a selector after a marketplace changes its markup.
No model decides what to sync, whether a sync succeeded, or how to recover.

## Credentials

Sellers supply their marketplace credentials, which we store encrypted with per-tenant data-encryption keys.
The founder recorded this as interim: "until we find a better way to have their creds."
Credential acquisition therefore sits behind a seam from the first commit, with one implementation today and room for two better ones later: an interactive remote browser the seller logs into themselves, and an official partner integration if either marketplace grants one.
The seam is cheap now and expensive to retrofit, which is the reason it exists.
This section is superseded for marketplaces with no official API by the architecture reversal of 2026-09-03 below: as of that date no marketplace credential and no marketplace session is held centrally for that branch, the session lives on the seller's own device and is never handed to us, and the central vault applies only to the official-API tokens of the marketplaces on the API branch.
The seam the section exists to preserve is discharged rather than replaced, because the third and best implementation it left room for turned out to be the seller's own device rather than a better server-side one.

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
The desktop-client entry that stood here is struck by the 2026-09-03 architecture reversal below: a desktop client is now the first surface rather than a need the server-side architecture removes.

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

Amended 2026-09-03: both are implemented now, and the paragraph above is superseded on the point of raising a limit.
The interpreter renews the lease before every network-bearing effect and before every verification try, so the requirement is no longer that a whole run fit inside one lease but that no single uninterrupted stretch between two renews outlive it.
That left one stretch still failing, because the renew sits before the effect rather than inside the adapter: two TPT queue-job polls inside a single `submit` spend 360s with no heartbeat between them.
The founder raised `LEASE_TTL_SECS` from 300 to 600 by decision on 2026-09-03 to close it, choosing one number over a clock port that would have put a timer inside the adapter seam and handed every adapter a way to extend the lease it runs under.
The cost is stated rather than avoided: the reaper takes twice as long to reclaim an item from a device that really stopped, which the heartbeat already distinguishes from one that is merely slow.
The inequality is now asserted as a guarantee rather than as a recorded defect, by `tpts_theoretical_worst_case_submit_fits_inside_the_lease` in `crates/tam-engine/src/seed.rs`.
The decision and its alternative are recorded as question 7 of `docs/notes/design/engine-driver-split.md`, and the work landed as step 11c in that note's section 7.

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

## The TPT download, built from the wire format and gated live, 2026-08-29

G-O3 asked what TPT answers when the seller downloads their own product. The founder's capture missed the hop itself — a `target="_blank"` download needs DevTools "Preserve log" — but it settled everything around it, and an orchestrator probe settled the rest.

The entry point is `GET {ORIGIN}/Download/{canonicalSlug}-{id}`, read from `Product.downloadurl` in the product page's own SSR state.
The control is a plain anchor with no minted token, no nonce and no XHR, so it reproduces as an ordinary session-authenticated document navigation.
The product id is the identifier and the slug is decorative, as it is on `/Product/{slug}-{id}`; there is no asset id anywhere in the capture, and the payload is declared ZIP, which is the shape the importer already names every bundle.

The probe found the route gated.
A live fetch of the founder's own product answered `302` to `/Request-Authorization?authModal=login` for a cookie jar that authenticates the GraphQL reads — with `isProductAuthor: true` on that very product — and the entire Phase 3 write path.
Navigation headers did not change it.
The jar lacks `cf_clearance`, which the browser capture carries, so the document navigation is gated on a Cloudflare browser clearance where the XHR API path is not.

The decision is to build the capability and leave the sync gate closed.
`download_resource_bundle` is implemented from the wire format with a cassette for each answer it can meet, including the sign-in page a 200 can carry, which is why the archive is asserted positively by its own magic rather than inferred from a status.
`uncaptured_source` still names TPT, so `POST /{v}/sync` refuses TPT as a source and the drain never leases a request it cannot serve; TPT-as-source sync ships on the operator-manifest path.
Removing that row is a founder decision, and what would justify it is a live download — from a session carrying the clearance, or a finding that the route is browser-only.

Two things are deliberately not invented.
An off-origin redirect is refused rather than followed: no capture carries a signed hop, and the transport refuses a session request to any host but the origin, which is the rule that keeps the seller's cookies on the marketplace.
And the refusal for the gate is its own condition rather than `SessionExpired`, because the same jar that meets it authenticates everything else, so re-authenticating is the wrong remedy to send an operator after.

## The body carriage: pass-through one way, rendered the other, 2026-08-29

The founder approved `pulldown-cmark` as the workspace's Markdown-to-HTML renderer, which is the one dependency this decision adds.

The two directions are not symmetric, because the two marketplaces are not.
Tes takes either format and posts the matching `descriptionRawType`, which the 2026-08-29 live probe established against a real draft, so a TPT-sourced HTML body crosses to Tes as itself and no HTML-to-Markdown converter is needed anywhere in this system.
TPT stores and returns its description as HTML alone, so a Tes-sourced Markdown body is rendered at `project_fields` before it reaches the wire, and the interim that refused such a body rather than corrupting it is superseded.
The rendering runs on the declared format and never on a reading of the bytes; sniffing is what the declaration exists to prevent.

The extension set is CommonMark plus tables and strikethrough, and deliberately nothing else.
Those two are on because their absence changes a body: a pipe table would reach TPT as literal pipes.
Smart punctuation is off because its presence edits one, rewriting the seller's own quotes and dashes into other characters, which is not a rendering of their copy but a change to it.
Task lists are off because their rendering is an `<input>` element and what TPT's rich-text field does with one is unmeasured.
The renderer is pinned in `[workspace.dependencies]` for the reason `sha2` is, so that one Markdown body renders to one HTML body on every path that renders it.

## The architecture reversal: request origin moves to the seller's device, 2026-09-03

The founder reversed the server-side automation decision recorded under "Architecture" above, and the reversal is two-branch rather than wholesale.
Where a marketplace publishes an official API and issues a token for the purpose, automation stays server-side on infrastructure we operate, using that token; Etsy and Shopify are that branch.
Where no official API exists, every marketplace request originates on the seller's own device under the seller's own session; TeachersPayTeachers and Tes are that branch.
For the second branch the server is a control plane holding the catalogue, the mapping decisions, the ledger, the dashboard, the subscription and the kill switch, and it sends declarative intent describing an outcome; it never composes, signs or issues a request to a no-API marketplace, and never holds a session for one.

The reason is request origin rather than permission.
The Ninth Circuit's Perplexity decision turned on the architectural fact that Perplexity's servers never directly accessed Amazon's, so it is the user who accesses with the help of the tool; our servers indisputably access TPT's and Tes's, which concedes that prong and leaves only the authorization question, lost the day a cease-and-desist arrives.
Moving the request is the only change that alters which question is litigated rather than improving the answer to the existing one, and it was already the top-ranked mitigation in `../notes/legal/marketplace-terms-assessment.md`.
Consent does not cure it, so a seller handing us a session to operate while their device is off is refused: the seller's device is the only thing that opens a connection to a no-API marketplace.
An opt-in cloud mode is reserved as a later decision taken with counsel and is never the default.

The rule is enforced rather than remembered.
The registry records a transport class per marketplace, and a test fails the build if a no-API marketplace gains a server transport.
It is also visible to the seller: a badge on every marketplace row, a connect flow stating where the login happens, publish progress naming the device doing the work, and the wording "your login never leaves your device", which is never presented as a legal requirement because no statute imposes one.

The surface order becomes Windows desktop first, then Android, then iOS, all Tauri v2, with desktop bundles distributed through CrabNebula Cloud.
macOS leaves the first release because the founder has no Mac and Apple's licence forbids macOS on non-Apple hardware; Linux stays best-effort.
The browser extension is deferred rather than sequenced: the founder would take it only as a one-day build, and the one-day version is the server-composes relay this entry rules out.
The web console is retained, publishing directly to API-branch marketplaces and handing the no-API ones to the desktop agent.
The schedule stays deterministic and cron-shaped in both branches; only the location of the timer moves.

Two consequences are recorded here rather than edited into the sections they touch.
The "Credentials" section's central vault, and with it the session broker and its gateway route allow-lists, dissolve for the no-API branch, because there is no longer a credential to hold centrally; the seam that section exists to preserve is satisfied by the device instead of by a better server-side implementation.
Carried out 2026-09-04: `crates/tam-session-broker` and the socket client in `crates/tam-engine/src/broker_client.rs` are deleted, and `POST /{version}/connections/{connection}/revoke` marks the connection row revoked instead of tombstoning a vault. The `tam_broker` role is not dropped by that change: the following migration revokes every privilege it holds, it stays an inert `NOLOGIN` name in dev and CI because three frozen migrations grant to it and a replay cannot name a role that does not exist, and the drop itself is a production operator step in `../notes/runbooks/retire-tam-broker-role.md`. The gate that released it was `a_device_and_a_declaration_are_the_whole_link_a_tpt_write_needs` passing with the vault's connection writer removed, recorded in `../notes/design/engine-driver-split.md`. `connection_secret` is deliberately left standing, holding the only copy of what was sealed before this.
The kill switch and the subscription gate become the entitlement token described in `../notes/design/client-side-architecture.md`, which is the only enforcement available for work running on the seller's machine, and the server remains the authority per check-in rather than per marketplace request.

The full decision set, the evidence, and the re-baselined plan are in `../notes/design/vendoo-for-teachers-rethink.md`, which records thirty decisions D1 to D30 taken across four rounds on 2026-09-02 and 2026-09-03.
This entry is D1.
The build awaits an explicit founder go on that plan.

## The Lucide icon set, approved as a web dependency, 2026-09-05

The console takes the Lucide icon set as a web dependency — `@lucide/svelte`, under the ISC licence — approved by the founder on 2026-09-05.
The alternative rejected was a hand-copied path map carrying no dependency at all.
The reason is that the set is maintained and tree-shaken, and its icon names form a closed type, so a typo fails the web lane instead of rendering an empty box.
Adding a dependency is a founder decision rather than a way to make a build pass, and this entry is that gate passed for this one dependency.

## Marketplace names and logos, shown under a disclaimer, 2026-09-05

The Marketplaces page shows each marketplace's official name and logo to identify it, under a footer disclaimer that the names and logos belong to their owners and that Teachouse is not affiliated with or endorsed by them.
The founder approved this on 2026-09-05 knowing that Etsy and Shopify both state that use of their logo needs written permission, and that eleven of the candidate marketplaces publish no brand page at all.
The alternative rejected was wordmark tiles set in our own typeface.
The founder's stated intent is to approach each marketplace in due course.
The brand survey behind those facts sits in the marketplace catalogue, which is outside this repository; the decision is recorded in `../notes/design/console-redesign-plan.md`.

## Best fit, pre-ticked on each marketplace tab, 2026-09-05

The best-fit control on a marketplace tab of the authoring form is ticked by default, and the seller unticks it to answer that marketplace's fields themselves.
This amends the founder's own 2026-08-29 mapping-equivalence direction, which made best fit an opt-in convenience and explicitly not the default, and it amends the design record's "offer only what the target will accept, and never default" in `../research/rethink/cross-marketplace-mapping-tpt-base.md`.
It is an amendment rather than an oversight: the earlier direction was written before the form existed, and a seller who has selected four marketplaces and must answer every axis of each by hand is being asked to do the work the product exists to remove.

The amendment holds on three conditions, and each is enforced rather than asserted.

The tick never covers a field the registry marks `Delegation::Never`.
`resolution_for` already makes `Never` win over any opt-in, and `best_fit` declines by consulting exactly that function rather than by repeating the rule, so the Tes licence and TPT's copyright declaration stay the seller's whatever the tick says.
`PUT /{version}/elections/delegation` filters those axes out before writing, so ticking a marketplace that carries one succeeds on everything else rather than failing whole, and `ElectionRule::new` and the `election_rule_licence_never_delegated` CHECK both still refuse the row if anything reaches past the filter.

The tick writes a durable, revocable `ElectionRule` with `answer_kind = 'delegate'`, one per delegable axis, rather than setting a preference in a browser.
The delegation is therefore a row the seller can see, an operator can audit, and one call withdraws; the untick deletes exactly the delegations and never the literal answers the seller stated themselves, because withdrawing a permission is not withdrawing a decision.
The opt-in is read back per `(inventory, axis)` rather than per trigger, because the table admits a keyless rule for only the two triggers that generalise to nothing and a marketplace-level tick has no key to give for the other two.

A suggestion resolves only on explicit confirmation.
`resolved_by` returns nothing for a `Delegate` answer, so a delegated question stays a question and is never silently settled by the projection; the pre-selected value is rendered and the seller confirms it.

The ranking is over the resolved set and never over the target's vocabulary, which is what keeps a suggestion from becoming an invention.
Order is the edge kind first, `Exact` before `Broader`, then the seller's own stated order, and the sort is stable so their order survives within a kind.
An over-cap suggestion keeps exactly the cap and names what it dropped; a narrow suggestion keeps every value under the band; a supply suggests nothing, because the source carried no value and there is no set to rank.

The form templates the tick sits inside are a frontend concern and are recorded here only where the backend carries them.
A native field gains the words the platform's own form heads it with and the section that holds it, both from the captures already on file and absent where no capture recorded either, so a form renders "Resource type" where the wire says `mainType` and falls back to the wire name rather than to a reading invented for it.
The template choice itself is derived from the resource's destination set and stored nowhere: it is a fact about how a form was rendered rather than about the product, and re-opening a resource on another template must show the same product.

## The organisation slug, chosen at the first sign-in, 2026-09-05

An organisation carries a slug beside the name it already had: 3 to 32 characters, lowercase letters, digits and single interior hyphens, no hyphen at either end, unique case-insensitively under an index on `lower(slug)`.
The name stays what the seller wants printed; the slug is the unique, URL-safe handle they type and a support email quotes.
Both pay rent, so the slug is added beside the name rather than replacing it, and the slug is URL-safe from the day it exists rather than from the day something embeds it.

The choice happens at a first-run claim screen after the first successful sign-in, not on the signup form.
The reason is mechanical rather than aesthetic.
Registration never leads straight to the console: the identity service sends a verification link and the API refuses an unverified assertion, so the signup form's next screen is "Check your email", and the first session exchange -- the only moment an organisation is created -- happens after an email round trip, on a page load that may be a different browser or a different device.
A slug typed at signup has nowhere to live across that gap: `auth.user` is the identity service's and holding domain data there breaks the boundary this record already draws, `sessionStorage` is defeated by the verification link, and widening the assertion's claim set would make the identity service assert domain data that authorisation is never allowed to read from a token.

Existing organisations get `NULL` and no derived backfill, because a slug derived from the row's UUID is the provisional `org-{uuid}` name wearing a different hat and would burn a handle the seller may want.
A newly provisioned organisation meets the claim screen as a hard gate before the console, since a new seller has nothing to do there before naming their organisation and a dismissable prompt would become a permanent `NULL`.
An organisation that predates the change meets a banner it can dismiss, since it is mid-work and a gate would be a rude surprise for no gain.
Nothing in the row distinguishes the two, so migration 0057 records the distinction at the one instant it is knowable, writing `slug_deferred` true for every row that already existed and false by default for every row since.
The alternative was comparing `created_at` against a cutoff instant, which puts a date literal in whichever layer does the comparing and cannot be checked by a test that does not also control the clock.

A slug is changeable, the old one is not reserved and no history is kept.
Nothing depends on one today: no route embeds a handle, no marketplace listing carries it and no email template references it, so a rename breaks nothing.
The standing cost is recorded rather than discovered later -- the first public URL that embeds the slug converts this into a redirect obligation, and the change to make then is a slug-history table plus a permanent redirect, not a retroactive freeze.
"Changeable by the owner" cannot be expressed today and is deliberately not claimed: there is no ownership or membership concept, so the slug is changeable by any member of the organisation, which in practice is one person because signup provisions one organisation per subject.
Adding invites is the moment the ownership question arrives, rather than a moment at which it is silently answered.

The availability endpoint exists, session-gated and bounded per session, and the 409 on write stays authoritative.
The endpoint is what makes checking as the seller types possible, and the session gate costs nothing because the claim screen is post-sign-in by construction; without a bound it is a directory of every tenant's slug read one guess at a time.
Check-then-write is a race only the unique index settles, so the console renders the 409 even when its own check said the slug was free.
A reserved word is refused 422 and a collision 409, because a policy refusal is not a collision and the console renders them differently.

Shape lives in two places on purpose and policy in one.
The database CHECK restates the shape rule so a malformed slug cannot arrive from a path that is not the API; the reserved list and the refusal of a 32-character hexadecimal string stay in Rust, because the reserved list moves with our own route table and a second copy of it would drift.

One bound in this change lives outside `crates/tam-limits`, and it is recorded here rather than left to be discovered.
The availability endpoint carries a per-session probe budget -- thirty in a rolling minute, held in the API process and keyed by the session's user -- whose constant sits in `crates/tam-api/src/org.rs` beside the route it governs.
It is a limit, and limits are founder-gated wherever they live, so the founder's next limits round should either adopt it into `tam-limits` or remove it; until then it is an unadopted bound rather than an agreed one.
What it buys is that an account-holder cannot walk the endpoint one guess at a time and read out every tenant's slug; what it does not buy is durability, because a restart forgives a budget and a second API process would keep its own.
Nothing in the product's promises rests on it: the endpoint is advisory in both directions, and the unique index remains the only arbiter of who holds a slug.

## A resource may exist with no file; a marketplace listing may not, 2026-09-05

A resource kept on Teachouse alone may carry no file at all.
A file becomes required the moment the resource is pointed at a marketplace, for a draft there and for a live listing alike, so the rule is about the destination rather than about the lifecycle state.

This reverses the earlier decision that a product with no payload cannot be listed anywhere and should therefore be unrepresentable.
That reasoning was sound about marketplaces and wrong about us: a seller works on a resource before deciding where it goes, and refusing to hold that work made Teachouse the one place the resource could not live.
The invariant was doing two jobs -- keeping a marketplace write well-formed, and keeping the catalogue tidy -- and only the first was ever paid for.

So the invariant moves rather than disappearing.
`CanonicalProduct` carries an optional payload set, and where a payload exists it is still non-empty by construction, so nothing acquires a `Vec` with a runtime check.
Migration 0061 relaxes `assert_product_has_payload` under its own name, so both frozen constraint triggers keep their attachments and the function simply stops raising for a product no mapping names; a new deferred constraint trigger on `mapping` closes the other direction, which the two frozen triggers cannot see because they fire on `product` and `product_file`.
The removal side needs no new code: deleting the last payload fires the existing `product_file` trigger, which now raises exactly when a mapping exists.

`POST /{version}/products` refuses a create that names an inventory and carries no payload, `POST /{version}/products/{product}/mappings` refuses a mapping onto a product with none, and `draft_refusals` raises `PayloadMissing` only where a marketplace is targeted.
The destination reaches the model as a flag on the draft, defaulting to absent, which is what lets a saved template -- a partial draft nobody has chosen a destination for -- round-trip without being held to a marketplace's rules.
The wasm core, `POST /{version}/authoring/check` and the create path stay one function, so the sentence a seller reads as they type and the answer the server gives remain one answer.

What is deliberately not claimed: this says nothing about whether a payload-less resource may be published, because it may not, and nothing about deleting the last file from a resource that already reaches a marketplace, which stays refused by the mapping rule above.

The migration is not reversible past this point.
Relaxing the rule reverses cleanly; the payload-less products created afterwards do not, so a rollback has to deal with those rows first.
That is a runbook obligation rather than part of this decision, and it is named here so it is not discovered from a failed migration.

One consequence is carried forward rather than solved.
`partition_files` used to raise `CorruptRow` for a product with no live payload row, which was always right under the old invariant; it now returns an absent payload for that condition unconditionally, and cannot tell a resource legitimately kept here from a mapped product that has somehow lost its file.
`ProductRepo::get` does not read `mapping` in the same call, so the distinction needs one more statement in that transaction to recover.
It is deliberately not added here for two reasons: the state is believed unreachable, since both deferred triggers re-query `product_file` at commit and a single transaction that inserts a mapping and deletes the last payload is visible to both, so at least one raises; and the reachability argument is reasoned rather than demonstrated, because the existing race test covers two concurrent transactions rather than one transaction doing both writes.
What would close it is a repository-level test of that interleaving and the mapping count threaded into the read, and neither is large.
Until then a corrupted product reads back as a resource with no file, which is the wrong sentence for a state that should be impossible.
