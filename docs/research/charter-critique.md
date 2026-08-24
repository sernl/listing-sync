# Adversarial critique of the engineering charter

Produced by the charter:critique agent, 2026-08-24.
It verified its claims by compiling; treat its factual findings as correct unless disproved by execution.

---

## Verdict

The scholarship is clean and the section 4 rejection table is the best thing in the document. The artefact it produces does not work. I compiled the charter's own configuration on the exact toolchain it names (rustc 1.97.1, clippy 0.1.97): the flagship `limits.rs` does not compile, it violates the charter's own lint table twelve times, and every panic-workaround an agent will reach for passes the full lint set with zero diagnostics. A charter whose primary artefact was never run is a charter nobody will follow past week three.

## Prioritised changes

### Tier 1 — the artefact is broken as written

1. Fix `crates/limits`: `Tier` is never defined or imported, so the section 6 code block fails with four `E0433: cannot find type Tier in this scope` errors — the document's stated primary artefact has never been compiled.
2. Delete the three tautological const assertions (`API_REQUESTS_PER_MINUTE.len() == Tier::COUNT` and its two siblings): an array declared `[NonZeroU32; Tier::COUNT]` cannot have a different length, so these are exactly the vacuous assertions section 4 condemns when rejecting the density metric, sitting in the charter's own showcase.
3. Reconcile `missing_assert_message` with `const { assert!(...) }`: the eight const assertions in section 6 each fire `clippy::missing_assert_message`, which the charter denies workspace-wide, so the adopted rule and the showcase are mutually exclusive as written.
4. Remove the eight `use super::*;` lines from `limits.rs` or stop denying `pedantic`: `clippy::wildcard_imports` fires on three of them, plus one `duration_suboptimal_units` on `Duration::from_millis(500)` — the charter breaks its own gate in a fifty-line file.
5. Drop `--deny warnings` from `cargoClippyExtraArgs` or stop pretending `nursery` and `cargo` are advisory: I measured a one-line crate producing seven hard errors under the charter's exact crane wiring (six `cargo_common_metadata` demanding `description`, `license`, `repository`, `readme`, `keywords`, `categories` on unpublished proprietary crates, plus one `nursery` hit), which nullifies the charter's own argument that denying `nursery` "teaches the founder to bypass the gate."
6. Weaken `#![forbid(unsafe_code)]` to `deny` in the Leptos hydrate crate and any Tauri mobile crate: I verified `forbid` rejects macro-generated `unsafe`, and `#[wasm_bindgen]` expansion contains it, so "the single highest-confidence line in this charter" is the line most likely to be ripped out at 2am — the exact failure mode the charter chose `forbid` to prevent.
7. Delete `allow-unwrap-in-consts = true` and `future-size-threshold = 16384` and wire `excessive-nesting-threshold` to an enabled lint: the first two are the upstream defaults, and `clippy::excessive_nesting` is never enabled anywhere in the table, so all three lines are decoration in a document that argues rules must be mechanically checked.

### Tier 2 — velocity, pre-PMF

8. Move "is any of this permitted by the marketplace's terms" from question one of seven on the last page to a gate in section 1: an 860-line engineering charter whose product premise may be unlawful has its dependencies inverted.
9. Cut the "day one of the production build" list to what is genuinely expensive to retrofit — schema plus `TenantId` on every repository method, the `AdapterError` taxonomy including `Ambiguous`, `IdempotencyKey` on publish, release `overflow-checks`, explicit timeouts on every external call, bounded channels, `CatchPanicLayer` plus `spawn_supervised`, secret types, database backups, licence allow-list — because everything else in section 8 is a Tuesday afternoon at any point in the next year.
10. Defer explicitly: `pedantic` at deny, `too_many_lines`, `indexing_slicing`, `[bans]`/`[sources]`/`[bans.build]`, `cargo-machete`, `cargoDoc` with `RUSTDOCFLAGS=--deny warnings`, `missing_docs`, OpenAPI plus `oasdiff`, `cargo-hack` matrices, mutation testing, fuzzing, Kani, and the simulator — that is most of section 8 and all of section 7 below the acceptance row, and none of it gets harder by waiting.
11. Ship `crates/limits` with about eight constants rather than sixty: the charter marks three as uncalibrated and then writes fifty-seven more with no measurements behind them, which is a maintenance surface masquerading as discipline.
12. Add a stage between "the spike" and "the production build" — the first paying deployment, which gets item 9 and nothing else — because pre-PMF a charter's job is to stop you writing code you will rewrite, not to make throwaway code pretty.
13. Delete the `trybuild` compile-fail test from day one and keep only the two-tenant integration test in the same paragraph: `trybuild` output is brittle across toolchain bumps and the integration test gets most of the value for a fraction of the upkeep.

### Tier 3 — over-application

