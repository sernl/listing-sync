# Engineering charter

## 1. What this charter is for

This charter decides where this project spends rigour, and converts as much of that decision as possible into something the compiler, the linter, or a flake check enforces rather than something a person remembers.
Its governing idea is that rigour is a budget denominated in two scarce currencies — the founder's review attention, and the cost of un-shipping a mistake — and it must be concentrated where those two coincide: the database schema, tenant isolation, the sync state machine's durability and idempotency semantics, money, and the public API contract.
Everywhere else, and particularly in the per-marketplace HTML heuristics that will break and be rewritten regardless, the correct amount of ceremony is close to none.

Two sources shape the specifics: TigerBeetle's TIGER_STYLE.md and Gerard Holzmann's "The Power of Ten – Rules for Developing Safety Critical Code" (NASA/JPL, IEEE Computer, June 2006; manuscript at spinroot.com/gerard/pdf/P10.pdf).
Both were written for contexts unlike this one — a single-purpose Zig database with all memory allocated at startup in the first case, statically-allocated embedded C with no operating system in the second — so this document is as much a record of what was rejected as of what was adopted, and section 4 is the load-bearing half.
Holzmann's own meta-criterion governs the whole thing: a rule that cannot be mechanically checked is a weak rule, which is why this charter's primary artefacts are a `[workspace.lints]` table, a `clippy.toml`, a `crates/limits`, and a set of flake checks, with prose reserved for the rules that genuinely cannot be automated.

One standing constraint bounds every recommendation below, and it is a requirement in the core rather than a preference.
Rust is required for the engine, all I/O, batch processing, the automation layer, and anything computationally heavy.
That is a founder decision recorded in the decision register, not an engineering taste, and it is not reopened here.
TypeScript is for the user interface, and for one bounded identity service.
That service is better-auth, running as `tam-auth`, and it owns platform-user identity and browser session only: registration, sign-in, social and passkey credentials, email verification, password reset, and the keys for the tokens it issues.
It owns no domain data, performs no marketplace request, and holds no marketplace credential.
It reaches Postgres only as the `tam_auth` role, whose grants are confined to the `auth` schema; it never reads or writes a table in `public`, never sets `app.current_org`, and never contacts the session broker.
Authorisation — which organisation a request speaks for and what it may do there — is decided in Rust from Postgres, and is never asserted by a token claim.
Any extension of this service beyond identity and session is a new founder decision, not an application of this one.
This is why the web client is Svelte and SvelteKit and why the Android client carries no Rust at all.
So the preference is at the edges and the requirement is in the core: nothing below should be read as licence to move a core component out of Rust, and nothing below should be read as an instruction to write an interface in Rust.

The charter is three files, because one file carrying all of it would exceed the length at which any part of it stays reviewable.
This file, called the rules file below, carries sections 1 through 8: what the charter is for, the rules adopted, modified and rejected, the assertion discipline, the limits crate, and the testing pyramid.
[`operational-charter.md`](operational-charter.md), called the operations file below, carries sections 9 through 18: secrets, backup and restore, migrations, observability, the kill switch, inbound abuse, prompt injection, deployment, data protection, and what the whole thing costs.
[`enforcement-toolchain.md`](enforcement-toolchain.md), called the toolchain file below, carries the primary artefacts themselves: the lint table, the `clippy.toml`, the `deny.toml`, the release profile, the limits module and the flake checks.
Section numbering runs continuously across the first two.

### The gate that precedes every rule below

Nothing in this document is worth reading until one question has a written answer, and the settled architecture changed which question it is.
Automation runs on infrastructure this project operates, against seller accounts, using credentials sellers supply, so the vendor rather than the seller is the acting party under every marketplace's terms.
That was a founder decision taken against the round-one research recommendation, which favoured local-first execution on account-safety and credential-custody grounds, and it was taken with those risks stated.
It is recorded as deliberate in the decision register rather than inherited from a draft, and the earlier local-first posture is superseded rather than merely disfavoured.

The gate is therefore what each marketplace's terms permit a vendor to do on a seller's behalf, which is a harder question than the one an earlier draft asked and is the only question that can invalidate the product rather than the design.
An engineering charter for a product whose premise may be unlawful has its dependencies inverted, and the cost of discovering the answer late is not a refactor but a shutdown.

Two things follow, and each is a gate rather than a task.
Read each marketplace's terms and record, in the repository, the specific clauses that bear on automated interaction and on bulk listing creation, with the retrieval date, because terms change and an undated reading is worthless.
Where the terms are ambiguous, get the ambiguity in front of a solicitor before the production build starts, not before launch.
Until both are done, `limits::marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX` stays at its deliberately conservative starting value, and section 6 makes raising it require this answer by name.

The wedge is chosen so the first chargeable product does not wait on the hardest of those answers.
Tes GB-to-US inventory duplication is a bulk-tool job entirely inside one marketplace, needing no cross-marketplace mapping and no third-party exposure.
Etsy is the second connector and is the one case where a sanctioned write API and published API terms grant the licence outright.
TeachersPayTeachers is a gated, optional third connector, and every screen must be complete and worth paying for without it.
The order in which the answers arrive therefore shapes the roadmap rather than the engineering, which is the point of choosing the wedge that way.

## 2. Rules adopted directly

These transfer with little or no reinterpretation.
Every lint and configuration key named below was verified present on rustc 1.97.1, cargo 1.97.0 and clippy 0.1.97; version numbers are as observed on 2026-08-24.
Upstream citations give the section and, where a verbatim quotation was confirmed, the quoted phrase; line numbers are deliberately absent, because a clone at a later commit already showed them drifting.

| Rule | Source | Rust form | Machine enforcement |
|---|---|---|---|
| All code compiles warning-free at the strictest setting, from day one | Power of 10 Rule 10, "from the first day of development"; TIGER_STYLE §Safety | One `[workspace.lints]` table at the workspace root, every member carrying `[lints] workspace = true` | `craneLib.cargoClippy` with `--all-targets -- --deny warnings` as a flake check |
| Assertions detect programmer errors; operating errors are handled | TIGER_STYLE §Safety, "Assertions detect programmer errors" | `assert!` for bugs, `Result` for the world, partitioning the space rather than competing (section 5) | `clippy::unwrap_used` and `clippy::expect_used` denied in library crates, forcing an explicit choice at every fallible site |
| Split compound assertions; assert implications with `if` | TIGER_STYLE §Safety | `assert!(a); assert!(b);` rather than `assert!(a && b)`, preferring `assert_eq!`, which prints both operands | `clippy::missing_assert_message` denied; a grep rejecting `&&` inside `assert!(` is a zero-false-positive five-line check |
| Assert relationships between compile-time constants | TIGER_STYLE §Safety | `const { assert!(...) }` blocks (stable since 1.79) over the limits table and the tier quotas | Build failure, the strongest tier available |
| Pair assertions across a lossy boundary | TIGER_STYLE §Safety | Assert before serialising and again after deserialising; assert the payload before the adapter fills the form and again against the scraped-back confirmation | Every type crossing the trust membrane declares `#[serde(try_from = "RawX")]`; the two-tenant integration test is the check that a bypass shows up as a value that should have been rejected |
| A limit on function length | TIGER_STYLE §Safety (70); Power of 10 Rule 4 (~60) | 70 lines, taking TIGER_STYLE's figure over Holzmann's to absorb Rust's `match` arms and `where` clauses | Advisory rather than gated: `too_many_lines` is raised in the second clippy lane at `too-many-lines-threshold = 70`, for the reason recorded in the toolchain file |
| Put a limit on everything | TIGER_STYLE §Safety; Power of 10 Rule 2 | Every shared-resource bound is a named constant in one crate (section 6), and nothing else declares one | `clippy::disallowed_methods` banning `tokio::sync::mpsc::unbounded_channel` and `reqwest::Client::new`; `clippy::infinite_loop` denied |
| Centralize control flow; keep leaf functions pure | TIGER_STYLE §Safety | The orchestrator owns every branch and calls helpers that compute a `Decision` enum rather than helpers that act | Crate boundaries: the pure decision crate does not depend on tokio, reqwest or sqlx, so cargo checks it |
| Run at your own pace, not the external party's | TIGER_STYLE §Safety | The Stripe webhook handler validates, enqueues and returns 200; upload handlers store and enqueue; adapters are driven by a rate-limited lease from the job ledger | The axum crate must not depend on the browser driver or the LLM client, pinned by a `cargo-deny` `[bans]` entry giving each of those one permitted parent |
| Pass options explicitly rather than relying on defaults | TIGER_STYLE §Safety | `reqwest` has no default request or connect timeout, so every client is built through the builder and every call is additionally wrapped in `tokio::time::timeout` | `disallowed-methods` entries for `reqwest::Client::new` and `reqwest::get`, each carrying a message pointing at the builder |
| Declare variables at the smallest possible scope | TIGER_STYLE §Safety; Power of 10 Rule 6 | Mostly free in Rust; the residue is module visibility and axum state, split per-router via `FromRef` rather than one god `AppState` | rustc `unreachable_pub` denied is the precise mechanical form, with `clippy::significant_drop_tightening`, `await_holding_lock` and `await_holding_refcell_ref` denied |
| Distinct types for index, count, and size | TIGER_STYLE §Off-By-One | Newtypes: `PageIndex(u32)`, `PageCount(u32)`, `AttemptCount(u32)`, `PriceMinor(i64)`, `TenantId` | `clippy::as_conversions` denied in domain crates forces `TryFrom` with a handled failure, and the newtypes are themselves compile-checked |
| Restrict pointer and unsafe use | Power of 10 Rule 9, read as a goal rather than as text | `#![forbid(unsafe_code)]` at every crate root, forbid rather than deny, so an agent cannot locally override it at 2am | rustc lint; the single highest-confidence line in this charter |
| Check every return value | Power of 10 Rule 7 | `Result` and `Option` are already `#[must_use]`, so close the specific holes rather than casting a wide net | `unused_must_use`, `clippy::let_underscore_must_use`, `clippy::let_underscore_future` and rustc `let_underscore_drop` all denied |
| Every error path must be tested, not merely present | TIGER_STYLE §Safety, citing the OSDI '14 finding that 92% of catastrophic failures came from incorrect handling of explicitly signalled non-fatal errors | Every `Err` variant the adapters, job ledger and billing reconciliation can produce has a test that constructs it and exercises the handler | Error enums stay small and per-module, so an exhaustive `match` in the test module fails to compile when a variant is added untested |
| Line length and formatting are settled by the formatter | TIGER_STYLE §Style By The Numbers | rustfmt defaults already give 4-space indent and `max_width = 100`, so both numeric style rules come free | `craneLib.cargoFmt`, wired through treefmt-nix so nix, TOML and markdown share the gate |
| A small standardized toolbox | TIGER_STYLE §Tooling | `cargo xtask` rather than `scripts/*.sh`; one flake, one formatter config, one lint table, one test runner | If it is not in the flake devShell it is not in the toolbox, and `nix flake check` gates it |

