# Enforcement toolchain

This file is the primary artefact of the engineering charter, which links here rather than inlining it.
The charter itself is two files: [`engineering-charter.md`](engineering-charter.md) carries sections 1 through 8 and [`operational-charter.md`](operational-charter.md) carries sections 9 through 18, so a charter section number cited below resolves in one or the other.
This file is a third sibling because it is the part edited most and read least, and because every file in this set must stay under the file-length threshold.
Everything named was verified present on rustc 1.97.1, cargo 1.97.0 and clippy 0.1.97 unless explicitly marked otherwise.
The blocks below are the artefact rather than a description of one: the `[workspace]` stanza, the `[workspace.lints]` table, the `clippy.toml`, the member manifest, the `[profile.release]` block, `crates/ban-probe` and the whole of `crates/limits` were extracted verbatim from this file into one workspace at `/tmp/claude/claude-1000/-home-sernl-projects/7acda150-a31b-4ba6-9a45-cbb73bfa0e51/scratchpad/correct1`, which passes `cargo clippy --all-targets -- --deny warnings`, `cargo fmt --check` and `cargo test` clean at five tests.
Nothing here is excerpted or paraphrased, so that workspace is rebuildable from this file alone once the `members` list is filled in for the project at hand, and the claims a dependency-free workspace cannot exercise were measured on the separate four-crate graph named in the cargo-deny section and on the ban-resolution workspaces at `/tmp/claude/claude-1000/-home-sernl-projects/7acda150-a31b-4ba6-9a45-cbb73bfa0e51/scratchpad/repair2`.
The `deny.toml` below passed `cargo-deny 0.20.2 check bans licenses sources` when it was written; that tool is not present on the machine this revision was checked on, so its measurements are carried forward rather than re-run, and every one of them is marked where it appears.

## The gated lane has two levels, not three

One measured fact governs the shape of everything below.
Cargo translates `[workspace.lints]` entries into rustc command-line flags rather than into crate attributes, and the flake check appends `-- --deny warnings` afterwards, so the later flag wins.
Measured on 1.97.1: `too_many_lines = "warn"` in the table plus `--deny warnings` produced `error: this function has too many lines (91/70)` and a failed build, while the same tree without `--deny warnings` produced a warning and exit 0.

So in the gated lane there is no `warn`.
The table has two levels, `allow` and fatal, and every `warn` in an earlier draft was a `deny` wearing a softer word.
This voids the charter's own argument for keeping `nursery` at warn, because at warn it broke the build anyway, and it means an instruction to move `pedantic` or `too_many_lines` to warn cannot be implemented in the table at all.

Anything genuinely advisory therefore needs a second lane, and a second lane works.
Measured: with `too_many_lines = "allow"` in the table, a trailing `-W clippy::too_many_lines` still fires, because both are command-line flags and the later one wins.

```
# gated, in nix flake check
cargo clippy --all-targets -- --deny warnings

# advisory, reported and not gated
cargo clippy --all-targets -- \
  -W clippy::nursery -W clippy::cargo \
  -W clippy::too_many_lines -W clippy::cognitive_complexity -W clippy::type_complexity
```

Verified end to end on the workspace this file publishes: the gated lane exits 0 with no diagnostics, and the advisory lane over the same tree exits 0 as well while reporting ten hits from two lints.
Seven are `clippy::too_long_first_doc_paragraph` from `nursery`, all in `crates/limits`, and three are `clippy::multiple_crate_versions` from `cargo`, naming `getrandom` at 0.2.17 and 0.4.3, `syn` at 2.0.119 and 3.0.4, and `windows-sys` at 0.52.0 and 0.61.2.
An earlier draft reported only the first group and so described a run cleaner than the one that happens; both groups are the split working as intended rather than defects to fix, since the long first paragraphs are the provenance markers the charter requires, and the duplicate versions arrive through the probe crate's dependency graph and cannot be resolved from this workspace, which is exactly why `cargo` lives in the advisory lane rather than the gated one.

## The workspace stanza

The stanza has three lines and an earlier draft called it two, which cost a warning on every cargo invocation in the workspace.

```toml
[workspace]
members = ["crates/limits", "crates/ban-probe"]
resolver = "2"
```

Measured on 1.97.1: with the third line deleted, `cargo clippy` printed `warning: virtual workspace defaulting to 'resolver = "1"' despite one or more workspace members being on edition 2021 which implies 'resolver = "2"'`, followed by three notes, before every other line of output.
It is a cargo warning rather than a rustc one, so `--deny warnings` does not promote it and the build still succeeds, which is exactly the shape of failure this file exists to catch: a permanent line of noise at the head of every run, in the lane whose whole discipline is that its output is empty.
A virtual workspace has no package to inherit an edition from, so the resolver has to be stated, and stating it is also simply correct, since resolver 1 unifies features across build-dependencies and target-specific dependencies in a way edition 2021 does not intend.

## Workspace lints

One table at the workspace root, with `[lints] workspace = true` in every member.
The `priority = -1` field is mandatory wherever a group and one of its members both appear: a group at the same priority as an individual lint is a hard cargo error (`clippy::lint_groups_priority`), and a lower priority makes the group apply first so the individual override wins.

```toml
[workspace.lints.rust]
unsafe_code                = "forbid"
rust_2018_idioms           = { level = "deny", priority = -1 }
unused_must_use            = "deny"
let_underscore_drop        = "deny"
unreachable_pub            = "deny"
trivial_casts              = "deny"
elided_lifetimes_in_paths  = "deny"
unused_lifetimes           = "deny"

[workspace.lints.clippy]
all      = { level = "deny", priority = -1 }
pedantic = { level = "deny", priority = -1 }

# Pedantic exceptions. In a railway-oriented codebase where nearly every
# function returns Result, the doc lints demand a paragraph per function for
# no defect yield. `wildcard_imports` is deliberately NOT here: it is the one
# pedantic lint whose purpose is making a name's provenance visible in the
# file that uses it, which is worth more to an agent than to a human.
missing_errors_doc          = "allow"
missing_panics_doc          = "allow"
doc_markdown                = "allow"
module_name_repetitions     = "allow"

# Hand-picked from `restriction`; never enable that group wholesale.
unwrap_used                 = "deny"
expect_used                 = "deny"
panic                       = "deny"
todo                        = "deny"
unimplemented               = "deny"
unreachable                 = "deny"
integer_division            = "deny"
string_slice                = "deny"
dbg_macro                   = "deny"
mem_forget                  = "deny"
exit                        = "deny"
infinite_loop               = "deny"
missing_assert_message      = "deny"

# Async and concurrency hazards.
let_underscore_must_use     = "deny"
let_underscore_future       = "deny"
await_holding_lock          = "deny"
await_holding_refcell_ref   = "deny"
async_yields_async          = "deny"
significant_drop_tightening = "deny"
unused_async                = "deny"
large_futures               = "deny"

# Structure and exhaustiveness.
wildcard_enum_match_arm     = "deny"
missing_asserts_for_indexing = "deny"
disallowed_methods          = "deny"
disallowed_types            = "deny"
disallowed_macros           = "deny"

# Advisory: allowed here, raised in the second clippy invocation.
too_many_lines              = "allow"
type_complexity             = "allow"
cognitive_complexity        = "allow"

# Deliberate allows, argued in charter section 4.
must_use_candidate          = "allow"
panic_in_result_fn          = "allow"
indexing_slicing            = "allow"
```

