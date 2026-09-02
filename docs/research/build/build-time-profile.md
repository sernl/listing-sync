# Build time profile

This is a measurement of where the Rust build actually spends time in this workspace, taken on 2026-09-03 to answer whether an inputs-keyed build system such as Bazel would speed the gated lane up, and to price the cheaper alternatives against the same numbers.
It reports what was measured, not what is recommended in general; every figure below came from a run on this machine against this tree.

## How to read these numbers

The machine has 16 logical CPUs, 31 GiB of RAM and an NVMe root, running NixOS with rustc 1.97.1 and LLVM 22.1.6 from the flake's devshell.
`CARGO_BUILD_JOBS` is unset, so cargo used its default of one job per CPU, `-j 16`.
The workspace has 30 cargo members, 29 under `crates/` plus `apps/desktop/src-tauri`, totalling 103,312 lines of Rust across 687 packages in `Cargo.lock`, and 885 tests.

Several other agents were compiling on this machine throughout, in the shared `target/` directory, and the one-minute load average moved between 3.9 and 146 during the session.
Absolute numbers therefore carry substantial noise, and every measurement below was taken at least twice with the better run reported; where the spread between runs was large the other runs are shown beside it so the noise is visible rather than hidden.
Ratios measured within a single run — the contention comparison, the sccache hit rates, the linker arms — are far more trustworthy than the absolute wall-clock figures, and the argument in this document rests on the ratios.

Every experiment ran under `nix develop --command` with `CARGO_TARGET_DIR` pointed at a private directory in a scratchpad, so none of this touched the shared `target/`.
Because the committed sqlx offline cache is currently stale, the measurements set `DATABASE_URL` to the running dev Postgres and let the `sqlx::query!` macros resolve online; see the defects section for why, and note that this inflates `tam-storage` slightly relative to a green offline tree.

## The measurements

Checking the workspace, with `cargo check --workspace --all-targets --all-features`.

| measurement | best | other runs |
|---|---|---|
| cold check into an empty target directory | 63.3 s | 112.0 s, 202.4 s |
| warm no-op check | 0.77 s | 0.91 s, 3.21 s |
| check after touching leaf crate `tam-types` | 3.08 s | 3.26 s, 4.35 s |
| check after touching `tam-server` | 0.42 s | 0.47 s |
| target directory size, check only | 2.9 GB | 3.1 GB |

Linting, with `cargo clippy --workspace --all-targets --all-features -- --deny warnings`, and the other two steps of the gated lane.

| measurement | best | other runs |
|---|---|---|
| cold clippy into an empty target directory | 270.3 s | 311.2 s |
| warm no-op clippy | 0.40 s | 12.0 s, 13.5 s |
| clippy after touching `tam-types` | 6.87 s | 8.90 s |
| `cargo fmt --check` | 0.83 s | 2.15 s, 3.42 s |
| `just purity`, which is two `cargo tree` calls | 0.26 s | 0.31 s, 1.09 s |

Testing, with `cargo nextest run` as the gated lane invokes it.

| measurement | best | other runs |
|---|---|---|
| first nextest run in a check-warm directory | 213.5 s | 446.6 s |
| warm nextest run, no source change | 3.63 s | 12.1 s |
| `cargo nextest list` warm | 0.73 s | |
| execution of all 885 tests | 1.00 s | 0.99 s, 1.04 s, 1.15 s |
| slowest single test | 0.83 s | |
| target directory size, after building all test binaries | 17.0 GB | |

The slowest eight tests are all in `tam-standards::committed_ingest`, between 0.68 s and 0.83 s each, and the ninth is `tam-pipeline` at 0.57 s; nothing else exceeds 0.30 s.

Linking, measured with `cargo rustc` so the extra flags apply to the final binary only and its dependencies keep their fingerprints, best of three passes.

| variant | tam-server | tam-desktop (`teachouse`) |
|---|---|---|
| default linker | 2.60 s | 1.39 s |
| with mold | 2.28 s | 1.79 s |
| with `-C debuginfo=line-tables-only` | 1.79 s | 1.57 s |
| with mold and line-tables-only | 1.74 s | |
| with `-C debuginfo=0` | 1.96 s | |