Two notes on attribution.
TIGER_STYLE.md contains no testing section — testing appears in four scattered sentences and the simulator is named once, unexplained — so nothing in this charter should be cited as "TigerBeetle's testing discipline" from that document.
TigerBeetle's practice of shipping assertions enabled in production is evidenced in its repository (`build.zig` sets `preferred_optimize_mode = .ReleaseSafe`; `docs/internals/vopr.md` states it keeps assertions on in production) and is not a claim TIGER_STYLE.md itself makes.

## 3. Rules adopted in modified form

### Panic policy is per tier, not global

TIGER_STYLE says the only correct way to handle corrupt code is to crash, calling that a downgrade of a catastrophic correctness bug into a liveness bug.
That is a downgrade only because a TigerBeetle cluster has replicas to absorb it; on one self-hosted box with no failover an unconditional crash is a total multi-tenant outage triggered by one seller's malformed PPTX, so the premise is absent and the conclusion does not carry.

The default posture is nonetheless worse than either extreme, which is the finding that actually drives this rule.
All three cases were measured: a panic in an axum 0.8 handler with no catch layer closes the TCP connection with zero bytes returned, so the client sees a dropped connection rather than a 500 while the process serves the next request; a panic inside a detached `tokio::spawn` is swallowed with only a line on stderr; and `panic = "abort"` aborts the whole process on the first task panic and thereby disables `CatchPanicLayer` entirely.
Independently verified: `tokio::task::JoinHandle` is not `#[must_use]` in tokio 1.53.1, only `AbortHandle` is, and dropping the handle detaches the task and discards its `Result` including a panic captured as `JoinError`, so a sync job can panic and vanish, presenting as a job that hangs forever.

The policy is therefore scoped by blast radius and keeps `panic = "unwind"`, as follows.
In the web tier, `tower_http::catch_panic::CatchPanicLayer` converts the dropped connection into a clean 500, and its catch handler must log at error level with the request and correlation identifiers and increment a dedicated counter that alerts on any non-zero rate, because a silently swallowed panic is strictly worse than no assertion.
Use `tokio::sync::Mutex` or `parking_lot` rather than `std::sync::Mutex`, whose poisoning turns one unwound request into a cascading outage.
In the worker tier, every spawn goes through a `spawn_supervised` helper that joins the handle and treats `JoinError::is_panic()` as an event, and a panicking job routes to a terminal `needs_human` state and is never retried, because an assertion failure is deterministic and retrying it is a hot loop burning the tenant's quota and the LLM budget.
At startup and in configuration TIGER_STYLE applies literally: assert configuration invariants at boot and let the process die before it binds the listener.
If a second machine ever appears this calculus shifts materially toward TigerBeetle's original posture, and this section should be rewritten rather than quietly inherited.

### Assertions ship in release, because they cost nothing here

Rust inverts Zig's default: `debug-assertions` and `overflow-checks` are both off in the release profile, so arithmetic that panics in development silently wraps in production.
The consequence was confirmed with runtime operands, which is the only way the experiment can be run: an addition that overflows `u8` panics under the dev profile, yields `4` under `-O`, and panics again under `-O -C overflow-checks=on`.
The literal form `250u8 + 10` cannot demonstrate this, because constant-evaluated overflow is caught by the deny-by-default `arithmetic_overflow` lint and fails to compile in both profiles; an earlier draft cited that form and so described an experiment that cannot have been run as written.
For quota counters, retry budgets, price arithmetic and pagination offsets a silent wrap is exactly the catastrophic-correctness-bug class both sources exist to prevent, and the performance argument that constrains TigerBeetle does not exist here, since this system's data plane is a browser session filling a form and an LLM completion taking seconds.
The restored `[profile.release]` block lives in [`enforcement-toolchain.md`](enforcement-toolchain.md).

### The client is thin, and publishing is a server operation

The client targets the web first and Android second, and neither performs automation of any kind.
Publishing is a request the client makes and the server performs, so the client's job around sync is detailed progress reporting: files in sync, completed, failed, per marketplace.
Mobile is therefore a full client rather than a read-only one, and that is a consequence of the architecture rather than a product concession.

An earlier draft made the capability to publish a value only a desktop build could construct, so that a mobile build attempting to publish was a compile error rather than a runtime rejection.
That type is deleted rather than relocated, because it encodes a fact that is no longer true and would make the Android client structurally unable to do the one thing every client does.
What survives is the reason it existed, which was to put a capability difference in a type rather than in a convention, and there is a real capability difference left to model.
It is not device-shaped: it is that a tenant either holds a linked, unexpired marketplace connection or does not, so the publish request type takes a value only a live connection can produce and a tenant without one cannot construct a publish at all.
The asymmetry moves from the device to the connection, which is where it actually lives.

Two smaller consequences follow and are worth taking.
No client carries Rust, so no crate in the workspace is a candidate for `deny` rather than `forbid` on `unsafe_code`, which removes Miri from the deferred list along with the trigger that would have reintroduced it.
And the client feature matrix collapses to one row, because the web and Android clients are the same Svelte codebase over the same REST contract.

### The automation plane is server-side, and its crate is chosen for bounded commands

Automation runs on infrastructure this project operates: the founder's own NixOS machine on a home connection initially, and a dedicated NixOS box with production-grade specifications later.
That hosting choice is load-bearing rather than a cost preference.
A probe on 2026-08-25 found TeachersPayTeachers returns HTTP 200 to a plain request from the founder's consumer-ISP host, while earlier research measured HTTP 403 from datacentre infrastructure, so a move into a commercial datacentre re-opens a reachability question that a move between home connections does not.
Egress is fixed and declared in either case, and if the eventual production box sits in a datacentre that reachability must be re-probed before it is relied on.

The browser is driven by `thirtyfour` 0.37.5 over WebDriver Classic against a nixpkgs `chromedriver`, and the deciding property is a timeout property rather than an ergonomic one.
WebDriver Classic is request-response over HTTP, so every command is bounded at the client by the HTTP client's own timeout, whereas the Chrome DevTools Protocol is a persistent socket on which a command that never receives a response simply waits, which is exactly how an unattended fleet wedges.
Building `chromedriver` from the chromium source tree in nixpkgs makes version skew between driver and browser impossible, at the recurring cost of roughly fifteen to twenty-five forced browser upgrades a year, each an unreviewed change to how someone else's form renders and each requiring re-verification against the marketplace before it reaches production.

Three defects in that crate are load-bearing, are recorded with their source locations in `docs/research/server-side-architecture.md`, and must be wrapped before any outcome logic is written rather than discovered afterwards.
Its WebDriver BiDi commands carry no deadline at all, so every BiDi call is wrapped in `tokio::time::timeout` at the call site, and the wrapper has to account for a caller timeout leaking the pending entry.
Its BiDi event fan-out is a broadcast channel whose stream adapters discard a lagged receiver silently, so under load the completion event for a submit can be dropped with no error and the write scores ambiguous, which is an argument for keeping concurrency low rather than for enlarging the buffer.
And its conversion from the HTTP client's error type flattens that error into a string, erasing the distinction between a timeout and a connection failure, which is precisely the distinction that decides whether a write is ambiguous or safe to retry.

One consequence for the fault taxonomy is worth stating, because an earlier draft carried its opposite.
There is one browser engine rather than three, so `AdapterError::SelectorMissing` is a marketplace fact rather than possibly an engine fact, and the adapter-attempt metric in section 12 of the operations file is readable as a statement about the marketplace.
The end-to-end tier runs against this one automation plane, and there is no per-platform matrix to run it over.

### Sync is scheduled, not agentic

"Sync" in this product means deterministic scheduled execution: a cron-shaped trigger, a queue that runs at a time the seller chose, files reconciled the way a dotfile sync reconciles files, and it does not mean an agent deciding what to do.
LLMs appear in exactly two places — generating listing copy, and proposing a new selector when an adapter drifts — and in neither does a model decide what to sync, in what order, or whether a sync succeeded.
Both sit outside the state machine and behind the adapter membrane, and both produce values crossing a trust boundary, which therefore take the `Err` path plus a human gate rather than an assertion.

This is not a stylistic preference and it strengthens the sans-IO seam argument considerably, so it is stated here rather than leaving the seam justified only by testability.
A pure `fn step(&mut self, cmd: SyncCommand) -> SyncEffects` is only worth building if the thing it models is actually deterministic.
Put a model in the decision path and the seam buys nothing: the same seed produces a different trace on the next run, the committed failing seeds stop replaying, the checker set stops being an oracle because "did this sync succeed" becomes a judgement rather than a predicate, and the determinism bans on `Instant::now`, `Uuid::new_v4` and `HashMap` iteration order become theatre beside a far larger nondeterminism source they do not cover.
Because the engine is ordinary code on a schedule all of that holds: the schedule is a clock input, the tick arrives as a `TimerFired` command, time enters only as data, and the seeded replay is exact rather than approximate.
Server-side execution strengthens this rather than weakening it, because the schedule is now ours end to end and nothing runs on a machine this project does not operate, so the only nondeterminism left in the system is the marketplace itself and it sits behind the adapter membrane where the seam already puts it.