Four changes from the earlier draft need their reasons recorded.

`clippy::unreachable`, `clippy::integer_division` and `clippy::string_slice` are new, and the panic-cluster subsection below explains why they are necessary and why they are not sufficient.

`too_many_lines` is advisory permanently rather than after a one-month trial.
A 70-line ceiling is a ceiling on the exact centralize-control-flow shape the charter adopts, and an agent under a hard limit extracts arbitrary fragments to satisfy the counter rather than finding the natural seam.
The threshold stays at 70 in `clippy.toml` because the advisory lane still needs a number; what changes is that exceeding it prompts review rather than blocking a build.

`indexing_slicing` is dropped from deny.
In C an out-of-bounds index is memory corruption; in Rust it is a bounds-checked panic that `CatchPanicLayer` and `spawn_supervised` already contain, so its blast radius is already governed by the per-tier panic policy.
Its measured effect under this charter was negative: denying it pushes an agent toward `s.get(i).copied().unwrap_or_default()`, which produces a silent wrong number where the index would have produced a loud contained panic.

`nursery` and `cargo` are gone from the table entirely and live in the advisory lane, for the reason given above.

Three lints stay scoped per crate rather than workspace-wide, and the mechanism is a crate-root attribute rather than a second manifest table, for the reason recorded under crate-scoped configuration below.
`arithmetic_side_effects` — note the name, since `clippy::integer_arithmetic` was renamed and citing the old name is not an error but a warning the build exits 0 on, measured in the silent-failure subsection below — is denied at the crate root of the billing, quota-accounting and taxonomy crates only, where a wrong number is a commercial incident.
`as_conversions` is denied at the root of the domain crates, forcing `TryFrom` with a handled failure.
`print_stdout` and `print_stderr` are denied at the root of the axum, service and adapter crates and left allowed in the `xtask` helper and in build scripts.

The only sanctioned escape hatch is `#[expect(lint, reason = "...")]`, never `#[allow]`, and verified: a stale `#[expect]` produces `warning: this lint expectation is unfulfilled`, which under `--deny warnings` is a build failure, so suppressions self-clean rather than accumulating.

## Unpublished crates and the cargo group

The earlier draft's `cargo = { level = "warn", priority = -1 }` combined with `--deny warnings` made a one-line crate fail with six `cargo_common_metadata` errors demanding `description`, `license`, `repository`, `readme`, `keywords` and `categories` on crates that are proprietary and will never be published.

The fix is neither dropping `--deny warnings` nor allowing the lint.
Measured: adding `publish = false` to the member manifest silences all six, because the lint does not ask a crate that declares itself unpublishable for publication metadata.
That is also simply true, which an `allow` would not have been, and it self-corrects: if a crate is ever published the lint returns, which is the behaviour wanted.
`multiple_crate_versions` is the other `cargo`-group lint, it fires on any real dependency graph, it cannot be fixed from this workspace, and it is handled by the group living in the advisory lane, where the top of this file records the three hits it currently produces.

Each member manifest is therefore part of the artefact rather than boilerplate:

```toml
[package]
name    = "limits"
version = "0.1.0"
edition = "2021"
publish = false

[lints]
workspace = true
```

## clippy.toml

Three keys from the earlier draft are deleted as decoration in a document that argues rules must be mechanically checked.
`allow-unwrap-in-consts = true` and `future-size-threshold = 16384` are the upstream defaults, verified by toggling: with `allow-unwrap-in-consts = false` the limits crate produced five `unwrap_used` errors, one for each const `unwrap`, and with the key absent it produced zero.
`excessive-nesting-threshold = 5` configured `clippy::excessive_nesting`, which the table never enables.

```toml
too-many-lines-threshold       = 70
too-many-arguments-threshold   = 5
cognitive-complexity-threshold = 30

allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests  = true
allow-print-in-tests  = true
allow-dbg-in-tests    = true

disallowed-methods = [
  { path = "tokio::spawn",                            reason = "use spawn_supervised; a dropped JoinHandle discards the panic" },
  { path = "tokio::task::spawn",                      reason = "use spawn_supervised" },
  { path = "tokio::task::spawn_blocking",             reason = "use spawn_supervised_blocking" },
  { path = "tokio::task::spawn_local",                reason = "use spawn_supervised" },
  { path = "tokio::task::LocalSet::spawn_local",      reason = "use spawn_supervised" },
  { path = "tokio::runtime::Handle::spawn",           reason = "use spawn_supervised" },
  { path = "tokio::runtime::Handle::spawn_blocking",  reason = "use spawn_supervised_blocking" },
  { path = "tokio::runtime::Runtime::spawn",          reason = "use spawn_supervised" },
  { path = "tokio::runtime::Runtime::spawn_blocking", reason = "use spawn_supervised_blocking" },
  { path = "tokio::task::JoinSet::spawn",             reason = "route through spawn_supervised so the panic counter sees it" },
  { path = "tokio::task::JoinSet::spawn_blocking",    reason = "route through spawn_supervised_blocking" },
  { path = "tokio::task::JoinSet::spawn_local",       reason = "route through spawn_supervised" },
  { path = "tokio::sync::mpsc::unbounded_channel",    reason = "bound it: see crates/limits" },
  { path = "futures::channel::mpsc::unbounded",       reason = "bound it: see crates/limits" },
  { path = "reqwest::Client::new",                    reason = "no default timeout; build it with .timeout() and .connect_timeout()" },
  { path = "reqwest::get",                            reason = "no default timeout; use a configured client" },
  { path = "std::fs::read",                           reason = "stream it; uploads are bounded but not small" },
  { path = "std::fs::read_to_string",                 reason = "stream it; uploads are bounded but not small" },
  { path = "tokio::io::AsyncReadExt::read_to_end",    reason = "stream it under a byte cap" },
  { path = "str::split_at",                           reason = "panics off a char boundary; use split_at_checked" },
  { path = "str::split_at_mut",                       reason = "panics off a char boundary; use split_at_mut_checked" },
  { path = "serde_json::Deserializer::disable_recursion_limit", allow-invalid = true, reason = "the 128 default is the depth bound; exists only under unbounded_depth, which deny.toml denies" },
  { path = "std::env::var",                           reason = "configuration is read in one crate; expect-attribute the site that legitimately does" },
  { path = "std::env::vars",                          reason = "configuration is read in one crate; expect-attribute the site that legitimately does" },
  { path = "std::time::SystemTime::now",              reason = "time enters the engine as data; expect-attribute the driver that reads the clock" },
  { path = "std::time::Instant::now",                 reason = "time enters the engine as data; expect-attribute the driver that reads the clock" },
  { path = "std::panic::catch_unwind",                reason = "only the worker boundary may contain a panic" },
]

disallowed-types = [
  { path = "std::sync::Mutex", reason = "poisoning turns one unwound request into a cascading outage; use tokio::sync::Mutex" },
  { path = "std::any::Any",    reason = "downcasting defeats both static analysis and an agent tracing control flow" },
]
```