Splitting debuginfo out of the binary rather than reducing it was measured as a separate best-of-three series, with its own baseline taken in the same series.

| variant | tam-server |
|---|---|
| default | 2.78 s |
| with `-C split-debuginfo=unpacked` | 2.63 s |
| with `-C split-debuginfo=packed` | 2.75 s |

Both land inside the run-to-run spread of their own baseline, so relocating debuginfo does nothing measurable here while reducing its quantity does.

Debuginfo was also varied across the whole dependency graph rather than one link, by building two cold target directories at different `profile.dev.debug` settings and driving the same one-line edit through the full gated lane in each.

| dev-profile debuginfo | gated lane after a one-line edit, best of two | target directory |
|---|---|---|
| `2`, the default | 48.59 s | 10.27 GB |
| `line-tables-only` | 19.15 s | 7.13 GB |
| `0` | 34.95 s | 4.55 GB |

The wall-clock column is not monotonic in the amount of debuginfo while the size column is, and that discrepancy should be read as a warning rather than as a finding about `debug = 0`.
Less debuginfo cannot make linking slower, so the ordering 48.59, 19.15, 34.95 is evidence that these three arms did not separate cleanly under the load present when they ran, even at best-of-two; one discarded pass in the middle arm was a 137 s outlier taken while four other builds were running.
The size column is load-independent and does behave monotonically, and the controlled per-link series above, whose arms ran back to back within one series, is the other trustworthy signal.
None of these figures is comparable to the 78.30 s lane reported later, which was taken in a differently-warmed directory.

The debug binaries are large: `tam-server` is 189.0 MB and `teachouse` is 250.9 MB.
mold 2.41.0 linked cleanly under the nix toolchain with no failures, so the small margin is a real result rather than a broken experiment.

Compilation caching with sccache 0.17.0 against a local disk cache, which required `CARGO_INCREMENTAL=0` because sccache refuses to run otherwise, exiting with `sccache: incremental compilation is prohibited: Unset CARGO_INCREMENTAL to continue.`

| scenario | wall | Rust cache hit rate |
|---|---|---|
| cold target directory, cold cache, populating | 75.7 s | 0.00 %, 603 misses |
| fresh target directory at a different path, warm cache | 119.2 s | 0.00 %, 603 misses |
| same target directory path, artefacts wiped, warm cache | 29.4 s | 100.00 %, 603 hits |
| touching `tam-types` with sccache active | 10.2 s | |

In the different-path run the only 60 hits were the assembler and C objects inside `ring`'s build script, which hit at 100 %; every Rust invocation missed.
229 calls were non-cacheable at all, 213 of them for `crate-type`, which is sccache declining proc-macro and dylib crates.

Lock contention between two concurrent agents, each running the full workspace check after a one-line edit to a different crate.

| arrangement | pass 1 | pass 2 |
|---|---|---|
| one agent alone | 2.48 s | 1.47 s |
| two agents, separate target directories | 5.23 s | 2.08 s |
| two agents, one shared target directory | 10.09 s | 4.36 s |

Cargo printed `Blocking waiting for file lock on build directory` and `Blocking waiting for file lock on package cache` in the shared arrangement, confirming the mechanism rather than merely the symptom.

Nix, warm, without running the whole `nix flake check`.

| build | wall |
|---|---|
| `checks.x86_64-linux.portable`, first invocation | 45.6 s |
| the same, fully cached | 2.0 s |
| `checks.x86_64-linux.fmt` | 8.4 s |
| `checks.x86_64-linux.clippy` | 126.0 s, then failed |

## What the critical path says

The cold check emitted 890 compilation units over 202.3 s of wall time for 2,907 unit-seconds of work, which is cargo's own accounting of 14.37 units running at once against 16 cores.
Both halves of that ratio are inflated by the competing agents, since a descheduled unit's duration grows while wall time grows with it, but the shape holds: this dependency graph already saturates the machine.
The tail is short, with half the units finished by 154.9 s, ninety per cent by 182.4 s and ninety-nine per cent by 197.4 s of a 202.3 s build, and the last unit to finish is `tam-desktop` taking 1.2 s at the very end.
There is no serialised critical path here for a smarter scheduler to unwind.

