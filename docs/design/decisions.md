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