Every spawn path above was read out of the tokio 1.53.1 source rather than recalled: `src/task/spawn.rs`, `src/task/blocking.rs`, `src/task/local.rs`, `src/task/join_set.rs`, `src/runtime/handle.rs` and `src/runtime/runtime.rs`.
`JoinSet` is banned alongside the detaching entry points even though it does not detach, because the point of routing everything through one helper is that the panic counter and the `needs_human` transition have exactly one place to live, and `spawn_supervised` is implemented over `JoinSet` and carries its own `#[expect(clippy::disallowed_methods)]`, which makes the exemption greppable.
The tokio 1.53.1 source also confirms the charter's `JoinHandle` claim: the only `must_use` in `src/runtime/task/join.rs` is on `abort_handle()`, and the type's own documentation states that a `JoinHandle` detaches its task when dropped.

One entry carries a flag the others do not, and the reason generalises to any ban on a feature-gated item: `serde_json::Deserializer::disable_recursion_limit` exists only when serde_json is built with its `unbounded_depth` feature, which the 1.0.151 source puts behind `#[cfg(feature = "unbounded_depth")]` at `src/de.rs:213`.
Under default features the path does not resolve, and clippy 0.1.97 reports `serde_json::Deserializer::disable_recursion_limit does not refer to a reachable function`, which is one of the strings the scan below is required to fail on, so the entry as first written red-built this file's own gate in any project taking serde_json at its defaults.
`allow-invalid = true`, which clippy's own help text suggests, settles that without disarming anything: measured with the feature off the run is clean at exit 0, measured with the feature on a call to the method still produces `use of a disallowed method`, so the entry is armed exactly when the method exists and the control that keeps it from existing is one layer down, in `deny.toml`.
That entry also has to stay on one line, because a TOML inline table cannot span lines and clippy rejects the file outright when one does, which is at least the loud failure rather than the silent one.

## A crate-local clippy.toml replaces the workspace one

An earlier draft named four crate-scoped `clippy.toml` additions and closed with the caveat that whether nested resolution behaves as expected in this workspace layout was not verified.
It has now been verified, and it does not merge: a crate-local `clippy.toml` replaces the workspace-root file outright.

Measured on 1.97.1 against the workspace this file publishes.
With no crate-local file, a member calling `reqwest::Client::new`, `tokio::sync::mpsc::unbounded_channel` and `str::split_at` produced three `use of a disallowed method` errors and the run exited 101.
Adding a `clippy.toml` in that member's own directory containing nothing but one `disallowed-types` entry for `std::collections::HashMap` produced zero diagnostics of any kind and the run exited 0.
The same file also drops the `allow-*-in-tests` keys: an `unwrap()` inside a `#[test]` was clean under the root file alone and produced `clippy::unwrap_used` as soon as the crate-local file existed.

So the passage that described crate-scoped additions was not merely unverified, it was inverted.
Following it would have deleted, in every crate that gained its own file, all twenty-seven `disallowed-methods` entries, both `disallowed-types` entries, the three threshold keys and the five `allow-*-in-tests` keys, in exchange for the one entry that file names — and it would have done so at exit 0 with no diagnostic, which is this file's own definition of a rule that has quietly stopped existing.

### The design that works

There is exactly one `clippy.toml` in the tree, at the workspace root, and a flake check asserts it rather than trusting it: the check is a file count rather than a lint, because the failure it guards produces no diagnostic, so `fd -HI -g clippy.toml --exclude target` must print exactly one path.
Per-crate enforcement then splits by what kind of thing is being scoped, and only one of the two kinds can be scoped at all.

A lint level is per-crate through a crate-root attribute.
It is not per-crate through a second manifest table: a member cannot carry both `[lints] workspace = true` and its own `[lints.clippy]` entries, and cargo refuses the manifest outright rather than resolving a precedence, with `cannot override 'workspace.lints' in 'lints', either remove the overrides or 'lints.workspace = true' and manually specify the lints`.
Dropping `workspace = true` and restating the whole table in the member is the same replacement hazard in different syntax, so the mechanism is `#![deny(clippy::as_conversions)]` at the crate root, which composes with the inherited table rather than replacing it: measured, a widening cast in a member carrying that attribute produced `clippy::as_conversions` with `note: the lint level is defined here` pointing at the attribute, while every workspace-level lint stayed in force.
An `xtask` step holds a committed table of crate-and-attribute pairs and fails when one is absent, so an attribute deleted during a refactor is a build failure rather than a silent relaxation, which is the property the founder-gated path gives the workspace file and which a crate's own source would otherwise lack.

A ban-list entry has no per-crate form at all, and pretending otherwise is what produced the defect above, so each formerly crate-scoped entry is dispositioned rather than relocated.
Four move into the one root list because they are correct everywhere, with `#[expect(clippy::disallowed_methods, reason = "...")]` at the sites that legitimately need them, which turns each exemption into a greppable line and puts it on the expect ratchet: `std::env::var` and `std::env::vars`, so configuration is read in one place, and `std::time::SystemTime::now` and `std::time::Instant::now`, so every ambient clock read in the workspace is an inventory rather than an assumption — which is what `sync-core`'s crate-scoped clock ban was for, achieved without a second file.

The rest get a named replacement rather than a pretence.
`uuid::Uuid::new_v4` and the ambient `rand` entry point were already silent in `sync-core`, because they resolve only if that crate depends on those crates and the purity rule forbids exactly that, so the dependency-closure walk in the rules file's section 7 is and always was the control.
`std::collections::HashMap` and `HashSet` in `sync-core` are covered by the test that runs one seed twice in a single process and asserts byte-identical effect traces, which catches an iteration-order leak rather than making one less likely.
`std::string::String` in the secrets and telemetry-payload crates is carried by those crates' public APIs, since `Secret<P>` has no constructor taking a `String` and the telemetry payload is one struct of closed types with no conversion from one.
`core::result::Result::ok` and `core::option::Option::unwrap_or_default` in the billing and adapter crates are demoted rather than replaced, and that is the honest outcome: they belong to the substituted-default class, and the subsection below already states that no finite list of method paths closes it and that full review of the zero-debt core and property tests with independent oracles are its only controls.

## Silent failure, and the channels that carry it

One class of trap runs through this whole file, and every member of it fails the same way: the rule stops existing and the build stays green.
Four channels were measured on 1.97.1 under the gated invocation, and all four exited 0.
A `disallowed-methods` entry naming a crate absent from the graph produced no diagnostic at all; an entry naming a crate present in the graph but unreferenced by the compilation unit under check produced none either; a typo in an item under a path that unit does resolve produced a warning reading `std::fs::read_to_stringgg does not refer to a reachable function`; and the same typo in a `disallowed-types` path produced the same sentence ending in `reachable type`, which the earlier single-string control did not cover.