The rule that follows is the one an agent is most likely to erode, so it is stated as a rule: no model output may reach `SyncMachine::step` as anything other than data that has already crossed a smart constructor.
A model may propose a selector, but only a human or a deterministic validator may promote that proposal into the adapter; a model may draft listing copy, but only an approved-content type may reach a publish effect.

### Recursion is bounded, not banned

Power of 10 Rule 1 bans direct and indirect recursion, but its stated justification is instrumental: without recursion "we are guaranteed to have an acyclic function call graph... and can directly help to prove that all executions that should be bounded are in fact bounded."
Where boundedness is established another way the goal is met, and a blanket ban would push agents toward hand-rolled explicit stacks in cases where the recursive version is shorter and more obviously correct.
The operative rule is that no recursion may have a depth determined by untrusted input, because stack overflow in Rust aborts the process uncatchably, bypassing every panic-containment layer above, so one crafted upload becomes everyone's outage.
Concretely: nested archive extraction does not recurse at all, extracting at depth 1 and treating an inner archive as an opaque asset; OOXML uses a pull-based XML reader with an explicit element-depth counter rather than a DOM parser; JSON relies on serde_json's existing limit, verified as `remaining_depth: 128` in `src/de.rs` of serde_json 1.0.151, with the `unbounded_depth` feature that would make `Deserializer::disable_recursion_limit` exist at all denied in `deny.toml` and a `disallowed-methods` entry standing behind that denial; and the marketplace taxonomy tree, whose `SellerTaxonomyNode` nests through a self-referential `children` array in Etsy's own schema, is walked with an explicit worklist rather than by recursion, because its depth is the marketplace's to choose and not this project's.
An earlier draft named the node-graph mapping canvas here as the user-authored cyclic case, and that case no longer exists: the canvas is deferred in the decision register in favour of a virtualised product-by-marketplace table, which is flat, so the cycle-detection obligation is recorded as removed rather than left standing against a component nobody is building.
Where depth is bounded by data the project itself validated at load time, such as the taxonomy crosswalk, recursion is fine and the bound travels in the signature as a `Depth(u8)` parameter decremented on descent.

### Explicitly-sized types at boundaries, `usize` in memory

TIGER_STYLE's "use explicitly-sized types for everything, avoid architecture-specific `usize`" is written for a database with on-disk formats that must be identical across architectures, and applied literally in Rust it fights the language, because `Vec::len` and every index are `usize` and the resulting cast at every boundary multiplies exactly the silent-truncation sites the rule was meant to prevent.
Split it by location: explicitly-sized types for anything crossing into Postgres, JSON, the public REST API, or the automation worker's process boundary; `usize` idiomatically for in-memory indexing; newtypes over explicitly-sized types for domain quantities, which is strictly stronger than the original since it prevents semantic as well as width confusion.
`clippy::as_conversions` denied in the domain crates makes each width conversion a handled `TryFrom`.

### Trait objects are permitted where the implementor set is registered

Power of 10 Rule 9 says function pointers are not permitted, but its own rationale downgrades this to "should only be used if there is a strong justification for their use, and ideally alternate means are provided to assist tool-based checkers determine flow of control and function call hierarchies."
`Box<dyn MarketplaceAdapter>` is the correct design for an open set of marketplaces and is how the fake adapter gets injected, which the acceptance-test outer loop structurally requires, so satisfy Holzmann's escape conditions explicitly rather than obeying the rule text: dynamic dispatch is permitted where the implementor set is closed, enumerable, and registered in one `registry.rs`, which is literally the alternate means the paper asks for and gives an agent one file to read to find every adapter.
What is actually banned is the genuine hazard — `Box<dyn Fn>` callbacks stored in struct fields as a control-flow mechanism, and `std::any::Any` with downcasting — because a named trait method has findable implementors and an anonymous closure in a field does not.
Where the set is genuinely closed and small prefer enum dispatch, so `clippy::wildcard_enum_match_arm` denied makes adding a marketplace a compile error at every site that must handle it.

### Conditional compilation, not macros, is the preprocessor hazard

Rule 8's macro restrictions do not transfer: `macro_rules!` is hygienic and token-tree based, macros already expand to complete syntactic units, and variadic macros are unavoidable.
The conditional-compilation half transfers at full strength and arguably stronger than in C, because Cargo features are additive and unified across the whole dependency graph.

The size of that hazard has now shrunk twice, and both reductions are worth recording rather than silently inheriting a smaller number.
An earlier draft was written against Leptos, whose ssr, hydrate and csr split was the single largest source of interleaved `cfg` in the workspace and the reason a wasm-bindgen crate existed at all; with React there is no wasm-bindgen crate in the Rust tree, no hydrate crate, and no three-way rendering feature matrix, which removes most of the surface this rule was written to guard and materially changes the forbid-versus-deny argument in section 2.
The second reduction is the settled architecture: with no desktop client there is no platform split across Windows, macOS and Linux, so the shipped configuration is one Linux target and the matrix this rule guards is a single column.
Ban `#[cfg(...)]` inside function bodies and permit it only at item level, and name the shipped configuration as an explicit flake check target rather than pretending to test the powerset.
This is specifically an agent-driven-development concern, because an agent reading a file with interleaved `#[cfg(target_os = "...")]` blocks must mentally simulate the preprocessor to know which lines compile, and it will get that wrong.
The true Rust analogue of Holzmann's hazard is a first-party procedural macro, a build-time code generator whose output the agent cannot see, so prefer well-known derives and, if one is unavoidable, isolate it in its own crate with `insta` snapshots of its expansion.

### Zero technical debt applies to the core, not the periphery

TigerBeetle's zero-debt policy assumes a funded team, a decade horizon, and a product whose correctness is the value proposition, and a solo founder applying it uniformly will not ship.
Split the codebase by repayment cost: strict zero-debt for the database schema and migrations, tenant isolation, the job ledger's durability and idempotency semantics, billing and Stripe reconciliation, and the versioned public API contract; freely accepted debt in UI polish, per-marketplace HTML heuristics, LLM prompt scaffolding, and the ergonomics of the product-by-marketplace table.
The agent-driven angle sharpens rather than softens this, since agent-written code accretes debt faster and less visibly than human-written code and the binding constraint is the founder's review budget, which is the strongest available argument for pushing everything machine-checkable into lints and types so review can be spent entirely on the zero-debt core.
Differential rigour is itself machine-enforced, since `[workspace.lints]` is overridable per crate.

### Back-of-the-envelope sketches, over the right resources

TIGER_STYLE's design-phase performance sketch transfers as a practice; its resource list does not, because this system's dominant costs are third-party latency, third-party rate limits, and dollars rather than network, disk, memory and CPU.
The sketch that matters before building the sync engine is: listings per hour per browser session on the box; the marketplace's actual throttle and its behaviour at the limit; seconds and cents per LLM listing rewrite; concurrent browser sessions and ingestion subprocesses that fit in the box's RAM alongside the connection pool; tenants per machine at those numbers.
That arithmetic determines the ledger's concurrency ceilings and the pricing tiers, so it is a design-phase decision in exactly the sense the document means, over a different set of resources.
Dollars are a first-class resource here and the only one that scales with tenant behaviour rather than hardware, so the rate limiter bounds spend as well as requests.
The browser session is the other resource that scales with tenant behaviour, and unlike dollars it is bounded by the box, which is why section 4 makes its count fixed at startup rather than elastic.

### Cross-tenant leakage replaces buffer bleeds

TIGER_STYLE's buffer-bleed warning has no literal mechanism in safe Rust under `#![forbid(unsafe_code)]`, but the class it belongs to — one party receiving data belonging to another — is the most damaging bug available in a multi-tenant SaaS and deserves the paranoia the source directs at padding.
`TenantId` is a mandatory, non-defaultable, positional parameter on every repository method and every cache key, so an unscoped query is a type error rather than a review finding.
Do not rely on middleware that sets a current tenant in task-local state; that is precisely the check-to-use gap discussed below, and one forgotten scope is a cross-tenant leak.
Additional surfaces specific to this product are tenant-partitioned upload storage, no accumulation of another tenant's listing text into a shared LLM prompt context, and no other tenant's identifiers in error responses or logs.
Server-side automation adds the largest such surface rather than removing it, and an earlier draft claimed the opposite, so the correction is stated rather than quietly made.
One process now drives many sellers' authenticated marketplace sessions, and a browser is a component that accumulates state nobody asked it to keep.
A per-marketplace profile directory does not discharge this and never could, because separating profiles by marketplace leaves every tenant of the same marketplace sharing one, which is the leak itself.
The requirement is isolation per session and per tenant, and it is one operating-system mechanism rather than a convention: each browser session runs as its own templated systemd unit with a dynamic user and its profile under that unit's own state directory, so the kernel rather than the application owns the boundary, and stopping the unit reaps the browser's child processes that killing the driver does not.
Dynamic user identifiers are recycled from a small range, so a profile directory left behind by one session is a cross-tenant credential leak waiting for a collision; the unit must own its state directory rather than be handed a writable path, and the marketplace cookies persisted between sessions are a minimal enumerated allow-list per origin rather than a whole profile.
Enforce with the integration test that seeds two tenants and asserts zero cross-visibility on every public endpoint; a `trybuild` compile-fail test was considered and deferred, because its output is brittle across toolchain bumps and the integration test gets most of the value.

### Assert immediately before use, since every await is a time gap

TIGER_STYLE's rule that "functions run to completion without suspending" cannot transfer to an async service, and half-adopting it is worse than ignoring it, because an author who believes preconditions hold for a function's lifetime will assert at the top of an `async fn` and reason from that after an await.
What does transfer intact is the document's own place-of-check-to-place-of-use rule: never check a fact read from shared state, then await, then act on it.
Make the check and the act one atomic step and assert on its result, since `UPDATE jobs SET state='running' WHERE id=$1 AND state='pending' RETURNING *` followed by `assert_eq!(rows.len(), 1)` is a conditional write rather than a check-then-write, and is the correct shape for the job ledger's claim path regardless.
Where a value must survive an await, move it rather than re-reading it; precondition assertions on owned arguments remain worth writing, because arguments cannot change underneath an await.