Splitting those unit-seconds by what they were spent on is where the answer to the Bazel question lives.

| category | unit-seconds | share |
|---|---|---|
| third-party dependencies | 2,813 | 96.8 % |
| our 30 workspace crates | 94 | 3.2 % |
| proc-macro crates | 553 | 19.0 % |
| build scripts, compiling and running | 325 | 11.2 % |
| of which `ring` alone | 318 | 10.9 % |
| the tauri, gtk and webkit desktop stack | 78 | 2.7 % |

Only the first two rows partition the total; the rest are overlapping cuts through the third-party share and are not meant to be summed.

By phase, the frontend — parsing, macro expansion, type checking and metadata — accounts for 1,753 unit-seconds or 60.3 %, codegen for 242 or 8.3 %, and the remaining 913 or 31.4 % is build-script execution, linking and I/O.
The heaviest individual units are `ring`'s build script twice at 155.4 s and 153.6 s, then `tokio` at 146.5 s, `serde_derive` at 120.0 s, `pxfm` at 104.7 s, `futures-util` at 82.6 s, `regex-syntax` at 66.2 s and `icu_properties` at 62.1 s.
Our own slowest crates are `tam-api` at 5.2 s, `tam-storage` at 4.8 s and `tam-domain` at 3.6 s.
Because `--all-targets` fans each crate out into a library unit plus one unit per test target, `tam-storage` appears as 27 units and `tam-api` as 16, and the suite as a whole is 68 separate test binaries.

The single most important number in this document is that our own code is 3.2 % of a cold build.

## What is actually costing the day

A cold build is not the daily experience, so the gated lane was also measured end to end after a one-line edit to `tam-types`, in a target directory already warm for both clippy and the test binaries, best of two passes.

| step | time | share |
|---|---|---|
| `cargo fmt --check` | 2.15 s | 3 % |
| `cargo clippy --all-targets --all-features` | 14.38 s | 18 % |
| `just purity` | 1.09 s | 1 % |
| `cargo nextest run` | 60.66 s | 78 % |
| total | 78.30 s | |

Running the same lane with no edit at all costs 10.15 s.
So a one-line change to a widely-depended-on crate costs roughly 78 s, and 78 % of that is `cargo nextest run` — of which, by the numbers above, about 1 s is running tests and the rest is generating code for and linking the test binaries that transitively depend on the crate that changed.
There are 68 such test binaries, each carrying full dev-profile debuginfo, which is why this step dominates and why the debuginfo lever below is aimed at it.

Two hypotheses were tested here and both failed, which is worth recording so nobody spends time on them.
The gated lane runs clippy with `--all-features` and nextest with default features, which looked like it should force a full recompile between the two steps, but flipping back and forth cost 0.77 s and 1.79 s because cargo keeps both feature-set fingerprints side by side in one target directory.
Partitioning or sharding the test run is likewise pointless, because the entire suite of 885 tests executes in 1.0 s and the slowest single test is 0.83 s; there is no run time to shard.

## Cheap levers, ranked by measured saving

The ranking is by measured saving against the daily 78 s lane and the concurrent-agent case, divided by how much has to change to get it.

Giving each agent its own target directory is first, and it is the only lever whose measured effect is large, cheap and specific to this fleet's actual problem.
Two agents sharing one target directory took 4.36 s where two agents with separate directories took 2.08 s and a single agent alone took 1.47 s, so sharing costs 2.1 times the split-directory time and 3.0 times solo, and cargo names the mechanism in its own output.
That penalty applies to the whole 78 s lane, not to the 2 s micro-benchmark, and it grows with the number of concurrent agents rather than staying flat.
The change is to set `CARGO_TARGET_DIR` per agent, to a path that is stable for that agent across its whole life, for instance `CARGO_TARGET_DIR=$XDG_CACHE_HOME/cargo-targets/$AGENT_ID`; it does not belong in `.cargo/config.toml`, which would set one shared path for everyone.
The cost is disk, at 2.9 GB per agent for a check-only directory and 17.0 GB for one that has built the test binaries, against 320 GB free.