The second of those is the one an earlier draft understated, and its consequence is structural rather than incidental: clippy resolves a ban path against the crates the compilation unit under check actually loads, not against the workspace's resolved dependency graph, so a `std::` entry is validated everywhere while a `tokio::` entry is validated only where some file in that crate references tokio.
Measured on a two-member workspace in which both members declare serde_json and only one references it: the serde_json entry produced its diagnostic once, attributed to the referencing member and not to the other, and a deliberately misspelt `tokio::spawnnn` entry stayed silent in a member that declared tokio until a `use` of the crate was added, at which point the warning appeared.
Feature selection narrows the window further, on a tree where no other dependency unified those features in: with tokio taken at `rt` and `macros` only, the correct entries `tokio::sync::mpsc::unbounded_channel` and `tokio::io::AsyncReadExt::read_to_end` stopped resolving and produced that same reachability warning, because those items sit behind the `sync` and `io-util` features.
Ban-list validation coverage is therefore a function of which crates and features each member happens to use, and a green scan says that every entry resolvable in some compilation unit resolved there rather than that every entry is live.

A fifth channel was found for this revision, and it is worse than the other four because the diagnostic that covers them does not exist for it.
`str::split_at` and `str::split_at_mut` are paths on a primitive type, and clippy emits no reachability warning for a primitive-type path whether it resolves or not.
Measured: misspelling both, as `str::split_att` and `str::split_at_muttt`, produced zero diagnostics and exit 0, while the same file misspelling `std::fs::read_to_string` in the same run produced `std::fs::read_to_stringgg does not refer to a reachable function`.
The two entries do work when spelled correctly, since a call to each produced its own `use of a disallowed method` error, so this is a silent-disarm channel rather than a dead entry.

For those two entries, therefore, the claim that the reachability-warning channel is what keeps the ban list from going quietly dead is false, and it matters out of proportion to its size, because the panic-cluster subsection below makes `disallowed-methods` the only control on both methods: a typo in either removes the control outright and the run reports green.

So the workspace ban list gets a probe crate rather than trust, and the crate does two jobs rather than one: it puts every crate the ban list names in front of clippy, converting the unreferenced-crate channel into one that produces output, and it arms by call site the entries no channel reports on.

```toml
[package]
name    = "ban-probe"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
tokio      = { version = "1.53.1", features = ["rt", "sync", "io-util"] }
futures    = "0.3"
reqwest    = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
serde_json = "1.0.151"

[lints]
workspace = true
```

```rust
//! Puts every crate the workspace ban list names in front of clippy, and arms
//! by call site the two entries clippy cannot report on.

pub use futures as _futures;
pub use reqwest as _reqwest;
pub use serde_json as _serde_json;
pub use tokio as _tokio;

/// Arms the ban on `str::split_at`.
///
/// Clippy emits no reachability diagnostic for a primitive-type path, so a typo
/// in that `disallowed-methods` entry disarms it in silence. Calling the method
/// under an `expect` attribute inverts that: if the entry stops resolving, the
/// expectation goes unfulfilled and the gated lane fails.
#[expect(clippy::disallowed_methods, reason = "the call site is the probe")]
pub const fn split_at_ban_is_armed(s: &str) -> (&str, &str) {
    s.split_at(0)
}

/// Arms the ban on `str::split_at_mut`, by the mechanism above.
#[expect(clippy::disallowed_methods, reason = "the call site is the probe")]
pub const fn split_at_mut_ban_is_armed(s: &mut str) -> (&mut str, &mut str) {
    s.split_at_mut(0)
}
```

The crate is an ordinary workspace member, so the gated lane compiles it and the scan reads its output along with everything else, and its manifest names exactly the features the banned items sit behind: `rt` for the spawn entries, `sync` for `unbounded_channel`, and `io-util` for `read_to_end`.
An earlier draft also listed `net`, `time`, `fs` and `macros`, which no banned path sits behind at all, and naming only the three that remain is insurance rather than today's arming, since reqwest already unifies `sync` and `io-util` into this graph and its feature list is not this file's to control.
Measured: this file's `clippy.toml` against that crate produced zero reachability warnings, misspelling one path per crate produced four distinct ones, and the crate passed the gated invocation clean under the full lints table above.
Every crate the workspace list names has to appear in that manifest, which makes the probe crate the one place a reviewer checks when a ban is added.

The arming mechanism is the expect ratchet run in the other direction, and it is exact rather than heuristic: if the entry resolves the call site produces `use of a disallowed method`, the expectation is fulfilled and the run is clean, while if a typo stops it resolving nothing fires, the expectation is unfulfilled, and `unfulfilled_lint_expectations` under `--deny warnings` is a hard error.
Measured on the workspace this file publishes: with both paths spelled correctly the gated lane exits 0 with no diagnostics, and misspelling `str::split_at` alone produces `error: this lint expectation is unfulfilled` at exit 101, pointing at the attribute.
Both probes are `const fn` so the advisory lane does not add `missing_const_for_fn` to its output.

Two properties of the technique decide where else to use it.
It is strictly stronger than the reachability warning, since it proves the ban fires rather than that the path resolves, and it generalises to every entry at the cost of one call site each; it is applied here only to the two entries with no other control, and extending it is the right response to any future entry the scan cannot see.
It is also the wrong control for a deleted entry, because deleting the entry and its probe together is one coherent edit, so a CI grep asserting that the exact strings `path = "str::split_at",` and `path = "str::split_at_mut",` appear in the root `clippy.toml` runs alongside it: the probe catches a misspelling, the grep catches a deletion, and neither catches the other.

A misspelt lint name fails silently too, and is the likelier edit, since the lint table is the file an agent reaches for first: `this_lint_does_not_exist_xyz = "deny"` in `[workspace.lints.clippy]` produced `warning[E0602]: unknown lint`, carrying the note that the `unknown_lints` lint ignores `-D warnings`.
So does a renamed one: `integer_arithmetic = "deny"`, the exact rename this file warns about above, produced a warning reading `lint clippy::integer_arithmetic has been renamed to clippy::arithmetic_side_effects`, with the matching note about `renamed_and_removed_lints`.
Raising the level explicitly does not close either, which is the part worth measuring rather than assuming, since it is the first thing anyone tries: appending `-D unknown_lints` or `--forbid unknown_lints` left the unknown-lint diagnostic at warning level, appending `-D renamed_and_removed_lints` left the rename diagnostic at warning level, and cargo exited 0 in every one of those runs.

The control therefore has to read the gated lane's own output, and it has to cover every channel that produces output rather than the one an earlier draft named.
Run the gated invocation with `--message-format=json` and fail on any diagnostic whose `code` field is `E0602` or `renamed_and_removed_lints`, or whose message contains `does not refer to a reachable`.
The split is not arbitrary: the first two carry those strings as machine-readable codes, while the reachability warning carries no code at all and can only be matched on text.
Two channels produce no output for the scan to read and each gets its own mechanism rather than being folded into this one: the probe crate converts the unreferenced-crate case into a channel that does produce output, and the armed call sites with the exact-string grep cover the primitive-type-path case, where no output exists to scan for at all.
Stating that boundary matters more than the scan does, since the scan's own failure mode is being trusted for the two cases it cannot see.
That is a dozen lines of `xtask` rather than the one-line grep it replaces, and together with the probe crate it is what stands between this file and a lint table, a ban list, or both, that has quietly stopped doing anything.

