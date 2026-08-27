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
Publish is `POST /api/v2/resources/{id}/publish` and requires a valid `licence` set on the draft first; free resources use a Creative Commons licence (`CC-BY`, `CC-BY-SA`, `CC-BY-ND`), while `TES-PAID` requires a price.
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