Cutting dev-profile debuginfo is second, and it is the lever most likely to move the 60.66 s nextest step, though the measurement supports its direction more firmly than its size.
The controlled evidence is the per-link series, where `line-tables-only` took 0.81 s off a 2.60 s `tam-server` relink, or 31 %, with all arms run back to back inside one series.
The load-independent evidence is that the same setting writes 31 % fewer bytes into the target directory, 7.13 GB against 10.27 GB, and `debug = 0` writes 56 % fewer at 4.55 GB.
Since the debug binaries are 189 MB and 251 MB and 68 test binaries are relinked whenever a widely-depended-on crate changes, a 31 % cut in what must be generated and written on each of them is where the daily saving would come from.
The whole-lane arms did produce a best case of 19.15 s against a 48.59 s baseline, but they failed the monotonicity check described above, so that 61 % should be treated as an upper bound observed once rather than an expected saving.
The change is `[profile.dev] debug = "line-tables-only"` in the workspace `Cargo.toml`, which keeps backtrace line numbers and loses only variable inspection in a debugger, and it is founder-gated because `Cargo.toml` is a shared gate.
It is also the one lever here that could be re-measured cheaply and conclusively on an idle machine, which is worth doing before adopting it.

Fixing the crane source filter is third, because it is a one-character change that currently costs an unbounded amount.
`nix build .#checks.x86_64-linux.clippy` builds 50 derivations for 126 s and then fails, and the same is true of `nextest` and `bin`.

Skipping the desktop crate in the routine local lane is fourth and is a judgement call rather than a clear win.
`tam-desktop` contributes 77.5 unit-seconds of tauri, gtk and webkit dependency compilation to a cold build and is the last unit to finish, and the flake already excludes it from the sandboxed lanes with `--exclude tam-desktop`.
Matching that exclusion in the local `just check`, and covering the desktop crate in `just desktop-build` instead, would align the two and shorten cold builds, at the cost of catching desktop breakage later.

sccache is fifth and, as configured for the way this fleet works, it is a net loss.
At a stable target-directory path with the artefacts wiped it achieved a 100 % Rust hit rate and cut the rebuild from 75.7 s to 29.4 s, which is a real 2.6-fold speedup.
But at a different target-directory path it achieved 0.00 % on Rust, because sccache's Rust cache key includes the `--extern` and `--out-dir` paths, so moving the target directory invalidates every entry — which is precisely what per-agent target directories do.
It also refuses to run unless `CARGO_INCREMENTAL=0`, and losing incremental compilation made the daily incremental step 3.3 times slower, 10.2 s against 3.1 s.
The two top levers are therefore in direct conflict with sccache, and the per-agent directories are worth more.

mold is last and should not be adopted.
rustc 1.97.1 already uses `rust-lld` as the default linker on this target, which was confirmed by observing `rust-lld` processes during the concurrent agent's build, so mold is competing against a fast linker rather than against GNU ld.
It saved 0.32 s on a 2.60 s `tam-server` relink and lost 0.40 s on `tam-desktop`, both inside the run-to-run noise, and linking is not where this build spends its time.

Two further changes are worth naming even though they are not build-speed levers.
Setting `package.name` or `workspace.metadata.crane.name` in the root `Cargo.toml` would silence the crane placeholder warning that currently prints four times on every `nix develop` entry and names every derivation `cargo-package-*`.
And the gated lane cannot presently run at all, for the reason in the next section.

## What an inputs-keyed cache would add

Bazel's value proposition is that an action whose inputs have not changed is not re-executed, and that the resulting artefacts can be shared through a content-addressed cache across machines and users.
Measured against this workspace, most of that value is already collected by something else.

The 96.8 % of cold-build work that is third-party dependencies is already cached twice over.
Cargo never recompiles it after the first build in a given target directory, and crane already caches it in the nix store: `cargo-package-deps-0.0.1.drv` is a shared direct input of the `clippy`, `nextest` and `bin` checks, and its only tree-derived input is a `cleaned-mkDummySrc` directory that was inspected and contains exactly one file, `Cargo.lock`, alongside each crate's `Cargo.toml` and crane's `dummy.rs` stubs.
Rust source edits therefore never invalidate the dependency artefact, and only a `Cargo.toml` or `Cargo.lock` edit rebuilds it.
Bazel would be a third cache over the same artefacts.