## The panic cluster is not closed, and no lint closes it

This subsection exists because the lint table above is the part of this charter most likely to be mistaken for a guarantee.

Denying `unwrap_used`, `expect_used` and `panic` does not remove the class of bug those lints target; it removes the loud form and leaves the quiet one, and in a system doing price and quota arithmetic the quiet one is worse, because a contained panic routes a job to `needs_human` while a substituted default charges the wrong amount and reports success.

Six workarounds an agent reaches for naturally were measured under the full table.
The three new lints catch three of them — `unreachable!("caller guarantees Some")` fires `clippy::unreachable`, `a / b` fires `clippy::integer_division`, and `&s[0..2]` fires `clippy::string_slice`.
`str::split_at` is not caught by `string_slice` despite panicking on the same condition, and is reachable only through `disallowed-methods`, which is why it is listed there and why its entry is armed by a call site in the probe crate rather than trusted; `split_at_checked` exists on stable 1.97.1 and gives it a total alternative.
`o.unwrap_or_default()` and `s.get(i).copied().unwrap_or_default()` are caught by no lint and would be reachable only through `disallowed-methods`, which has no per-crate form, so banning the method in the billing, quota and taxonomy crates, where a substituted zero is a wrong number, while permitting it everywhere it is an ordinary idiom is not expressible at all, so both forms join the uncovered group below rather than the ban list.

The remainder are caught by nothing, and this is where the honesty is required.
`o.unwrap_or(0)` produced zero diagnostics under every configuration measured.
The explicit `match o { Some(v) => v, None => 0 }` produced `clippy::manual_unwrap_or_default`, whose suggested fix is `o.unwrap_or_default()`, so banning that method would set the lint table against itself, routing the agent out of a legible construct into a forbidden one and from there into the silent one.
The class is "substitute a plausible default for a missing value", its surface is unbounded, and no finite list of method paths closes it.

So the rule is stated as a limit rather than as coverage.
The lint table converts the easy panics into compile errors and nothing more.
The only controls on the substituted-default class are full review of the zero-debt core, meaning bodies and not just assertions, and property tests whose oracle is independent of the implementation.
Writing this down matters more than the lints do, because a founder who believes the table covers this stops reading the diff, and that belief is how the failure actually arrives.

## The expect ratchet

`#[expect(..., reason = "...")]` is the only escape hatch and `reason` is unchecked text, so an agent will write "complex orchestration logic" and move on, which makes the count the only mechanically-checkable signal, and it is checked.

CI counts `#[expect(` occurrences per crate and compares against a committed baseline file, failing when a crate's count rises.
Verified that the grep is sound: rustfmt wraps a long `#[expect(...)]` across lines but keeps `#[expect(` on its own line, so `rg -c '#\[expect\('` counted correctly both before and after formatting.
This is the charter's own advice about `too_many_lines` generalised — a rising suppression count is evidence against a rule rather than evidence of discipline — and it applies to the rule as much as to the code.

## Founder-gated shared state

The workspace lints table is shared global state, and an agent that hits a wall will edit it, because that is the cheapest path to a green build.

| Gated path | Why an agent edits it | What the edit costs |
|---|---|---|
| `Cargo.toml` `[workspace]` and `[workspace.lints]`, `clippy.toml`, `deny.toml` | Cheapest path to a green build | Silently invalidates the whole charter |
| A new `clippy.toml` anywhere below the root | Looks like a crate-scoped addition | Replaces the root file for that crate, at exit 0 |
| A crate-root `#![deny(...)]` line, and `crates/ban-probe` | Cheapest path past a per-crate lint or an armed ban | Removes a control whose absence produces no diagnostic |
| `crates/limits` | Cheapest path past a bound that is failing | Turns a designed ceiling into whatever made the test pass |
| Any `Cargo.toml` dependency addition | Cheapest path past a missing capability | Adds licence, supply-chain and architectural-edge risk that no later review revisits |

The mechanism has to work for a solo founder with no second reviewer, so it is a pre-commit hook rather than a CODEOWNERS rule: the hook refuses a commit whose diff touches any gated path, prints the path and the reason, and requires an explicit override flag.
Rows two and three are the ones a path-matching hook alone would miss, since both are additions rather than edits to a named file, so the hook is paired with the two checks that see them: the file count asserting one `clippy.toml`, and the `xtask` table of required crate-root attributes.
Deliberate changes cost one flag, and the flag is what makes the decision conscious.
A dependency addition additionally re-runs `cargo-deny` before the commit is allowed, since that is the moment the licence question is cheapest to answer.

## cargo-deny

Every key below was read from the shipped `deny.template.toml` and the config struct of cargo-deny 0.20.2, where the licences section is allow-list only and the older `copyleft`, `deny` and `unlicensed` fields are gone.
The file itself ran: `cargo-deny 0.20.2 check bans licenses sources` exited 0 against the verification workspace of the previous revision, and the ban semantics below were measured rather than reasoned about, on a purpose-built four-crate graph at `/tmp/claude/claude-1000/-home-sernl-projects/7acda150-a31b-4ba6-9a45-cbb73bfa0e51/scratchpad/repair1/edges`.
That graph holds the two scenarios as separate workspaces, `direct/` and `transitive/`, and `reproduce.sh` reruns all four measurements and prints the exit code and diagnostic counts for each.
`multiple-versions` stays unverified, because neither of those trees contains a duplicate.
For a proprietary product this is the section that matters most, because a copyleft transitive dependency is a legal problem discovered at exactly the wrong moment.