### Comments that are predicates become assertions

TIGER_STYLE's "always say why" and the founder's standing policy that comments are noise by default appear to conflict and do not, because rationale is precisely what code cannot express, so "always say why" is a specialisation of the existing rule.
Where the surprising fact is expressible as a predicate, write the predicate: an assertion that a response page is no larger than the declared page size replaces a comment claiming the same thing and, unlike the comment, cannot drift.
This matters doubly under agent-driven development, because an agent rewriting a module will preserve an assertion that fails loudly and will happily delete or falsify a comment.
Rationale that is not predicable belongs preferentially in the commit message, which survives the refactor that deletes the commented line, while test methodology — what fault is injected, what oracle is used, why this seed — is genuinely not expressible in the test body and therefore clears the existing bar.
Two comment forms are load-bearing because tooling parses them rather than people reading them, and they join the standing carve-out list: `#[expect(lint, reason = "...")]` attributes, and the marker comment a destructive migration carries under section 11 of the operations file.

## 4. Rules deliberately not adopted

Four rejections carry enough weight to be argued rather than tabulated.

### No static allocation, and no ban on allocating after startup

Power of 10 Rule 3 says do not use dynamic memory allocation after initialization and TIGER_STYLE §Safety says all memory must be statically allocated at startup, and neither transfers.
It is not physically satisfiable: `async fn` state machines, every `String`, `Vec`, `Box` and `Arc`, every spawned tokio task and every `Bytes` buffer allocate by construction, and request sizes and arrival rates are unknown until runtime, so there is no startup-time bound to allocate against.
The rule's own justification is also largely discharged already, since Holzmann lists forgetting to free memory, use-after-free, and overstepping allocation boundaries, all of which Rust's ownership model and bounds-checked indexing eliminate outright, and Rule 3's reasoning further depends on Rule 1's recursion ban to make stack usage statically derivable, so the chain is broken at both ends.
Most importantly for this project it is a rule an agent will try to obey and obey badly: told to avoid allocation, an LLM reaches for `static mut` buffers, `unsafe`, arena hacks, or fixed-size arrays with silent truncation, every one of which is strictly worse than the `Vec` it replaced.

What serves the same goal is section 6 in full, whose aim is bounded, predictable memory failing at a boundary of your choosing rather than an OOM kill: bounded channels only, explicit per-route body limits, streaming rather than buffering of PDF, PPTX and ZIP, a cap on decompressed output as the zip-bomb defence, an explicit connection-pool ceiling, and per-tenant quotas.
The one place the static-allocation spirit applies almost literally is the browser pool, and it applies harder on the cloud plane than it ever did on a desktop, so an earlier draft that filed it under desktop concerns had it exactly backwards.
A fixed number of browser sessions is spawned at startup and none is ever spawned on demand.
A session costs roughly four hundred megabytes resident and several CPU-seconds to launch, so demand-spawning converts a burst of tenant activity into a memory cliff that takes every other tenant's in-flight job down with it, which is precisely the shared-resource failure this section exists to prevent.
Pre-spawning also buys backpressure for nothing: work waits for a session that already exists rather than for one that has to be built, so saturation appears in the job ledger as queued work instead of arriving as an out-of-memory kill with the cause far from the symptom.
The count is `limits::browser::SESSIONS_MAX`, and it is the clearest entry in the limits crate under that crate's own admission rule, because exceeding it is an incident affecting tenants other than the one that caused it.
Ingestion gets the other half of the same discipline: it runs out of process under rlimits, which moves the memory bound from something the application must remember into something the kernel enforces, and the browser sessions get that too by each living in its own unit with its own memory, CPU and task limits.

### No zero-dependency policy

TIGER_STYLE §Dependencies states a zero-dependencies policy apart from the Zig toolchain, and adopting it would end this project, since the product cannot exist without tokio, axum, tower, serde, a Postgres driver, an HTTP client, a browser-automation client, a Stripe client and document parsers on the Rust side, nor without the React ecosystem on the front end.
The document's own justification is explicitly conditional on a premise this project does not share — "for foundational infrastructure in particular" — and TigerBeetle is foundational infrastructure with a decade horizon and a funded team, whereas this is a commercial SaaS whose scarcest resource is founder-time, so trading dependency risk for time-to-market is the correct trade here and the opposite trade is correct there.

Keep the reasoning and discard the conclusion.
Run `cargo-deny` with `advisories`, `licenses`, `bans` and `sources`, where licence checking is a legal requirement for a proprietary product rather than hygiene.
Prefer one well-maintained crate to three overlapping ones; be markedly more conservative in crates touching money, auth and cryptography than in crates touching PDF parsing; commit `Cargo.lock` and pin through the flake, which addresses the supply-chain half more rigorously than a dependency count ever could; and keep the dependency graph itself an architectural constraint so dependencies cannot leak into layers that should not have them.

### No crash-on-every-assertion, and no `panic = "abort"`

Covered mechanically in section 3, restated here because it is the rejection most likely to be quietly reintroduced by an agent reconciling the two source documents.
TigerBeetle asserts and aborts in release because it is single-tenant, replicated, and a wrong write is worse than downtime; here the cost function is inverted, because one tenant's edge case must not terminate in-flight sync jobs for every other tenant, and `panic = "abort"` additionally removes the ability to contain a panicking adapter at the worker boundary, which is the one place this architecture genuinely needs containment.
What serves the goal instead is the per-tier policy — `CatchPanicLayer` plus mandatory alerting at request scope, supervised joins and a `needs_human` terminal state at worker scope, literal crash-at-boot for configuration — together with `overflow-checks = true` in release, which catches the wrapping-arithmetic class that motivated much of the original posture at essentially no cost.

### No assertion-density metric

Both sources set a floor of two assertions per function (TIGER_STYLE §Safety; Power of 10 Rule 5); the discipline transfers and is section 5, the number does not, and all three implementation lanes independently reached that conclusion.
This charter takes the stronger position: do not measure it.
Rust's type system already discharges most of what a C assertion checks, so a density floor rewards the codebase that needs more runtime checking and creates pressure against making illegal states unrepresentable, directly against a standing preference.
It is also the single most gameable rule in either document, and in a codebase written largely by agents a numeric target will be met with plausible-looking vacuous assertions that dilute the signal from the real ones while the metric reports health.
Holzmann anticipates precisely this and forbids it in the rule text ("Any assertion for which a static checking tool can prove that it can never fail or never hold violates this rule"), but that clause requires a checker able to prove assertion vacuity in Rust, which does not exist, and an unenforceable anti-gaming clause guarding a gameable metric is worse than no metric.
What serves the goal is `cargo-mutants` (27.1.0), which measures the property actually wanted — would the tests and assertions catch a wrong answer — cannot be satisfied by filler, and names a surviving mutant as precisely a missing assertion or a missing test.
One configuration detail is load-bearing: cargo-mutants' own documentation warns that trees treating warnings as errors report an excessive number of mutants as unviable, because deleting code makes parameters unused, so `cap_lints = true` must be set in `.cargo/mutants.toml` or the mutation score is meaningless under this charter's deny-warnings policy.

The remainder are rejected more briefly.

