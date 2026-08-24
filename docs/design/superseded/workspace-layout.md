# Workspace and crate layout

Superseded on 2026-08-25 by [`2026-08-25-listing-sync-design.md`](2026-08-25-listing-sync-design.md), whose crate table and unit-to-binary mapping are authoritative.
This document is retained for its reasoning and is not live; do not implement from it.

Take the topology from a real production axum service rather than from a starter template.
The reference the research settled on is `rust-lang/crates.io`, which runs a production axum service as roughly 29 library crates under `crates/*` plus a binary and an admin CLI.
The single-crate reference repository the founder cited does not survive contact with this product, because the web client and the workers must share wire types that cannot carry a database dependency, and one crate cannot express that.

The organising question for every boundary is what changes together and what has to be testable on its own.
A crate earns its existence when it either breaks on someone else's schedule, needs a test environment the rest of the workspace does not, or holds a privilege the rest of the workspace must not.
Everything else is a module.

## The crates

| Crate | Runs where | Owns |
|---|---|---|
| `tam-types` | shared, and generated into TypeScript | pure serde ADTs for the wire and the domain vocabulary; the closed failure-code enumeration |
| `tam-domain` | server | projection, per-field normalisers, diff classification, invariants, smart constructors, state machines |
| `tam-taxonomy` | server | canonical terms, vocabularies, projection edges, reconciliation |
| `tam-storage` | server | sqlx repositories, migrations, checked-in offline query metadata, organisation-first signatures |
| `tam-marketplace` | server | the adapter seam, the error taxonomy including the ambiguous variant, receipts and fetch reasons, the cassette harness |
| `tam-marketplace-tes` | server | the Tes adapter, covering both the GB and US inventories |
| `tam-marketplace-etsy` | server | the Etsy Open API v3 client; no browser anywhere in its tree |
| `tam-marketplace-tpt` | server | the TPT adapter, gated on written permission and possibly never written |
| `tam-browser` | automation worker | the WebDriver and BiDi driver, per-call deadlines, per-session unit lifecycle |
| `tam-secrets` | session broker only | the key-encryption key, envelope encryption, data-encryption-key wrap and unwrap |
| `tam-pipeline` | pipeline worker | ZIP, PDF and PPTX inspection, cover and preview generation, malware scanning, object storage |
| `tam-ai` | server | provider abstraction, per-inventory house-style prompts, per-tenant token accounting, output validation back through `tam-domain` |
| `tam-api` | server, as a library | the axum router, extractors, middleware, error mapping, versioned route modules, the progress stream |
| `tam-server`, `tam-worker`, `tam-session-broker`, `tam-admin` | server | thin binaries holding argument parsing, configuration and wiring, and nothing else |

## Why each boundary is where it is

`tam-types` is separate because of what it must not depend on rather than what it does.
It is compiled to TypeScript for the web client and consumed by every other crate, so a database or HTTP dependency reaching it would propagate into the generated type surface and into every build that touches it.
Dependency removal is far more expensive than dependency addition, which is why this boundary is the first one drawn.

`tam-domain` is separate because it has no I/O and therefore runs property tests in milliseconds.
Projection, normalisation and diff classification are exactly the code that most needs exhaustive testing and least needs a database, and putting them behind a boundary that bans tokio, reqwest and sqlx is what keeps that true as an agent adds features.
The ban is asserted mechanically against the dependency graph rather than left as a convention, because a convention about dependencies is one `cargo add` away from being false.

`tam-marketplace` holds the seam, and the seam is defined without an async runtime dependency.
Writing `async fn` in a public trait fires `async_fn_in_trait` under a deny-warnings build on the pinned toolchain, and the compiler's own suggested desugaring is also what a multi-threaded runtime requires, so the trait is written explicitly:

```rust
pub trait MarketplaceAdapter {
    fn publish(
        &self,
        intent: &WriteIntent,
    ) -> impl Future<Output = Result<WriteOutcome, AdapterError>> + Send;
}
```

That form compiles clean with `-D warnings` and no dependencies at all on `rustc 1.97.1`, verified rather than assumed.

`tam-marketplace` is also where `WriteReceipt` and `FetchReason` live, and that placement was forced by compiling the domain sketch rather than chosen for tidiness.
The read hierarchy is enforced by giving `WriteReceipt` private fields and a constructor callable only from the write path, and Rust's finest visibility control is crate-scoped, so the capability is airtight only when the constructor and its sole caller share a crate.
Putting those two types in `tam-types` would have left the constructor public and the control decorative.

One crate per marketplace, not enum variants in one adapter.
Each breaks on someone else's release schedule, each needs its own fixture corpus, and each must be feature-gated off during an outage without disturbing the others.
Tes GB and Tes US are deliberately one crate and not two, because they share markup, a login and an upload flow, and differ only in vocabulary and currency, which are data.
That is the same test applied in the other direction: they change together.