```toml
[licenses]
allow = ["MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0", "Zlib"]
confidence-threshold = 0.93

# Measured, not anticipated: reqwest with rustls-tls reaches webpki-roots,
# whose licence covers Mozilla's CA set as data.
# Scoped to the one crate so the decision stays a greppable line rather than
# a licence added to the list above.
[[licenses.exceptions]]
crate = "webpki-roots"
allow = ["CDLA-Permissive-2.0"]

[licenses.private]
ignore = true          # the workspace's own unpublished crates

[bans]
multiple-versions = "warn"
wildcards = "deny"
allow-wildcard-paths = true
deny = [
  # Architectural edges. `wrappers` names every crate allowed to depend
  # directly on the banned one; any other direct dependent fails the check.
  { crate = "thirtyfour", wrappers = ["browser-driver"], reason = "one crate speaks WebDriver and every adapter goes through it" },
  { crate = "browser-driver", wrappers = ["automation-worker"], reason = "no axum, client-facing or telemetry crate may reach a live session" },
  { crate = "llm-client", wrappers = ["listing-copy"], reason = "every provider call goes through LlmBudget::reserve, which lives in listing-copy" },
  # The provider SDK takes its own entry, wrapped by llm-client, once a
  # provider is chosen. No crate name is written here before then.
]

# The JSON depth bound is a feature rather than a call site: with this feature
# off, the method that disables the 128-deep limit does not exist at all.
[[bans.features]]
crate = "serde_json"
deny  = ["unbounded_depth"]

[advisories]
yanked = "deny"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

The one licence exception is measured rather than anticipated, and it arrives with the HTTP client: a workspace pulling `reqwest` with `rustls-tls` reaches `webpki-roots 1.0.9` through `hyper-rustls`, and `check licenses` rejects its `CDLA-Permissive-2.0` against the allow list above and exits 4, while the scoped exception clears it and the run prints `licenses ok`.
The file shipped with that crate is the Community Data License Agreement Permissive 2.0, which covers Mozilla's certificate set as data rather than covering code, and whether that is acceptable for a proprietary product is a legal read rather than an engineering one, unverified here and the reason the exception is scoped to one crate instead of the licence joining `allow`.
An earlier dependency-free verification workspace could not have found this, which is the general lesson: a licence allow-list is only tested by a graph that has licences in it.

`[bans.build]` is gone: read at cargo-deny 0.20.2 `src/bans/cfg.rs`, `executables` is the lint level "for when executables are detected within crates with build scripts" and `interpreted` is "the lint level for interpreted scripts", both scanning crate contents rather than dependency edges, whereas the earlier draft's comment claimed they guarded against a build script pulling in an automation dependency tree.
`include-dev` stays at its default of false, since dev-dependencies do not ship.

`[[bans.features]]` is where the JSON recursion bound is actually enforced, and it was measured in both directions on 0.20.2.
Against a graph whose serde_json carries default features, `check bans` printed `bans ok` and exited 0; against the same graph with `features = ["unbounded_depth"]` it printed `error[feature-banned]: feature 'unbounded_depth' for crate 'serde_json = 1.0.151' is explicitly denied` and exited 2, which makes it the primary control on the depth bound and the `disallowed-methods` entry the second line behind it.

`allow-wildcard-paths = true` is not optional in a path-dependency workspace, and it was measured rather than copied: with `wildcards = "deny"` alone, an intra-workspace `{ path = "../<crate>" }` dependency carrying no version is reported as `error[wildcard]`, one per dependent crate, and the run exits 2.
The key exempts path dependencies while leaving the check in force for registry crates, which is the behaviour wanted.

The `[bans] deny` list carries the architectural edges the charter states in prose, and its mechanism is worth stating precisely, because an earlier draft claimed more for it than it does.
Measured on the four-crate graph: with one crate banned and a single wrapper named, a second crate depending on the banned crate directly produced `error[banned]` plus `warning[unmatched-wrapper]` naming that second crate, and the run exited 2; adding it to the wrapper list produced `bans ok` and exit 0.
The warning naming the unmatched parent is the useful half operationally, because the first run tells you which crates legitimately sit above the banned one instead of leaving you to guess.

The limitation is the part that matters, and it is measured too: `wrappers` matches direct parents only.
With `api` depending on `middle`, `middle` depending on the banned crate, and `middle` listed as a wrapper, cargo-deny reported `bans ok` even though `api` reaches the banned crate through it.
So `[bans]` cannot express "this crate's dependency closure excludes that one", which is the shape of the `sync-core` purity rule, and stating it as a ban would have produced a rule that passes while being false.
What enforces that rule instead is an `xtask` check over `cargo metadata`'s resolved graph: walk the closure from `sync-core` and fail when tokio, reqwest, sqlx or the browser driver appears anywhere in it.
That is a few dozen lines, it is exact, and it covers the transitive reach the ban mechanism does not.

## The limits module

`crates/limits` is the fourth primary artefact, and it lives here with the other three because it is edited on the same cadence as they are.
The charter's section 6 carries the admission rule that governs what may enter this crate, the provenance-marker scheme, and the table of the ten constants with their values; this is the source.
It compiles, lints and tests clean on rustc 1.97.1, cargo 1.97.0 and clippy 0.1.97 — `cargo clippy --all-targets -- --deny warnings` exits 0, `cargo fmt --check` exits 0, and `cargo test` reports five passed — in the verification workspace named at the top of this file, whose copy of the crate is the block below and nothing else.
The whole file is published rather than an excerpt of it, because an excerpt makes a reader reconstruct the constants that the assertions and the tests refer to, and a reconstruction is not the artefact.

```rust
//! Every resource bound in the system. Nothing outside this crate declares one.
//!
//! Admission rule: a constant belongs here only when exceeding it is a
//! shared-resource incident affecting tenants other than the one that caused
//! it, and when no type, database constraint, or OS-level limit already bounds
//! it. A number that bounds only its own caller is an ordinary `const` next to
//! that caller. This crate is deliberately small; a large limits module is a
//! maintenance surface impersonating discipline.
//!
//! Every constant admitted to the resource-bound set carries a provenance
//! marker in its doc comment, a factual claim about where the number came
//! from; the three per-tier rates are covered by the marker on `Tier::quota`.
//! `MEASURED` cites an observation recorded during the spike, named in the
//! comment. No constant carries this marker yet, because the spike has not run.
//! `SIZED` means derived by arithmetic from a quantity that is known
//! independently of the spike, such as the box's RAM or a protocol limit.
//! `UNCALIBRATED` is a guess, and is a release blocker for the first paying
//! deployment rather than a wish. The test below pins how many of them exist,
//! so lowering the budget is a deliberate edit and raising it cannot pass
//! unnoticed. Drive it to zero before taking money.
//!
//! This crate has no dependencies and must keep none, so that every other
//! crate can depend on it without acquiring an edge.

#![forbid(unsafe_code)]

use std::num::NonZeroU32;

/// Billing tier.
///
/// Declared here rather than in the billing crate because `limits` is a leaf
/// with no workspace dependencies and the quota table below is keyed by it.
/// The billing crate re-exports this type; it does not redeclare it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    Free,
    Pro,
    Studio,
}

/// The quotas a single tier grants.
///
/// One struct per tier rather than parallel arrays indexed by a discriminant.
/// Arrays would need an index, a bounds check, and a length assertion per
/// array; a struct returned from an exhaustive `match` needs none of those,
/// and adding a tier becomes a compile error at the one site that matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TierQuota {
    pub api_requests_per_minute: NonZeroU32,
    pub listings_max: u32,
    pub storage_bytes_max: u64,
}

impl Tier {
    /// Every tier. Test-only scaffolding: nothing in production indexes it.
    ///
    /// Rust cannot check on stable that this array is total over the enum, so
    /// the forcing function is the exhaustive `match` in `all_is_total_over_the_enum`,
    /// which fails to compile when a variant is added. `wildcard_enum_match_arm`
    /// is denied workspace-wide, so that match cannot be silenced with `_`.
    pub const ALL: [Self; 3] = [Self::Free, Self::Pro, Self::Studio];