| Rule | Source | Why not | What serves the goal instead |
|---|---|---|---|
| Functions run to completion without suspending | TIGER_STYLE §Cache Invalidation, "functions run to completion without suspending" | Every `.await` is a suspension, and the rule is not merely inapplicable but actively misleading if half-adopted, since it invites reasoning from a precondition across an await | The document's own place-of-check-to-place-of-use rule: atomic conditional writes, assert on the result, `clippy::await_holding_lock` |
| In-place initialisation via out-pointers | TIGER_STYLE §Cache Invalidation | A workaround for Zig's lack of guaranteed return-value optimisation, needing `MaybeUninit` and `unsafe` to port, and the document itself notes the pattern is viral, which in Rust means unsafe propagating through the type graph | Return values normally; `Box` or `Arc` where pointer stability is genuinely needed; keep `#![forbid(unsafe_code)]` |
| Pass args over 16 bytes as `*const` | TIGER_STYLE §Cache Invalidation | Rust moves by default and `Copy` is opt-in, so the accidental-stack-copy bug cannot occur and the size heuristic is meaningless | `&T` to read, `T` to own, `&mut T` to mutate; `clippy::large_types_passed_by_value` for the residue |
| Total recursion ban | Power of 10 Rule 1 | The justification is instrumental — an acyclic call graph proving bounded execution — and boundedness can be established otherwise, while a blanket ban pushes agents toward hand-rolled stacks that are harder to review | No recursion whose depth is set by untrusted input; explicit worklists there, `Depth(u8)` in the signature elsewhere |
| Function pointers not permitted | Power of 10 Rule 9 | Would ban trait objects, hence the adapter seam and the fake adapter the acceptance loop needs, forcing a `match` on a marketplace enum at every call site, which is worse for humans and much worse for agents | `dyn` where the implementor set is registered in `registry.rs`; enum dispatch where the set is closed; ban stored `Box<dyn Fn>` fields and `Any` |
| No more than one level of dereferencing | Power of 10 Rule 9 | Uncountable in Rust, where `&`, `Box`, `Arc`, `Ref` and auto-deref interact and the compiler inserts derefs invisibly, and `Arc<T>` is one level by the C reading while carrying none of the hazard | A type-complexity budget in the advisory lint lane, targeting `Arc<RwLock<HashMap<K, Arc<Mutex<V>>>>>`, which is what an agent produces when unsure where state belongs |
| Preprocessor macro restrictions | Power of 10 Rule 8 | `macro_rules!` is hygienic and token-tree based, and variadic macros are unavoidable in Rust, so the rule would ban `println!` and every `tracing` macro while catching nothing | Keep the conditional-compilation half at full strength (section 3); redirect the macro concern to first-party proc macros and `build.rs` |
| `unused_results` (strict Rule 7) | Power of 10 Rule 7 | Fires on `HashMap::insert`, `Vec::pop`, `HashSet::insert` and dozens of correct idioms, and the predictable agent response is reflexive `let _ =`, which destroys the deliberate-ignoring marker Holzmann's own `(void)` cast was meant to be | `#[must_use]` plus `unused_must_use`; close the verified holes instead — supervised spawn, `let_underscore_must_use`, `let_underscore_future` and rustc `let_underscore_drop` — and leave `Result::ok` to the substituted-default class, since a ban-list entry has no per-crate form to scope it with |
| `clippy::restriction` or `clippy::nursery` as a group | Power of 10 Rule 10, read as "most pedantic setting" | `restriction` is a menu of mutually contradictory opt-in lints and is not meant to be enabled wholesale, and the argument for keeping `nursery` at warn was void as written, since a `warn` entry combined with `--deny warnings` was measured on 1.97.1 to produce a hard error | Deny `clippy::all` and `clippy::pedantic` with named exceptions, hand-pick from `restriction`, and move `nursery` and `cargo` into a separate non-gating advisory clippy invocation where `warn` genuinely means warn |
| `clippy::must_use_candidate` | Proposed, rejected | Measured at 6 warnings in 25 lines of ordinary service code, firing on essentially every pure function and training agents to paper the codebase with meaningless attributes | `Result` and `Option` are already `#[must_use]`; apply `#[must_use]` by hand to the few domain types where dropping the value is a real bug |
| `clippy::panic_in_result_fn` | Proposed by one lane, rejected by two | Its own description covers functions returning `Result` that contain a panic or assertion, so under railway-oriented error handling it forbids assertions in exactly the functions section 5 targets, and the two rules cannot both be enforced | The section 5 convention: `Result` carries operating errors, `assert!` carries programmer errors, and both legitimately coexist in one function |
| Miri as a routine check | Proposed, rejected | Under `#![forbid(unsafe_code)]` there is nothing of yours for it to examine, it cannot run this code anyway because it has no syscalls and so no tokio, axum, database or browser-driver test executes under it, and it needs nightly | Nothing routine, because with React rather than Leptos there is no wasm-bindgen crate, and no client carries Rust at all, so no crate is currently expected to drop `forbid` |
| Fuzzing the ingestion parsers as the adversarial-input control | Proposed, rejected as the primary control | You will depend on a ZIP, OOXML and PDF parser rather than write one, so fuzzing mostly produces findings in someone else's crate that you cannot fix and must carry as an unactioned backlog | Run ingestion out-of-process under memory, CPU and wall-clock rlimits and reap it, discharging the zip-bomb, decompression-ratio and runaway-parse limits at the OS layer, where they hold against a parser bug rather than only against the inputs you thought of |
| `[bans.build] executables` and `interpreted` in cargo-deny | Proposed, rejected | Verified against cargo-deny 0.20.2 `src/bans/cfg.rs`, both scan crate contents for shipped binaries and interpreted scripts, and neither has anything to do with which dependency tree a build script pulls in, which is what an earlier draft's comment claimed | Nothing here, since the architectural-edge goal is served by `[bans] deny` entries and by the crate graph itself, which is where it was already served |
| `cargo-deny` `[bans]` as the check on a crate's whole dependency closure | Proposed, rejected | Its `wrappers` field matches direct parents only, measured on a purpose-built graph, so a crate reaching a banned crate through a permitted parent passes and the rule reports green while being false | `[bans]` for the edges it can express, meaning a banned crate with one permitted parent; an `xtask` walk of the closure `cargo metadata` resolves, for `sync-core` purity |
| A one-line grep over clippy's output as the control on configuration typos | Proposed, rejected | It covers one of the four ways a lint table or ban list fails silently, missing a misspelt lint name, a renamed one, and a ban path no compilation unit ever resolves, each of which leaves the build green | A scan of the gated lane's JSON output failing on the code `E0602`, the code `renamed_and_removed_lints`, or the text `does not refer to a reachable`, over a workspace carrying a probe crate that references every crate the ban list names |
| `cargo-semver-checks` | Proposed, rejected | It checks Rust crate APIs against semver, whereas this project's versioned public contract is HTTP and no crate is published, so it would police internal workspace boundaries nobody depends on | Generate OpenAPI from the axum handlers with `utoipa` (5.5.0), commit the spec, and diff it in CI with `oasdiff` so a breaking wire change fails the build |
| Acronym capitalisation (`VSRState` over `VsrState`) | TIGER_STYLE §Naming | Opposite to Rust convention, and agents trained on idiomatic Rust will drift back constantly, producing perpetual meaningless review churn | Follow Rust convention; `clippy::upper_case_acronyms` is on by default and enforces it in the idiomatic direction |
| Newline grouping around allocation; braces on single-statement `if` | TIGER_STYLE §Cache Invalidation and §Style By The Numbers | Moot, since Rust has no `defer`, RAII means deallocation is not a statement to pair visually with anything, and braceless `if` bodies do not parse, so the goto-fail class is structurally impossible | Nothing, though it is worth recording as an illustration of the general principle: prefer rules the compiler enforces over rules people follow |
| Hot-loop mechanical sympathy and primitive-argument extraction | TIGER_STYLE §Performance | There is no hot loop in the relevant sense, since time is spent waiting on a browser session, an LLM completion and a rate limiter, all in seconds, and extracting primitive-argument functions also defeats the newtype discipline adopted above | Optimise third-party latency, rate limits and dollars: batch database writes and LLM calls, cache crosswalk lookups, keep the pre-spawned browser sessions warm, and revisit CPU only if document parsing shows up in a profile |
| Build a VOPR-style whole-system simulator | TIGER_STYLE §Safety, where VOPR is named once and never explained | `src/vopr.zig` is roughly 1,800 lines atop some 7,000 lines of harness, built by a funded team over years, and much of its scope is consensus, view changes and storage fault atlases this system does not have | The proportionate simulator in section 7, over a sans-IO core: a few hundred lines, days rather than months, aimed at the faults that actually occur |
| madsim, turmoil, loom, shuttle, Antithesis | Ecosystem survey, 2026-08 | madsim (0.2.34) requires forked tokio and tonic plus `[patch.crates-io]` overrides and a `RUSTFLAGS` cfg, covering none of this project's actual I/O; turmoil (0.7.2) simulates a peer network this engine does not have and warns its own API is in flux; loom tests atomics that do not exist here; shuttle targets a concurrency bug not yet found; Antithesis is a commercial hypervisor platform and still cannot simulate the marketplace | The bespoke simulator over the sans-IO core needs no dependency surgery; revisit turmoil if API and workers become separate networked processes, and shuttle if a real job-ledger race appears |
| Creusot, Prusti, Verus, Flux | Verification-tool survey, 2026-08 | Verus requires writing in a Verus dialect rather than verifying ordinary Rust, Creusot requires Why3 and Pearlite, Flux has no tagged releases, and Prusti's latest release is tagged v-2023-08-22-1715 despite ongoing repository activity | Kani on a narrow subset only, per section 7, since its docs state `await` is unsupported and concurrency is out of scope, confining it to the taxonomy crosswalk and a pure transition function |
| `clippy::cognitive_complexity` as a hard deny | Proposed, rejected as a gate | The heuristic has a long history of firing on clear code and missing unclear code, and denying it produces arbitrary function splits that make code harder to follow, directly contradicting "centralize control flow" | The function-length figure, which is objective, kept in the advisory lane with a generous threshold as a prompt for review |
| `clippy::print_stdout` and `print_stderr` workspace-wide | Proposed, rejected at that scope | Correct in service crates and wrong in the `xtask` helper and in build scripts, both of which legitimately write to stdout, and denied globally it produces per-file `#[allow]` attributes, which is where a lint stops meaning anything | Deny at the crate root of the axum, service and adapter crates, with `allow-print-in-tests = true`, and deny `clippy::dbg_macro` and `clippy::todo` workspace-wide, where the case is unambiguous |

## 5. The assertion discipline

Both source documents are built around assertions and both make the same distinction in different words.
TIGER_STYLE §Safety states it directly.

> Assertions detect programmer errors.
> Unlike operating errors, which are expected and which must be handled, assertion failures are unexpected.
> The only correct way to handle corrupt code is to crash.

Holzmann's Rule 5 arrives at the same place from the other side, and his own worked example is almost universally misread — `if (!c_assert(p >= 0) == true) { return ERROR; }` returns an error rather than aborting, and he notes that where there is nowhere to print, "the assertion turns into a pure Boolean test that enables error recovery from anomolous behavior."
So the two documents together describe a partition rather than a hierarchy: an assertion is a claim that the program's own reasoning is sound, and a `Result` is a report about the world.
This composes with railway-oriented error handling rather than competing with it, because the two never overlap.

### The three-question test

Applied in order, at the point of writing.

1. Can this condition be produced by data crossing a trust boundary — an HTTP request body, marketplace HTML, an uploaded PDF, PPTX or ZIP, an LLM completion, a Stripe webhook, a database row written by an older build?
   Then `Err`, always, because an assertion here is a remote denial of service.
2. Can it be produced by the world being flaky — a timeout, a 429, a redesigned form, a declined card, a crashed browser session?
   Then `Err`, since these are TIGER_STYLE's operating errors.
3. Is there any input or environment that produces this other than the code being wrong?
   If no, then `assert!`.

The architectural seam that makes this mechanical rather than judgemental is parse-don't-validate.
The smart constructor is the membrane: `Listing::try_new(raw) -> Result<Listing, ListingError>` returns `Err` on the outside, and every function downstream takes `Listing` rather than `RawListing` and asserts its invariants, because a violation there means the constructor is broken or was bypassed, which is a bug by construction.
Stated compactly: `Err` on the way in, `assert!` once inside.

### Closing the deserialisation hole