14. Move `clippy::pedantic` to warn, or deny it with `missing_errors_doc`, `missing_panics_doc`, `doc_markdown`, `module_name_repetitions`, and `wildcard_imports` allowed: in a railway-oriented codebase where nearly every function returns `Result`, `missing_errors_doc` alone demands a doc paragraph per function for zero defect yield.
15. Make `too_many_lines` warn permanently rather than running the proposed one-month deny trial: a 70-line ceiling is a ceiling on the exact centralize-control-flow shape the charter adopts three rules earlier, and the charter already records one lane flagging it as undecided.
16. Drop `indexing_slicing` from deny: in C an out-of-bounds index is memory corruption, in Rust it is a bounds-checked panic that `CatchPanicLayer` and `spawn_supervised` already contain, and its measured effect here is negative (see item 21).
17. Replace "fuzz the three untrusted parsers" with running ingestion out-of-process under memory, CPU, and wall-clock rlimits: you will depend on a ZIP/OOXML/PDF parser rather than write one, so fuzzing produces findings you cannot fix, while a reaped subprocess discharges the zip-bomb and decompression-ratio limits at the OS layer where they actually hold.
18. Cut `[bans.build] executables/interpreted = "deny"`: on a graph containing browser automation, Tauri, and Leptos this produces a wall of day-one findings, and the stated justification is not what those keys do.
19. Narrow `#[serde(try_from = "RawX")]` to types crossing the trust membrane and delete the CI grep for `#[derive(Deserialize)]`: the grep cannot see multi-line derives, `serde::Deserialize` paths, or `#[derive(sqlx::FromRow)]`, which is the actual database path it was meant to guard.
20. Reframe "review the assertions and type signatures rather than the bodies" as narrowing review rather than replacing it: an agent that writes both the code and its assertions writes assertions that agree with the code, so this is the most consequential unevidenced claim in the document and the mechanism by which a plausible wrong feature ships unreviewed.

### Tier 4 — agent reality

21. Add `clippy::unreachable`, `clippy::integer_division`, and `clippy::string_slice` to the deny list and state plainly that the panic cluster is not closed: I wrote six agent-natural workarounds under the charter's full table — `unreachable!("caller guarantees Some")`, `o.unwrap_or_default()`, `s.get(i).copied().unwrap_or_default()`, `a / b`, `s.split_at(2)`, and `assert!(o.is_some()); o.unwrap_or_default()` — and every one produced zero lints, so denying `unwrap_used`/`expect_used`/`panic` currently converts loud contained panics into silent wrong numbers in a system doing price and quota arithmetic.
22. State in the charter that no lint catches "panic replaced by a silent default", and that the only control is full review of the zero-debt core plus property tests: pretending the lint table covers it is how the founder stops reading the diff.
23. Enumerate `tokio::task::spawn_blocking`, `JoinSet::spawn`, `Handle::spawn`, and `spawn_local` alongside `tokio::spawn` in `disallowed-methods`, or the supervised-spawn rule is decorative (unverified locally — no tokio available offline, but the path-matching is per-method).
24. Add a CI ratchet on `#[expect(...)]` count per crate: `reason` strings are unchecked text and an agent will write "complex orchestration logic" and move on, so the count is the only mechanically-checkable signal, and this is the charter's own advice about `too_many_lines` generalised.
25. Extend the gated-shared-state rule beyond the three lint files to `Cargo.toml` dependency additions and `crates/limits`: an agent hitting a wall raises a limit or adds a crate for exactly the reason it edits the lint table.

### Tier 5 — gaps neither source covers

26. Add a secret-handling section: this system holds marketplace seller credentials, Stripe keys, and LLM keys, and the charter mentions none of it — the machine-checkable form is a `Secret<T>` newtype with a redacting `Debug`, `zeroize` on drop, a `disallowed-types` entry on the raw string types, and encryption at rest with a rotation story.
27. Add backup, restore, and a restore drill: one self-hosted box, no failover, paying multi-tenant customers, and no mention of RPO, RTO, or a tested restore is a larger risk than every lint in section 8 combined.
28. Add schema and data migration discipline: the charter declares migrations zero-debt and then says nothing about expand/contract, forward-only versus reversible, or testing migrations against a production-shaped snapshot, while its own section 5 worked example depends on "a row written by an older or newer deployment" being a live case.
29. Add observability beyond the panic counter: span and field conventions, correlation-id propagation from request into worker, an explicit log-redaction rule for credentials and tenant PII, health and readiness endpoints, and a per-adapter success-rate metric — the last being the only thing that tells the founder Tes changed their form before customers do.
30. Add a per-marketplace kill switch and a scheduled synthetic canary: `SelectorMissing → needs_human` handles one job, nothing handles the fleet case where every job fails at once, and a runtime flag that drains and pauses one adapter is cheap precisely because the charter's premise is that you can redeploy in minutes.
31. Add inbound abuse handling: `tier::API_REQUESTS_PER_MINUTE` is a constant with no named enforcement point, and with a per-tenant LLM spend cap and no authn/authz model, upload magic-byte sniffing, SSRF policy for user-supplied URLs, or signup abuse control, a weakly-guarded endpoint is a direct financial exploit.
32. Add a prompt-injection rule: attacker-supplied PDFs reach a model whose output reaches a live marketplace listing on a paying seller's storefront, which is the one genuinely novel trust boundary in the system, and section 9 raises it as a question rather than answering it.
33. Add deployment and rollback: the entire velocity argument rests on "redeployable in minutes", and the document covers Nix builds while saying nothing about how a deploy happens, how it rolls back, or how a bad migration is backed out.
34. Add data-protection constraints: UK/EU obligations over teacher and pupil-adjacent material, deletion and export duties, and a DPA with the LLM provider constrain whether tenant content may reach a third-party model at all, which shapes the architecture more than any lint here.