    /// UNCALIBRATED. Rates are placeholders chosen so the free tier cannot
    /// saturate one box at the concurrency below; listing and storage ceilings
    /// are placeholders until pricing is set. Nothing here is measured.
    #[must_use]
    pub const fn quota(self) -> TierQuota {
        match self {
            Self::Free => TierQuota {
                api_requests_per_minute: FREE_RPM,
                listings_max: 100,
                storage_bytes_max: 1 << 30,
            },
            Self::Pro => TierQuota {
                api_requests_per_minute: PRO_RPM,
                listings_max: 5_000,
                storage_bytes_max: 20 << 30,
            },
            Self::Studio => TierQuota {
                api_requests_per_minute: STUDIO_RPM,
                listings_max: 100_000,
                storage_bytes_max: 200 << 30,
            },
        }
    }
}

const FREE_RPM: NonZeroU32 = NonZeroU32::new(60).unwrap();
const PRO_RPM: NonZeroU32 = NonZeroU32::new(600).unwrap();
const STUDIO_RPM: NonZeroU32 = NonZeroU32::new(3_000).unwrap();

pub mod http {
    /// SIZED against RAM, not against traffic: at this ceiling the concurrent
    /// request limit cannot buffer more than a small fraction of the box's
    /// memory. Applies to every route except the ingestion upload routes.
    /// Revisit once the spike records real listing-metadata payload sizes.
    pub const REQUEST_BODY_BYTES_MAX: u64 = 2 * 1024 * 1024;

    /// UNCALIBRATED. No file-size census of any connected marketplace's
    /// resources has been taken. The spike's recording obligation covers it.
    pub const UPLOAD_BODY_BYTES_MAX: u64 = 256 * 1024 * 1024;
}

pub mod ingest {
    /// SIZED: the absolute ceiling on bytes written out of an archive,
    /// independent of the ratio check below. Bounding compressed input is not
    /// a zip-bomb defence; bounding decompressed output is.
    pub const ARCHIVE_UNCOMPRESSED_BYTES_MAX: u64 = 1024 * 1024 * 1024;

    /// UNCALIBRATED. Uncompressed divided by compressed, checked incrementally
    /// rather than after the fact. The number must be set from the ratio
    /// distribution of real seller bundles, which has not been sampled; set
    /// deliberately loose so a false rejection is unlikely before it is.
    pub const ARCHIVE_COMPRESSION_RATIO_MAX: u64 = 200;
}

pub mod job {
    use std::time::Duration;

    /// UNCALIBRATED: retry count is only meaningful once the fault taxonomy
    /// from the spike says which faults are worth retrying at all.
    pub const ATTEMPTS_MAX: u32 = 5;

    /// SIZED against support response time, not against publish duration: a
    /// wedged job must free its lease well inside one working day. Attempts
    /// multiplied by exponential backoff can exceed any sane duration at a
    /// small attempt count, so this deadline is enforced alongside the count
    /// rather than derived from it.
    pub const WALL_CLOCK_MAX: Duration = Duration::from_mins(30);

    /// UNCALIBRATED: bounded by how many ingestion subprocesses and browser
    /// sessions fit in the box's RAM alongside the connection pool, which has
    /// not been measured. Automation runs here, on our own infrastructure, so
    /// it is inside this bound rather than outside it. Deliberately low.
    pub const CONCURRENT_JOBS_GLOBAL_MAX: u32 = 8;
}

pub mod browser {
    use std::num::NonZeroU32;

    /// UNCALIBRATED. The count of browser sessions spawned at startup and never
    /// spawned on demand. One process drives every seller's authenticated
    /// session, so an unbounded pool is a cross-tenant memory incident rather
    /// than one tenant's slow job. The hardware ceiling is roughly thirty
    /// concurrent sessions on the current box; the operating point is set by
    /// marketplace pacing, which is an order of magnitude lower and unmeasured,
    /// so this starts at the serialised end and moves only on evidence.
    pub const SESSIONS_MAX: NonZeroU32 = NonZeroU32::new(2).unwrap();
}

pub mod llm {
    /// SIZED as a loss ceiling rather than from a token price: this is the
    /// per-tenant daily spend the business is willing to lose to a runaway
    /// loop before a human looks. Recompute against the provider's actual
    /// per-token price once one is chosen. Spend is a first-class bounded
    /// resource here, not a proxy for request count.
    pub const CENTS_PER_TENANT_PER_DAY_MAX: u32 = 500;
}

pub mod marketplace {
    use std::num::NonZeroU32;

    /// UNCALIBRATED, and the only constant here with a legal rather than an
    /// operational justification. Tes publishes no throttle and none has been
    /// observed; where one is published, as on Etsy, the lower figure binds.
    /// Raising it requires the written-terms answer in the charter's section 1.
    pub const OUTBOUND_REQUESTS_PER_MINUTE_MAX: NonZeroU32 = NonZeroU32::new(30).unwrap();
}

/// Relationships between constants, checked at compile time rather than at run
/// time. Each is a claim that could actually be false after an edit; a claim
/// the declaration already guarantees is not written here.
const _: () = {
    assert!(
        http::UPLOAD_BODY_BYTES_MAX >= http::REQUEST_BODY_BYTES_MAX,
        "the upload ceiling must not be tighter than the ordinary body ceiling"
    );
    assert!(
        ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX >= http::UPLOAD_BODY_BYTES_MAX,
        "an archive at the upload ceiling must be able to expand at all"
    );
    assert!(
        http::UPLOAD_BODY_BYTES_MAX * ingest::ARCHIVE_COMPRESSION_RATIO_MAX
            > ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX,
        "the ratio cap must be able to bind before the absolute cap, or one of them is dead code"
    );
    assert!(
        usize::BITS >= 64,
        "byte bounds are u64 and are converted to usize at the axum boundary"
    );
    assert!(
        browser::SESSIONS_MAX.get() <= job::CONCURRENT_JOBS_GLOBAL_MAX,
        "a pre-spawned session with no job that can reach it is memory held for nothing"
    );
};

#[cfg(test)]
mod tests {
    use super::{http, ingest, job, Tier};

    /// The `UNCALIBRATED` markers are a countdown, not decoration.
    ///
    /// Pinning the count makes removing a marker a deliberate edit and makes
    /// adding one impossible to do quietly. Drive the number to zero before the
    /// first paying deployment; that is what makes the marker a release
    /// blocker rather than a wish.
    #[test]
    fn uncalibrated_markers_are_ratcheted() {
        let marked = include_str!("lib.rs")
            .lines()
            .filter(|line| line.trim_start().starts_with("/// UNCALIBRATED"))
            .count();
        assert_eq!(
            marked, UNCALIBRATED_BUDGET,
            "the uncalibrated-constant budget moved; lower it deliberately or calibrate the number"
        );
    }

    /// Counts doc-comment markers only, so prose and assertion messages that
    /// mention the marker do not inflate it.
    const UNCALIBRATED_BUDGET: usize = 7;

    #[test]
    fn all_is_total_over_the_enum() {
        for tier in Tier::ALL {
            match tier {
                Tier::Free | Tier::Pro | Tier::Studio => {}
            }
        }
        assert_eq!(
            Tier::ALL.len(),
            3,
            "a variant was added to Tier without being added to Tier::ALL"
        );
    }

    #[test]
    fn every_tier_grants_a_strictly_larger_listing_quota_than_the_one_below() {
        let quotas: Vec<u32> = Tier::ALL.iter().map(|t| t.quota().listings_max).collect();
        for pair in quotas.windows(2) {
            assert!(pair[1] > pair[0], "tier quotas must increase: {pair:?}");
        }
    }