`#[derive(Deserialize)]` on a validated newtype silently bypasses the smart constructor and materialises illegal states straight out of JSON or Postgres, so every type crossing the trust membrane declares `#[serde(try_from = "RawX")]`.
The phrase "crossing the trust membrane" is the operative narrowing: it applies to types deserialised from an HTTP body, a marketplace response, an LLM completion, a Stripe webhook, or a database row, and not to every type in the domain crate, because applied to every type it produces a `RawX` shadow for internal structs that never leave the process, which is ceremony with no membrane behind it.

An earlier draft proposed a CI grep for `#[derive(Deserialize)]` in the domain crate, and that grep should be deleted rather than fixed.
Measured against four realistic derive forms it found one of the three that actually carry `Deserialize`, because it cannot see `#[derive(Debug, Deserialize)]`, cannot see the `#[derive(serde::Deserialize)]` path form, and cannot see `#[derive(sqlx::FromRow)]`, which is the database path it was meant to guard; rustfmt also breaks a derive list past `max_width` onto one item per line, at which point no single-line grep can see any of it.
A grep with that hit rate is worse than no grep, because it reports green and stops anyone looking.
What replaces it is the integration test that already exists for a different reason — seed two tenants, exercise every public endpoint, assert zero cross-visibility — where a type that bypassed its smart constructor shows up as a value that should have been rejected.
If a mechanical check is wanted later, the only sound one is a lint over the HIR rather than over the text, which is a `dylint` crate rather than a grep, and a real project rather than a day-one one.

### Concrete rules

Use `assert!` rather than `debug_assert!` for anything guarding cross-tenant boundaries, money, job-state transitions, or a write to a marketplace, and let `[profile.release]` carry `debug-assertions = true` so the rest fire in production too.
Reserve `debug_assert!` for checks whose cost is superlinear in the operation, such as re-verifying a whole collection is sorted or re-hashing an uploaded file.
Split compound assertions, prefer `assert_eq!` for its operand printing, and write implications as `if a { assert!(b); }`.
Every assertion carries a message, enforced by `clippy::missing_assert_message`, because in agent-driven development the panic message is often the only signal the founder gets from a module they did not read line by line.
Assert the negative space as well as the positive, per TIGER_STYLE §Safety, but prefer modelling the negative space out of existence with an enum, since an exhaustive `match` is checked at compile time and an assertion is not.
Push relationships between constants into `const { assert!(...) }` blocks, which fail the build rather than the process.
Pair assertions across every lossy boundary: before and after serialisation, before the adapter fills the form and again against the scraped-back confirmation, before the database state transition and again on the `RETURNING` row, before the Stripe call and again on the webhook.

### What review is for, and what it is not

The ordering TIGER_STYLE §Safety prescribes is the highest-leverage item in either document for this project, because agent-driven development's characteristic failure is its exact inverse: agent writes code, tests go green, no mental model exists anywhere.
Build the mental model first, encode it as assertions and types, then write the implementation, then run property tests and fault injection.

An earlier draft then said that review targets the assertions and type signatures rather than the bodies, and that claim was doing more work than it can carry.
An agent that writes both the code and its assertions writes assertions that agree with the code, so where the model of the problem is wrong the assertions encode the same wrong model, agree with the implementation perfectly, and review of the assertions alone returns green.
This is the mechanism by which a plausible wrong feature ships unreviewed, and it is the most consequential claim in this document, so it is stated as a limit rather than as a technique.

What the assertion discipline actually buys is a narrower review rather than a substituted one.
Reading fifteen assertions and six signatures tells you whether the model is the one you intended, which is a real and cheap question to answer and the one a solo founder can afford to ask on every change.
It does not tell you whether the implementation matches that model, and only three things do: property tests whose oracle is independent of the implementation, `cargo-mutants`, and reading the body.
The zero-debt core therefore gets its bodies read, in full, every time; the periphery gets the narrowed review, and the honest reason is that its bugs are cheap and visible rather than that the narrowed review is sufficient.
The ordering matters too, so write the assertions before the implementation, in a separate pass, or they encode a description of the code that emerged rather than the model you had before it existed — an assertion written after the body it guards is a comment with a panic attached.
The document's own caution applies and should be quoted rather than paraphrased: "a fuzzer can prove only the presence of bugs, not their absence."

### Worked examples

A marketplace adapter that receives an unexpected DOM state returns `Err`, without exception.
The marketplace's HTML is untrusted input from a third party who can change it without notice, so question 1 answers yes on the first pass and this is also question 2's central case, and the specific failure — the expected selector is absent, or the page navigated somewhere unanticipated — is not a defect in this code but the single most likely routine event in the entire system.
Asserting here would convert every marketplace redesign into a worker-killing panic storm affecting all tenants.
The error must be typed precisely rather than collapsed, because recovery differs: `SelectorMissing { selector }` is permanent until someone fixes the adapter and routes to `needs_human`, while `Timeout` should back off.
The one assertion that belongs nearby is on the way out, asserting after a successful submission that the scraped confirmation agrees with the payload that was sent, which is the pair-assertion that will actually catch marketplace drift in production.

The job ledger popping a row asserts, but only on the result of an atomic operation, never on a value read beforehand.
The claim is written as a conditional write — `UPDATE jobs SET state='running' WHERE id = $1 AND state = 'pending' RETURNING *` — with `assert_eq!(rows.len(), 1)` after it, because reading the row's state, awaiting, and then asserting it is still pending is the place-of-check-to-place-of-use bug in its purest form and, in a ledger with any concurrency, a live race.
Once the row is claimed its state transition is entirely governed by code this project owns, so `assert!(matches!(job.state, JobState::Running))` inside the state machine is a bug check and belongs as an assertion.
Two adjacent cases are `Err` rather than assertions: zero rows returned means another worker claimed it or the lease expired, which is an ordinary operating outcome, and a row whose `state` column holds a value this build's enum does not recognise is data written by an older or newer deployment crossing a trust boundary, so it deserializes to `Err`.

A taxonomy lookup that finds no mapping is the genuinely ambiguous case, and the answer depends on where the boundary was drawn.
If the crosswalk is validated at load time to be total over the `UsGrade` enum — and it should be, being project-owned data, small and finite — then a missing mapping at lookup time means the validation is broken or was bypassed, and it asserts; better still, make the lookup infallible by returning `UkAgeRange` rather than `Option<UkAgeRange>` from a table proven total at startup, so there is nothing to assert because there is nothing to miss.
If instead the input grade came from a marketplace response or a user-supplied CSV and has not yet been parsed into `UsGrade`, the failure is at the membrane and returns `Err(TaxonomyError::UnmappedGrade { .. })`.
The rule generalises: a missing mapping for a value the type system guarantees is in range is a bug, and a missing mapping for a value that arrived as a string is an input error, and what makes this cheap is doing the totality check once at boot, where a failure kills the process before it binds the listener.

A Stripe webhook arriving with a bad signature returns `Err`, and specifically a 400 with no further processing.
Signature verification exists precisely because the input is untrusted and attacker-controlled, so an assertion here is a trivially-triggerable remote denial of service against the whole billing path, and it is not an anomaly worth alerting on at low volume, since scanners will send garbage to any public endpoint.
The handler validates the signature, enqueues the event and returns 200, with no reconciliation inline per the run-at-your-own-pace rule.
Every assertion in this area lives downstream of that membrane: after the enqueued event is parsed into a domain type, assert that the amount is non-negative, that the tenant referenced exists in the same transaction, and that applying it twice is a no-op, because at that point a violation means the reconciliation logic is wrong rather than that someone sent a bad request.

## 6. Bounding everything

TIGER_STYLE §Safety says to put a limit on everything because in reality everything has a limit, and Power of 10 Rule 2 requires every loop to carry a statically provable upper bound.
This is the rule that carries the actual content of "all memory allocated at startup" into a context where that rule cannot apply, and it matters more here than at its source, because this service is multi-tenant so one tenant's pathological input must not exhaust a shared resource, and its dependencies are third-party websites and LLM APIs whose behaviour is not under this project's control.
An unbounded queue does not fail cleanly; it degrades invisibly until memory or latency collapses, at which point the cause is far from the symptom.

Holzmann's carve-out is honoured explicitly and matters here, because an agent applying Rule 2 mechanically will bound the worker loop and silently stop the service after N iterations.

> This rule does not, of course, apply to iterations that are meant to be non-terminating (e.g., in a process scheduler).
> In those special cases, the reverse rule is applied: it should be statically provable that the iteration cannot terminate.

The worker loop and the accept loop exit only on an explicit shutdown signal, and every iteration either makes progress or awaits.
`clippy::infinite_loop` is denied workspace-wide so each such loop must carry `#[expect(clippy::infinite_loop, reason = "...")]`, which yields a greppable inventory of every intentionally unbounded loop in the system.

### What belongs in the limits crate

An earlier draft listed roughly sixty constants, marked three of them uncalibrated, and left the other fifty-seven implying a precision that does not exist.
That is a maintenance surface impersonating discipline, and under agent-driven development it is worse than it looks, because an agent reading sixty named bounds will use them, wiring dependencies on numbers nobody chose.

The crate ships with ten constants and an admission rule, and the admission rule is the durable part.
A constant belongs in `crates/limits` only when exceeding it is a shared-resource incident affecting tenants other than the one that caused it, and when no type, database constraint, or OS-level limit already bounds it; a number that bounds only its own caller is an ordinary `const` next to that caller.
By that test the pagination page budget, the XML depth counter and the channel capacity all move out, since each is real and each is enforced but none is a cross-tenant blast radius.
The browser pool sizing moves the other way, and the reason is the architecture rather than a change of mind: under local-first execution it bounded one seller's own laptop and belonged next to its caller, and under server-side automation it bounds memory every tenant shares, which is the admission rule's central case.
The discipline that is genuinely day-one is not the capacity number but the ban on `unbounded_channel`, which lives in `clippy.toml` and costs nothing to impose early.