The 3.2 % that is our own code would not get faster, because rustc's unit of compilation is the crate in both systems.
`rules_rust` compiles one crate per action just as cargo does, so an edit to `tam-types` invalidates every downstream crate under Bazel exactly as it does under cargo, and the pipelining that lets dependents start from metadata before codegen finishes is already present here — the timing data carries `unblocked_rmeta_units` edges.
Neither can Bazel add parallelism the graph does not contain, and the graph already runs at 14.37 units in flight on 16 cores with 99 % of units done by 97.6 % of wall time.

Three things an inputs-keyed cache would genuinely add, in descending order of measured worth.

It would let concurrent agents share compiled artefacts, which is the one real gap the measurements expose.
sccache cannot do this: its 0.00 % Rust hit rate across target-directory paths, against 100.00 % at a stable path, shows that the cargo-plus-sccache combination keys on the build location rather than on content.
A content-addressed cache would let a second agent start warm, worth roughly the 63 s to 270 s of a cold check or clippy rather than anything on the warm loop.

It would cache build-script execution content-addressably, which matters because `ring` alone is 318 unit-seconds, 10.9 % of a cold build and 95 % of all build-script cost.
Today that work is redone once per target directory, so per-agent directories multiply it.

It would cache test results, so unchanged test binaries are not re-run — worth about 1.0 s, since that is how long all 885 tests take.

Set against that, migrating means encoding 687 `Cargo.lock` packages through `crate_universe`, re-expressing the workspace lints table that `Cargo.toml` currently owns as a founder gate, and reproducing the sqlx offline cache, the tauri build with its `frontendDist`, the five cross-compilation targets in `check-portable`, `cargo-nextest`, `cargo-deny`, `cargo-audit` and the crane checks that already exist and work.
The measured ceiling is the difference between the current 78 s edit-to-verdict loop and one where nothing but the changed crates and their test binaries is rebuilt, and since that is already what cargo does, the realistic gain on the daily loop is close to zero.

The conclusion the numbers support is that Bazel would buy cross-agent artefact sharing at the price of re-encoding the entire build, and that per-agent target directories plus a debuginfo reduction reach most of the same daily benefit for two configuration lines.
The honest counter-argument is that cross-agent sharing is real and unaddressed by any lever listed here, so if the fleet grows to many agents doing cold starts, the calculation changes; it does not change for one founder and a handful of agents on one machine.

## Two defects found while measuring

The committed sqlx offline cache is stale, so the gated lane cannot currently run.
`SQLX_OFFLINE=true cargo check` fails with `there is no cached data for this query` for eight `sqlx::query!` invocations in `crates/tam-storage/src/jobs.rs`, at lines 309, 821, 969, 1074, 1087, 1145, 1174 and 1530.
The queries themselves are fine: pointing `DATABASE_URL` at the running dev Postgres compiles all of them, so the schema matches and only `crates/tam-storage/.sqlx` is behind.
Regenerating it needs a `cargo sqlx prepare` run, which was deliberately not performed here because it writes into the repository.

The crane source filter drops the standards fixtures, so three nix checks fail after doing all their work.
`flake.nix` line 51 admits `.*/docs/design/data/.*\.json`, which does not match a `.jsonl` path, and `crates/tam-standards/tests/committed_ingest.rs` pulls in `ngss.jsonl`, `teks.jsonl`, `va-sol.jsonl`, `ccss.jsonl` and `tpt-node-ids.jsonl` with `include_bytes!` and `include_str!`.
`builtins.match ".*/docs/design/data/.*\.json"` against such a path returns null, while `.*/docs/design/data/.*\.jsonl?` matches, so widening the pattern by one character fixes it.
Until then `checks.clippy`, `checks.nextest` and `checks.bin` each build 50 derivations and then fail compiling `tam-standards`, which is 126 s wasted per attempt.