    #[test]
    fn the_wall_clock_deadline_outlives_the_full_retry_budget_at_the_backoff_base() {
        let base = std::time::Duration::from_millis(500);
        let worst = base * job::ATTEMPTS_MAX;
        assert!(
            job::WALL_CLOCK_MAX > worst,
            "the deadline must not fire before the retries are exhausted"
        );
    }

    #[test]
    fn a_maximal_upload_expanding_at_the_maximal_ratio_exceeds_the_absolute_cap() {
        let expanded = http::UPLOAD_BODY_BYTES_MAX * ingest::ARCHIVE_COMPRESSION_RATIO_MAX;
        assert!(
            expanded > ingest::ARCHIVE_UNCOMPRESSED_BYTES_MAX,
            "the absolute cap is the binding one for a maximal upload"
        );
    }
}
```

The test module above carries five tests, and two of them are the discipline rather than the arithmetic.
`uncalibrated_markers_are_ratcheted` counts lines in `include_str!("lib.rs")` whose trimmed form starts with `/// UNCALIBRATED` and asserts the total against a committed budget of seven, so removing a marker is a deliberate edit and adding one cannot pass unnoticed.
`all_is_total_over_the_enum` matches every `Tier::ALL` element against an explicit variant list, so adding a variant produces `E0004: non-exhaustive patterns` rather than a silently short array; the other three check that tier quotas increase, that the wall-clock deadline outlives the full retry budget at the backoff base, and that a maximal upload expanding at the maximal ratio exceeds the absolute cap.

`browser::SESSIONS_MAX` is new in this revision, and it is the one constant the architecture change created rather than moved: under local-first execution the browser pool sized one seller's own laptop and was excluded by this crate's own admission rule, while under server-side automation it sizes memory every tenant shares, which is that rule's central case.
It arrives with a const assertion relating it to `job::CONCURRENT_JOBS_GLOBAL_MAX` that can actually be false after an edit rather than being vacuous: raising the session count above the global job ceiling produced `error[E0080]: evaluation panicked: a pre-spawned session with no job that can reach it is memory held for nothing`, at build time, which is the failure wanted.

## The release profile

Rust's release profile disables both flags this system wants, so it is restored explicitly.
The charter's assertions-ship-in-release subsection carries the argument; this is the artefact.

```toml
[profile.release]
overflow-checks = true
debug-assertions = true
```

Verified with runtime operands: an addition that overflows `u8` panics under the dev profile, yields `4` under `-O`, and panics again under `-O -C overflow-checks=on`, and a profile with both flags restored still builds as an optimized artefact.
If profiling ever contradicts this, add a separate `[profile.dist]` rather than weakening the default.

## Nix flake checks, by speed tier

The fast tier is what `nix flake check` runs, and it must stay fast enough that the founder actually runs it.

| Check | Tool | Notes |
|---|---|---|
| Format | `craneLib.cargoFmt`, `craneLib.taploFmt`, treefmt-nix | Covers Rust, TOML, nix and markdown under one gate |
| Lint, gated | `craneLib.cargoClippy` | `cargoClippyExtraArgs = "--all-targets -- --deny warnings"` |
| Lint, advisory | `cargo clippy` in a plain derivation | Groups raised with `-W`; reported, never gated |
| Lint-configuration scan | `cargo xtask` over the gated lane's JSON output | Fails on `E0602`, `renamed_and_removed_lints`, or `does not refer to a reachable` |
| One clippy.toml | `fd` in a plain derivation | Exactly one path, at the workspace root; a second file replaces it silently |
| Ban-list strings | `rg -F` over the root `clippy.toml` | The two primitive-type paths must be present verbatim; the probe crate arms them |
| Crate-root attributes | `cargo xtask` over a committed crate-and-attribute table | A missing `#![deny(...)]` is a build failure, not a silent relaxation |
| Dependency closure | `cargo xtask` over `cargo metadata` | `sync-core`'s closure must contain no tokio, reqwest, sqlx or browser driver |
| Unit, scenario, property, acceptance tests | `craneLib.cargoNextest` | Includes the committed simulation seed corpus |
| Sandboxed ingestion | `craneLib.cargoNextest`, hostile-document corpus | Parsers run in a child process under rlimits; see charter section 7 |
| Migration lint | `cargo xtask` over `migrations/` | Destructive statements require a marker comment naming the expand migration and the elapsed soak |
| Licences, bans, sources | `craneLib.cargoDeny` | See the traps below |
| Advisories | `craneLib.cargoAudit` | Against a pinned `advisory-db` flake input; see the traps below |

Both supply-chain checks are cheap: crane's `cargoDeny` and `cargoAudit` set `cargoArtifacts = null` and `cargoVendorDir = null`, so neither compiles the tree.

Three traps are worth stating plainly, all three confirmed by reading crane's source at the revision cited rather than its README.
`cargoDeny` defaults to `cargoDenyChecks ? "bans licenses sources"` (`lib/cargoDeny.nix:8`), which omits the `advisories` check, so a default wiring gives licence and ban coverage while silently not checking for vulnerabilities.
`cargoAudit` defaults to `cargoAuditExtraArgs ? "--ignore yanked"` (`lib/cargoAudit.nix`), so the yanked signal is off in the audit lane too, meaning `yanked = "deny"` in `deny.toml` is necessary but not sufficient and this argument must be overridden explicitly.
`cargoAudit` runs `cargo audit -n -d ${advisory-db}` where `-n` means no-fetch and `advisory-db` is a `flake = false` input, which is what makes it hermetic and also means the check is only as fresh as `flake.lock`.
Holzmann's Rule 10 requires code to be checked daily; a pinned advisory database nobody bumps satisfies the letter and defeats the purpose, so schedule an automated `nix flake update advisory-db` with a CI run on the bump.
These details were read at crane HEAD 692f7e9 (2026-08-21), which is not necessarily the revision this project's `flake.lock` will pin, so re-confirm all three against the pinned revision.

The slow tier runs on a schedule or on pull requests, never in the local gate, because putting it there means the local gate stops being run.
`cargo-mutants` (27.1.0) with `--in-diff` for pull-request scope, configured with `cap_lints = true` in `.cargo/mutants.toml`, which is mandatory under this charter's deny-warnings policy or deleted code makes parameters unused and mutants are reported as unviable.
The simulation seed sweep, opening an issue with the seed on failure.
The end-to-end suite against a sandbox account, run once against the server-side automation plane, since there is one browser engine and no per-platform matrix to run it over.
The prompt-injection fixture corpus, and the synthetic marketplace canary, both of which are scheduled jobs rather than build steps.

One shipped-configuration note: use `cargo-hack --each-feature` plus an explicit matrix of the configurations actually shipped rather than `--feature-powerset`, which is exponential and is precisely the cost Holzmann's Rule 8 warns about; with React on the front end, no Rust on any client and one Linux target for the server, that matrix is a single column, so the note is now about keeping it that way rather than about pruning it.