The second durable part is that every constant carries a provenance marker in its doc comment and that the marker is a factual claim.
`MEASURED` cites an observation recorded during the spike and is carried by no constant yet, because the spike has not run; `SIZED` means derived by arithmetic from a quantity known independently of the spike, such as the box's RAM or a protocol limit; `UNCALIBRATED` is a guess, and is a release blocker for the first paying deployment rather than a wish.
A test counts the `UNCALIBRATED` doc markers and asserts the total against a committed budget, currently seven, so lowering the budget is a deliberate edit and raising it cannot pass unnoticed.
The crate has no dependencies and must keep none, so that every other crate can depend on it without acquiring an edge.

| Constant | Provenance | Value | Why it is a shared-resource bound |
|---|---|---|---|
| `http::REQUEST_BODY_BYTES_MAX` | SIZED | 2 MiB | Sized against RAM rather than traffic, so concurrent requests cannot buffer the box |
| `http::UPLOAD_BODY_BYTES_MAX` | UNCALIBRATED | 256 MiB | No file-size census of any connected marketplace's resources has been taken |
| `ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX` | SIZED | 1 GiB | The absolute ceiling on bytes written out of an archive |
| `ingest::ARCHIVE_COMPRESSION_RATIO_MAX` | UNCALIBRATED | 200 | Checked incrementally; the ratio distribution of real seller bundles is unsampled |
| `job::ATTEMPTS_MAX` | UNCALIBRATED | 5 | Meaningful only once the fault taxonomy says which faults are worth retrying |
| `job::WALL_CLOCK_MAX` | SIZED | 30 minutes | A wedged job must free its lease well inside one working day |
| `job::CONCURRENT_JOBS_GLOBAL_MAX` | UNCALIBRATED | 8 | Ingestion subprocesses and the connection pool against the box's unmeasured RAM |
| `browser::SESSIONS_MAX` | UNCALIBRATED | 2 | Sessions are pre-spawned at startup, so this is the pool every tenant shares |
| `llm::CENTS_PER_TENANT_PER_DAY_MAX` | SIZED | 500 | The daily spend the business will lose to a runaway loop before a human looks |
| `marketplace::OUTBOUND_REQUESTS_PER_MINUTE_MAX` | UNCALIBRATED | 30 | Legal rather than operational, and gated on the terms answer in section 1 |

The tier quotas are the eleventh entry and are a type rather than a constant, which is the substantive fix rather than a repair: one `TierQuota` returned from an exhaustive `match` replaces three parallel arrays indexed by a discriminant.
The module's source, the five compile-time relationship assertions and the five-test module are listed in [`enforcement-toolchain.md`](enforcement-toolchain.md) alongside the other three primary artefacts, and that listing compiles, lints and tests clean on rustc 1.97.1, cargo 1.97.0 and clippy 0.1.97.

### What changed from the earlier draft, and why

`Tier` was never defined, so the earlier block did not compile, producing six `E0433: cannot find type Tier in this scope` errors rather than the four the critique reported.
The parallel arrays it indexed are gone, and that is the substantive change: three of the earlier const assertions checked that arrays declared `[T; Tier::COUNT]` had length `Tier::COUNT`, which an array of that type cannot fail, so exactly the vacuity this charter condemns when it rejects the assertion-density metric was sitting in the charter's own showcase.
Replacing three parallel arrays with one `TierQuota` returned from an exhaustive `match` removes the index, the bounds check, and the thing the vacuous assertions were pretending to check, and adding a fourth variant now produces `E0004: non-exhaustive patterns` in both the library and the test.
The assertions that remain are ones that can actually be false after an edit, and breaking one produces `E0080` carrying its own message.

Every assertion now carries a message, which reconciles `clippy::missing_assert_message` with `const { assert!(...) }`; the earlier block fired that lint nine times, so the adopted rule and the showcase were mutually exclusive as written, and since a literal message is permitted in const evaluation the two rules compose once the messages are written.
The wildcard imports are gone: seven of the eight `use super::*;` lines fired `clippy::wildcard_imports` under the charter's own deny of `pedantic`, not the three the critique reported, and they could have been kept by allowing that lint, but `wildcard_imports` is the one pedantic lint whose purpose is making the provenance of a name visible in the file where it is used, which is worth more under agent-driven development than under human review.
`Duration::from_secs(30 * 60)` became `Duration::from_mins(30)`, because `clippy::duration_suboptimal_units` fired eight times on `from_secs` values that are whole minutes and did not fire on `Duration::from_millis(500)` at all, which is the opposite of what the critique reported on both count and site; `from_mins` and `from_hours` are stable and const on 1.97.1, verified by compiling and running them.
Byte bounds are `u64` rather than `usize` so the widths are explicit, per the boundary half of the explicitly-sized-types rule, with one const assertion recording that the conversion to `usize` at the axum boundary is total on the platforms shipped.

The operations file names further candidates for admission — a migration lock timeout, a fleet-wide LLM ceiling, a signup rate, a canary cadence — and each is admitted by the same rule, arriving in the same change as the function that enforces it and never before.

### How the limits are actually applied

Constants only bind if the surfaces that could bypass them are closed, so each category has a matching enforcement and most of the enforcement is not a constant.
Bounded channels only, with `unbounded_channel` and `futures::channel::mpsc::unbounded` in `disallowed-methods`, and real backpressure meaning the producer awaits `send()` rather than `try_send`-and-drop.
Request bodies capped per route with `DefaultBodyLimit::max(..)` plus `tower_http::limit::RequestBodyLimitLayer`, and the streaming upload reader counting bytes itself, because axum's default covers only `Bytes`-derived extractors and is bypassed the moment a handler streams the body, while `Content-Length` is attacker-controlled and chunked encoding carries none.
Decompressed output bounded via `Read::take(..)`, since bounding compressed input is not a zip-bomb defence, with the whole ingestion step additionally run out-of-process under rlimits as described in section 7.
Every outbound HTTP client built through `reqwest::ClientBuilder` with `.timeout()` and `.connect_timeout()` set, and every external call additionally wrapped in `tokio::time::timeout`.
Retry loops written as `for attempt in 0..job::ATTEMPTS_MAX` with a wall-clock deadline alongside.
Pagination loops bounded by a strict-cursor-advance check as well as a page count, since a marketplace returning the same cursor forever will loop inside a page budget indefinitely if the budget is counted per page rather than per iteration.
Selector waits bounded by both a deadline and a poll count, which is the highest-risk loop in the system because a redesign makes a selector permanently unmatchable.
Browser sessions bounded by being pre-spawned at `browser::SESSIONS_MAX` and leased, so there is no code path that creates one, and by three independent deadlines with deliberately different roles: the WebDriver client's per-command timeout, fixed when the client is constructed; a cancellation token carrying the job's wall-clock budget, which is the path that records an outcome; and a systemd runtime limit set strictly longer than that budget, which is the backstop that produces an ambiguous outcome when the process itself is wedged.
Each of these is backed by a test that feeds the pathological case and asserts a typed `Err` rather than a crash.

## 7. The testing pyramid, and where simulation sits

The pyramid sits underneath the existing acceptance-test outer loop and inner TDD cycle rather than replacing them; nothing in either source document competes with those.

| Tier | What it covers | Cost | Where it runs |
|---|---|---|---|
| Type-level and `const` assertions | Illegal states, constant relationships, tenant scoping, exhaustive `match` over closed sets | Free | Every build |
| Unit and example tests | Ordinary logic, plus one test per `Err` variant that exercises the handler | Milliseconds | Fast flake check |
| Deterministic scenario tests | Scripted fault sequences against the pure sync core, no async, no sleeps, no mocking framework | Milliseconds | Fast flake check |
| Property tests | Taxonomy crosswalk totality and round-trip; per-marketplace mapping laws; parser invariants | Seconds | Fast flake check |
| Acceptance tests | The Gherkin outer loop, driven through the fake adapter | Seconds | Fast flake check |
| Sandboxed ingestion | ZIP, OOXML and PDF parsing in a child process under rlimits, over a hostile-document corpus | Seconds | Fast flake check |
| Deterministic simulation | Seeded fault injection over the sync state machine, with checkers and a convergence phase | Minutes | Scheduled, plus a committed seed corpus in the fast check |
| Prompt-injection corpus | The listing-copy pipeline against fixtures shaped like OWASP's scenario list | Seconds | Scheduled |
| Mutation testing | Whether the assertions and tests would catch a wrong answer | Hours | Scheduled, `--in-diff` on pull requests |
| Bounded model checking (Kani) | Taxonomy crosswalk totality; the pure transition function | Minutes per harness | On demand |
| End-to-end against a real browser | The adapter against a real sandbox account, on the server-side automation plane | Slow, non-deterministic | Scheduled, outside the deterministic tier |

Three placements deserve justification.
Mutation testing rather than assertion counting is the measure of assertion quality, for the reasons in section 4.
Kani is the only realistic verifier and applies to two places only, because its documentation states that `await` is unsupported and concurrency is out of scope: the taxonomy crosswalk, where totality and round-trip are exactly the shape of property a bounded model checker proves well, and the sync transition function, provided it is extracted as a pure synchronous function; it requires bounded loops, so section 6 pays for itself twice, and if the crosswalk domain turns out small and finite an exhaustive test over the cross product may dominate a proof harness for a fraction of the effort.

Sandboxed ingestion replaces fuzzing as the adversarial-input control, and the reasoning belongs here rather than in a table cell.
This project will depend on a ZIP, OOXML and PDF parser rather than write one, so a fuzzer's findings mostly land in someone else's crate, and a finding you cannot fix is a backlog item rather than a control.
Running ingestion out-of-process under `setrlimit` and reaping it discharges the decompression-ratio ceiling, the absolute expansion ceiling and the runaway-parse case at the OS layer, where they hold against a parser bug rather than only against the inputs you enumerated, and where the failure is a killed child and a typed `Err` rather than a dead service.
It also converts the memory bound from a semaphore into a cgroup, which is the same argument that already puts each browser session in its own systemd unit, so the two are one decision and it is made rather than pending; fuzzing remains worth doing on any parser this project writes itself, which currently means none.