`tam-browser` is separate because it is the only crate whose test closure needs a real Chromium and the only one carrying a forced upgrade cadence, roughly fifteen to twenty-five browser bumps a year on the security channel, each an unreviewed change to how someone else's form renders.
It is also the crate most likely to be deleted outright: if the M-1 probe finds the Tes upload is a plain multipart form POST, the browser fleet, the per-session isolation, the shared-memory work and most of the maintenance line all evaporate, and a plain HTTP client takes its place.
A boundary that lets a whole subsystem be deleted is worth having on the first commit even if it is thin.

`tam-secrets` is separate because it is a privilege boundary, and a privilege boundary is a process, not a module.
Only the session broker links it, the broker runs as its own user with the key-encryption key delivered as a credential, and its entire external surface is one call that leases an already-authenticated browser endpoint rather than returning session material.
A compromised automation worker can then misuse the connections it leased and cannot exfiltrate the vault.

`tam-api` is a library rather than a binary so that integration tests drive the router in-process with no bound port and no race for one.
This is nearly free to do now and annoying to retrofit once test helpers have grown around a running server.

`tam-pipeline` is separate because it carries the heaviest native dependencies and runs on its own host, and because its worst inputs are hostile: a decompression bomb, a symlink-traversal archive, a malformed document.
Isolating it means the API's build closure never contains an office suite and the ingestion process can be reaped by the operating system under memory, CPU and wall-clock limits, which discharges the bomb and ratio limits at the layer where they actually hold.

## What must exist on the first commit, and what can wait

Five boundaries are expensive to retrofit and are drawn immediately.

`tam-types` is first, for the dependency-removal reason above.
`tam-marketplace` with the three-valued outcome is second: adding a third result to a call site that assumed two touches every caller, every stored state and every test, and doing it after listings exist means backfilling a state that was never recorded.
The organisation-first repository signature in `tam-storage` is third, and it is the single most expensive item in the whole design to add later, because it changes every signature, every call site and every fixture at once.
The broker as a separate process is fourth, because a privilege boundary cannot be introduced without redesigning every call site that currently holds plaintext session material.
`tam-api` as a library is fifth and is nearly free.

Everything else can be merged for the first few months and split when it hurts.
`tam-domain`, `tam-taxonomy`, `tam-storage` and the job ledger can start as one `tam-core` crate, which is what the research already recommended; the boundaries between them are cheap to draw later because they are internal and no external consumer depends on their shape.
`tam-ai` can start as a module inside `tam-server`, since the first listing-copy work is one extraction call and a few rendering calls.
`tam-pipeline` can start as a module and move out when the office suite and the malware scanner arrive, which is the point at which the closure becomes the problem.
`tam-browser` can start inside `tam-marketplace-tes` and move out when a second browser-driven adapter exists, which may be never.
`tam-marketplace-tpt` does not exist until a written reply arrives.

## Four patterns worth taking from the reference repository

The founder's reference repository is `sheroz/axum-rest-api-sample`, MIT, v0.1.12, and the research named four things in it worth adopting close to verbatim.

The structured API error type with its builder, and specifically its split between what a debug build discloses and what a release build discloses, which is the right shape for an error surface that must be useful to the founder and uninformative to an attacker.
The API-version extractor implemented as a request-parts extractor, which puts versioning in the type system rather than in string handling inside each handler.
The refresh-token rotation scheme in which a refresh token references its paired access token, so a single logout revokes both rather than leaving a live access token behind a revoked refresh token.
And the three-tier revocation model, which is the one item that needs a substitution rather than a copy: the reference implements it against Redis, and the stack chosen here has no Redis anywhere, because the research rejected every Redis-backed Rust rate-limiting crate on maintenance grounds and the deployment is a single self-hosted machine.
What transfers is the tiering, implemented against PostgreSQL, and that substitution is a judgement made here rather than a finding carried from the research.

## Five things in it that must not be copied

Each of these is a verified defect in the reference repository and each has a direct analogue this product could reproduce by accident.

The login handler compares a client-supplied `password_hash` field against a stored single-round SHA-256, with no argon2, bcrypt or scrypt anywhere in the tree, which means the stored hash is the password.
`.env` files containing the JWT secret and the database password are committed to git.
Raw tokens and full claims are logged at info level.
The user struct carrying the password hash and salt is serialised directly to API clients.
And there is no tenancy anywhere in the model, which for a product sold to schools and agencies is the single most expensive thing to retrofit and is the reason the organisation identifier appears in every repository signature in this design.

The last two are the ones an agent is most likely to reproduce, because both come from taking the obvious shortcut: serialising the database row straight to the client, and reading the tenant from wherever it happens to be available.
Both are addressed structurally rather than by review, the first by keeping wire types in `tam-types` and never deriving them on storage rows, and the second by the positional parameter.