### Tier 6 — house style and hygiene

35. Split the file: at 859 lines it exceeds the 800-line threshold, and section 8 is the part edited most and read least, so it belongs in a sibling `enforcement-toolchain.md`.
36. Unbold the four worked-example labels at lines 272, 280, 287, 295: house style avoids bolded section labels, and these are labels.
37. Break lines 57-59 and 243-244 to one sentence per line.
38. Replace upstream line-number citations with section plus quoted phrase: your own clone drift already shows the rot — `§Naming L292` should be L287, and `functions run to completion without suspending` is L431-433, not the cited L433-435.
39. Correct the overflow repro: `250u8 + 10` is a deny-by-default `arithmetic_overflow` compile error in both profiles, not a dev-panic and release-wrap, so the sentence describes an experiment that cannot have been run as written — the substantive claim is right and I confirmed it with runtime values (release yields 4, `overflow-checks=on` panics).
40. Add crane's second audit trap to the section 8 trap list: `cargoAudit` defaults `cargoAuditExtraArgs ? "--ignore yanked"` at the very revision you cite, so the yanked signal is off in the audit lane as well as needing `yanked = "deny"` in `deny.toml`.

## Where the charter is right

The source scholarship is accurate and I verified it rather than assuming it. Every Power of Ten quotation is verbatim against `spinroot.com/gerard/pdf/P10.pdf`, including Rule 9's function-pointer rationale, Rule 5's anti-vacuity clause, Rule 2's non-terminating carve-out, Rule 1's acyclic call graph, and Rule 10's "daily" — the charter even reproduces Holzmann's own "anomolous" typo. Every TIGER_STYLE line range lands inside the correct section of the file at HEAD and the quoted text matches. Every version number checks out on crates.io: cargo-mutants 27.1.0, cargo-machete 0.9.2, utoipa 5.5.0, cargo-deny 0.20.2, turmoil 0.7.2, madsim 0.2.34, serde_json 1.0.151, tokio 1.53.1. The tokio `JoinHandle` claim is correct — it carries no `must_use` at 1.53.1. The crane traps are correct and I confirmed them at the exact revision cited (692f7e9): `cargoDenyChecks ? "bans licenses sources"` does omit advisories, and `cargoAudit` does run `cargo audit -n -d ${advisory-db}` with null artifacts. `clippy::lint_groups_priority` is a hard deny-by-default error, as stated. Every lint name in section 8 exists and every clippy.toml key parses on 1.97.1.

Substantively right and worth keeping unchanged: the rejection of the assertion-density metric in favour of `cargo-mutants`; `AdapterError::Ambiguous` as a distinct variant with `IdempotencyKey` on publish, which is the single most valuable design decision in the document; `TenantId` as a mandatory positional parameter rather than task-local state; the sans-IO seam with tokio, reqwest, and sqlx banned from its tree; the per-tier panic policy and the refusal of `panic = "abort"`; `overflow-checks` in release; `#[expect]` over `#[allow]`, which I verified self-cleans and hard-fails when stale; and treating the three lint files as founder-gated shared state, which is the best agent-specific idea here. Section 4 is the load-bearing half, as the charter itself claims, and it should be the template for the missing sections rather than the lint table.

Working files: `/tmp/claude/claude-1000/-home-sernl-projects/7acda150-a31b-4ba6-9a45-cbb73bfa0e51/scratchpad/lintcheck/` (lint reproductions), `/tmp/claude/claude-1000/-home-sernl-projects/7acda150-a31b-4ba6-9a45-cbb73bfa0e51/scratchpad/ovf/` (overflow and `forbid` reproductions), `/tmp/claude/claude-1000/-home-sernl-projects/7acda150-a31b-4ba6-9a45-cbb73bfa0e51/scratchpad/p10.txt` (extracted Power of Ten text).

Sources: [rust-clippy lint configuration](https://doc.rust-lang.org/clippy/lint_configuration.html), [tokio JoinHandle](https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html), [The Power of Ten](https://spinroot.com/gerard/pdf/P10.pdf)