### Where deterministic simulation sits, and what it is not

The bug class simulation finds and property tests do not is the interleaving bug, a fault arriving at one specific point in a multi-step protocol, and here that is precisely the highest-cost failure this product can ship: a publish that times out after the marketplace created the listing, producing duplicate listings on a paying seller's storefront.
TigerBeetle achieves determinism by stubbing the clock, network and disk, driving everything from one tick loop seeded by a single `u64` that, together with the git commit, replays any failure exactly.
That technique transfers, but the remote here is a real browser driving a real website and is not simulable, and simulating the marketplace would test this project's model of it rather than the thing itself, drifting and giving false confidence exactly where the real risk lives.
So the boundary behaviour is what gets simulated — latency, failure, ambiguity, and wrong-but-well-formed answers — and the real browser is covered separately by a small, slow, deliberately non-deterministic end-to-end suite, which is what TigerBeetle itself does alongside its simulator.

Three warnings, one of which is the most likely self-deception on this path.
`tokio::time::pause()` and `#[tokio::test(start_paused = true)]` control one of the four axes of nondeterminism, making backoff and timeout tests instant, which is genuinely valuable and free, but they do nothing about randomness, task scheduling or I/O completion order, so a test using them is fast rather than reproducible.
Second, three ambient nondeterminism sources leak into a nominally deterministic core and are invisible in review: `std::collections::HashMap`'s per-process `RandomState` iteration order, `tokio::select!`'s deliberately randomised branch polling, overridable with `biased;`, and unseeded `Uuid::new_v4`.
Third, a run that finds a bug it cannot reproduce has produced anxiety rather than information, so the seed is printed on every run and a failing seed is committed under `sync-sim/seeds/` and replayed by the fast check forever after.

### The trait seam

The seam is worth more than the simulator it enables and should be cut on the first day the sync engine exists, because retrofitting it later means rewriting the engine.
It is also the single best structural decision for agent-driven development, since a synchronous, dependency-free state machine is the largest unit of business logic an agent can hold entirely in context and modify without reaching for anything it cannot see, and a failing seed is a complete, self-contained bug report to hand it.
Sync being deterministic scheduled execution rather than agentic execution is what makes the seam sound rather than decorative, for the reasons given in section 3, and the settled architecture strengthens that argument rather than weakening it.
The schedule is a cron this project owns, running on a machine this project operates, so the thing the seam models is genuinely a pure state machine and not an approximation of one.
Everything above the adapter is a sans-IO state machine with no async, no clock read and no ambient randomness; time enters only as data, randomness only as a seeded generator or an explicit jitter input.
The declarations below are illustrative and uncompiled: a shape to build to, not a verified artefact.

```rust
// crates/sync-core — no tokio, no reqwest, no sqlx, no browser driver.

pub enum SyncCommand {
    StartBatch { batch: ListingBatch, now: Timestamp },
    ProbeCompleted { listing: ListingId, outcome: Result<RemoteListing, AdapterError> },
    PublishCompleted { listing: ListingId, outcome: Result<PublishReceipt, AdapterError> },
    TimerFired { token: TimerToken, now: Timestamp },
    OperatorResolved { listing: ListingId, choice: ConflictChoice },
    LeaseLost { at: Timestamp },
    CancelRequested,
}

impl SyncMachine {
    /// Total, synchronous, allocation-light. Describes work; never performs it.
    /// `SyncEffects` is a bounded `SmallVec` of `SyncEffect`: Probe, Publish,
    /// SetTimer, RecordDrift, EscalateConflict, BatchFinished.
    pub fn step(&mut self, cmd: SyncCommand) -> SyncEffects { /* pure */ }
}

// crates/sync-ports — the one abstraction that earns its keep.

#[async_trait::async_trait]
pub trait MarketplaceAdapter: Send + Sync {
    async fn probe(&self, r: &MarketplaceRef) -> Result<RemoteListing, AdapterError>;
    async fn publish(&self, p: &ListingPayload, k: &IdempotencyKey)
        -> Result<PublishReceipt, AdapterError>;
    async fn retract(&self, r: &MarketplaceRef) -> Result<(), AdapterError>;
    fn capabilities(&self) -> MarketplaceCapabilities;
}

pub enum AdapterError {
    /// Outcome unknown: the write MAY have landed. Never collapse into Timeout.
    Ambiguous { after: Duration, evidence: AmbiguityEvidence },
    Timeout { after: Duration },
    RateLimited { retry_after: Option<Duration> },
    AuthExpired,
    SelectorMissing { selector: SelectorId },
    UnexpectedNavigation { to: Url },
    PartialSubmit { accepted: Vec<FieldId> },
    RemoteRejected { code: RemoteRejectCode, field: Option<FieldId> },
    Transport(TransportFault),
}
```

Note `fn step`, not `async fn`, and effects returned as a bounded collection rather than an unbounded `Vec`.
The driver in the outer crate owns the tokio runtime, executes each effect against the adapter, and feeds the result back as the next command.
Three adapter implementations exist: the real per-marketplace adapter driving a leased server-side browser session over WebDriver, `SimulatedAdapter` (an in-memory marketplace model plus a seeded fault injector), and `ReplayAdapter` (a recorded real trace); the trait stays narrow, with no `run_arbitrary_script` escape hatch, because anything that escapes the trait escapes the simulator.

`AdapterError::Ambiguous` is the decisive variant and the reason to enumerate the fault taxonomy as a type rather than injecting a generic error.
Collapsing an ambiguous write into `Timeout` tells the state machine a lie, and the resulting duplicate listing is both the most likely and the most expensive bug this product will ship.
Every publish therefore carries an `IdempotencyKey` derived from product, marketplace and content hash, so the recovery path for `Ambiguous` is a defined probe-then-reconcile rather than a guess.
`clippy::wildcard_enum_match_arm` denied in `sync-core` makes adding a fault variant a compile error at every handling site, which is the machine-checked form of "all errors must be handled".

### Enforcing the seam, and the simulator's shape

The purity of `sync-core` is enforced by its dependency graph rather than by discipline, and by a check that can see the whole of that graph: an `xtask` step walks the resolved closure `cargo metadata` reports for `sync-core` and fails when tokio, reqwest, sqlx or the browser driver appears anywhere in it.
The three ambient leaks get three different controls, because the one control an earlier draft named for all three does not exist.
A crate-local `clippy.toml` was that answer, and it cannot be used at all: a crate-local file replaces the workspace-root one rather than extending it, measured in the toolchain file, so adding three entries for `sync-core` would delete every entry the workspace list holds for that crate.
The ambient clock is therefore covered by the workspace ban list denying `SystemTime::now` and `Instant::now` everywhere, which makes the few legitimate clock reads a greppable inventory of `#[expect]` attributes rather than an assumption; ambient randomness is covered by the closure walk, since a generator has to arrive as a dependency and the walk sees dependencies; and hash iteration order is covered by the determinism test below rather than by a lint, which is stated as a limit rather than as coverage.
A `cargo-deny` `[bans]` entry was the earlier answer for the closure and does not hold either, because its `wrappers` field matches direct parents only, so a crate reaching a banned crate through a permitted one passes; the measurement is in the toolchain file, and `[bans]` keeps the edges it can genuinely express, such as the browser driver having exactly one permitted parent.
A test that runs the same seed twice in one process and asserts byte-identical effect traces catches any leak the lints miss, and is worth writing first.
The simulator itself is a few hundred lines over that core: a seeded `rand_chacha` generator, a virtual clock as a monotonic `u64` of simulated nanoseconds, a priority queue of pending effects with sampled completion times, the `SimulatedAdapter`, a fault injector parameterised by probabilities, and a checker set.
The checkers assert business invariants rather than the absence of panics, because "it did not crash" is a weak oracle: no two remote listings for one product-marketplace pair; batch accounting closes; a retry never un-succeeds an item that previously succeeded; no publish is issued for an unresolved conflict; every timer token is either fired or cancelled.
Each run has two phases mirroring the VOPR's own structure, a chaos phase with faults enabled and then a convergence phase with the injector disabled, and failing to converge is a distinct, loudly-named failure carrying the seed rather than a generic timeout — because safety checking alone passes trivially for a system that gave up and did nothing, and for a product whose promise is that listings stay in sync, a stuck sync is indistinguishable from a broken one.
A second return on the seam arrives long before the simulator does: once the engine is a pure state machine driven by explicit commands, writing "publish times out, then the probe shows the listing exists, then the seller edits it locally" is a dozen deterministic lines with no mocking framework and no flakes, which is the shape of an acceptance scenario, so the same harness serves the existing outer loop rather than competing with it.

## 8. The enforcement toolchain

The `[workspace.lints]` table, the `clippy.toml`, the `deny.toml`, the release profile, the limits module and the flake checks with their speed tiers live in [`enforcement-toolchain.md`](enforcement-toolchain.md).
They are a sibling file rather than a section because they are the part of this charter edited most and read least, and because a single file carrying all of it would exceed the length at which any part stays reviewable.
Four findings recorded there change rules stated above and should be read before any lint is added.

The gated clippy lane has two levels rather than three, because a `warn` in the lints table combined with `--deny warnings` is a hard error.
The panic cluster is not closed by any lint set, so the substituted-default class is controlled by full review of the zero-debt core and by property tests with independent oracles, or not at all.
A misspelt lint name, a renamed one, and a typo in a `disallowed-methods` or `disallowed-types` path all leave the build green, and a ban path is validated only in compilation units that reference the crate it names, so the rules in this charter are only as real as the scan that catches those failures and the probe crate that puts every ban path in front of it — with the further measured exception that a primitive-type path such as `str::split_at` produces no reachability diagnostic at all, so those entries are armed by a call site in the probe crate rather than by the scan.
And a crate-local `clippy.toml` replaces the workspace-root file rather than merging with it, which is why there is exactly one such file in the tree and why per-crate lint enforcement lives in a crate-root attribute instead.

Sections 9 through 18 continue in [`operational-charter.md`](operational-charter.md).
