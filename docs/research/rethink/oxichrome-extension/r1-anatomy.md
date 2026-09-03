# oxichrome anatomy and feasibility (R1)

Prepared 2026-09-03.
Clone examined: `~/ghq/github.com/0xsouravm/oxichrome`, acquired today with `ghq-sync` and promoted to full history with `ghq-sync --full`.
Every `path:line` citation below is into that clone at the commit in the table.
Every URL was fetched 2026-09-03.

## Metadata

| Field | Value | Source |
|---|---|---|
| Repository | https://github.com/0xsouravm/oxichrome | `gh api repos/0xsouravm/oxichrome` |
| Commit examined | `718c56d0d08763600c704ef18ae0cb060e7d8e83` | `git log -1` |
| Commit date | 2026-07-28 10:08:50 +0530 | `git log -1` |
| Branch | `main` | `git rev-parse --abbrev-ref HEAD` |
| Created / last push | 2026-02-11 / 2026-07-28 | `gh api repos/0xsouravm/oxichrome` |
| Stars / forks / watchers | 327 / 10 / 2 | `gh api repos/0xsouravm/oxichrome` |
| Open issues + PRs | 1 issue, 3 PRs | `gh issue list`, `gh pr list` |
| Licence | MIT | `LICENSE:1`, workspace `Cargo.toml:16` |
| Commits / authors | 19 / 5 | `git rev-list --count`, `git shortlog -sne` |
| Tags / releases | `v0.1.0`, `v0.2.0` (both 2026-02-14) | `gh api .../releases` |
| Workspace version | 0.2.1 (unpublished) | `Cargo.toml:14` |
| crates.io latest | 0.2.0, published 2026-02-14 | https://crates.io/api/v1/crates/oxichrome |
| Total downloads, all 5 crates | 1,304 | crates.io API, five crates summed |
| MSRV | none declared | `rg rust-version` returns nothing; `rust_version: null` on every published version |
| `unsafe` blocks | 0 | the two `rg '\bunsafe\b'` hits are the string `wasm-unsafe-eval` at `oxichrome-build/src/manifest.rs:99,265` |
| Tests | 38 unit tests, no integration or browser tests | `rg -c '#\[test\]'` |
| CI | none | no `.github/` directory exists |
| Rust source | roughly 2,700 lines across 45 files | `wc -l` over the tree |

## What it actually is

oxichrome is a build-tool-plus-proc-macro framework that compiles a single Rust `cdylib` to wasm and generates a complete Manifest V3 extension directory around it, not a bindings crate and not a DevTools-protocol controller.
The evidence is that `cargo-oxichrome` drives `cargo build --target wasm32-unknown-unknown`, then `wasm-bindgen --target web`, then writes `manifest.json`, `background.js`, `popup.html`/`popup.js`, `options.html`/`options.js` and one `content_script_<fn>.js` per content script, then optionally runs `wasm-opt -Oz` (`oxichrome-cli/src/commands/build.rs:34-149`).
Nothing in the tree speaks the Chrome DevTools Protocol, and nothing drives a browser from outside; the only browser contact is `#[wasm_bindgen]` extern declarations against the in-page `chrome.*` global (`oxichrome-core/src/js_bridge.rs:7-65`).
It is simultaneously a template generator (`cargo oxichrome new`, `oxichrome-build/src/templates.rs:1-62`) and a thin binding layer, but the binding layer is the smallest of its parts.

## Anatomy

### Crate layout

Five workspace members plus two examples (`Cargo.toml:3-11`).
`oxichrome` is a facade that re-exports the macros, the runtime modules and a `__private` module holding `wasm_bindgen`, `wasm_bindgen_futures`, `js_sys`, `serde`, `serde_json`, `serde_wasm_bindgen` and `leptos` for macro expansion to reach (`oxichrome/src/lib.rs:1-33`).
`oxichrome-core` is the runtime: extern declarations, three API modules and an error type (`oxichrome-core/src/lib.rs:1-8`).
`oxichrome-macros` is the proc-macro crate (`oxichrome-macros/Cargo.toml:12-13`).
`oxichrome-build` holds the source parser, manifest writer, JS/HTML shim writer and project templates (`oxichrome-build/src/lib.rs:1-10`).
`oxichrome-cli` builds the `cargo-oxichrome` binary with three subcommands: `build`, `clean`, `new` (`oxichrome-cli/src/main.rs:25-37`).

### How an extension is declared

Six attribute macros, not the five the README claims (`oxichrome-macros/src/lib.rs:16,23,34,47,60,73` against `README.md:45`).
They are `#[oxichrome::extension]` on a unit struct carrying `name`, `version`, `description`, `permissions` and `host_permissions` (`oxichrome-macros/src/parse.rs:109-116`); `#[oxichrome::background]` on an async fn; `#[oxichrome::on(namespace::event)]`; `#[oxichrome::popup]` and `#[oxichrome::options_page]` on Leptos components; and `#[oxichrome::content_script(matches = [...], run_at, all_frames, css)]`.
There are no traits and no configuration file — the attributes are the whole declaration surface.

The build tool does not read the macro expansion; it re-parses the Rust source text with `syn` and walks it for those same attributes (`oxichrome-build/src/source_parser.rs:194-235`).
Two consequences follow and both are hard constraints.
First, only `src/lib.rs` is parsed — `find_lib_rs` returns that one path and nothing walks the module tree (`oxichrome-cli/src/commands/build.rs:240-246`), so every macro invocation that must appear in the manifest has to live in that single file.
Second, the attribute must be written with exactly two path segments whose first is literally `oxichrome`, because `is_oxichrome_attr` checks `segments.len() == 2 && segments[0].ident == "oxichrome"` (`oxichrome-build/src/source_parser.rs:48-54`); importing the macro through the prelude and writing `#[extension(...)]` compiles but silently produces an empty manifest.

### Manifest generation

One manifest per invocation, selected by `--target chromium|firefox`, written into `dist/<browser>/` (`oxichrome-cli/src/main.rs:29-31,45-49`, `build.rs:58-62`), so covering both browsers means two builds and two output trees.
`manifest_version` is hardcoded to 3 (`oxichrome-build/src/manifest.rs:81`); MV2 is not reachable.
Chrome gets `background: {service_worker: "background.js", type: "module"}` (`manifest.rs:94-97`).
Firefox replaces that whole key with `background: {scripts: ["background.js"], type: "module"}` and adds `browser_specific_settings.gecko.id` derived as `<lowercased-name-with-dashes>@oxichrome.dev` (`manifest.rs:126-146`).
The gecko id is not configurable — an open PR proposes making it so and has not been merged (PR #5, opened 2026-05-17, still open).
The generated manifest can express exactly these keys: `manifest_version`, `name`, `version`, `description`, `permissions`, `host_permissions`, `action.default_popup`, `background`, `content_security_policy.extension_pages`, `options_ui`, `content_scripts`, `web_accessible_resources` (`manifest.rs:6-27`).
There is no `icons`, no `default_icon`, no `commands`, no `declarative_net_request`, no `minimum_chrome_version`, no `version_name`, no `key`, no `update_url`, and no mechanism to merge a user-supplied fragment.
An undocumented escape hatch exists by accident: `static/` is copied over `dist/<browser>/` after the manifest is written (`build.rs:90-91` then `build.rs:126-130`), so a `static/manifest.json` silently replaces the generated one.
`web_accessible_resources` is hardcoded to expose `wasm/*` to `<all_urls>` with no way to narrow it (`manifest.rs:118-121`), which means any web page can probe for and load the extension's wasm module.

### WebExtension API coverage

The bindings are hand-written `#[wasm_bindgen] extern "C"` declarations in one 77-line file, not generated and not delegated to a bindings crate (`oxichrome-core/src/js_bridge.rs:1-65`).
The complete surface is: `runtime.getURL`, `runtime.sendMessage`, `runtime.onInstalled`, `runtime.onMessage`; `storage.local.get`, `storage.local.set`, `storage.local.remove`, `storage.onChanged`; `tabs.query`, `tabs.create`, `tabs.sendMessage`, `tabs.onUpdated`, `tabs.onActivated`.
Of the APIs asked about, present: storage (local only), runtime messaging, tabs.
Absent entirely: alarms, cookies, webRequest, declarativeNetRequest, offscreen, notifications, identity, downloads, scripting, permissions, contextMenus, windows, action, and `storage.sync`/`storage.session`.
`#[oxichrome::on]` can only name an event whose bridge function already exists, because it formats the identifier as `chrome_{namespace}_{event}_add_listener` and calls it (`oxichrome-macros/src/codegen/event_handler.rs:29,79`), so the reachable event set is exactly five: `runtime::on_installed`, `runtime::on_message`, `storage::on_changed`, `tabs::on_updated`, `tabs::on_activated`.
Anything outside that list has to be declared by hand with `#[wasm_bindgen]` in the user's own crate, which the colour-picker example already does for the EyeDropper API (`examples/color-picker/src/lib.rs:15-27`) — a workable but unassisted path.
`web-sys` is a dependency of `oxichrome-core` with only the `console` feature enabled (`oxichrome-core/Cargo.toml:16`), so DOM and `fetch` access is the consumer's own responsibility.

### Async and JS interop

Every async API is `js_sys::Promise` bridged through `wasm_bindgen_futures::JsFuture` (`oxichrome-core/src/storage.rs:9-41`, `tabs.rs:9-30`, `runtime.rs:8-17`).
Data crosses through `serde-wasm-bindgen` in both directions with `Serialize`/`DeserializeOwned` bounds (`storage.rs:21,27`, `tabs.rs:10,13`).
Errors funnel into `OxichromeError` with `Js`, `Serde` and `UnexpectedValue` variants and a `From<JsValue>` impl (`oxichrome-core/src/error.rs:36-58`).
`gloo` is not used anywhere.
Event handlers are wrapped as `Closure<dyn FnMut(JsValue, ...)>` that spawn the async body with `spawn_local` and then `forget()` the closure (`oxichrome-macros/src/codegen/event_handler.rs:73-80`).

Two consequences of that wrapper matter for a control-plane design.
The closure returns unit, so an `onMessage` handler cannot `return true` to hold the `sendResponse` channel open, which is the only MV3-supported way to answer a message asynchronously; request-response messaging with an async body is therefore not expressible.
And the closure body is detached via `spawn_local`, so the handler's completion is unobservable to the caller.

### Content scripts, popup, options and messaging

Content scripts are supported and were added by an outside contributor, not the maintainer (PR #2, merged 2026-03-25).
Each generates a standalone JS loader that dynamically imports the wasm through `chrome.runtime.getURL` rather than a static import, which is required because content scripts are not modules (`oxichrome-build/src/shims.rs:115-135`).
Note that the `matches` and `all_frames` arguments are discarded inside the proc macro (`oxichrome-macros/src/codegen/content_script.rs:9-10`); they reach the manifest only through the separate source re-parse.
Popup and options pages are Leptos CSR components mounted with `leptos::mount::mount_to_body` (`oxichrome-macros/src/codegen/popup.rs:22-25`, `options_page.rs:22-25`), served from a fixed HTML shell with no `<title>`, no favicon and an inline two-line stylesheet (`shims.rs:49-62,82-95`).
Leptos is an unconditional dependency of the facade crate with no feature gate (`oxichrome/Cargo.toml:21`), so every extension compiles it whether or not it has any UI.
The examples pin `leptos = "0.7"` while the facade pins `0.8.15` (`examples/counter-extension/Cargo.toml:12` against `oxichrome/Cargo.toml:21`), a skew that would put two Leptos majors in one graph.
Messaging between contexts is whatever `runtime.sendMessage`, `tabs.sendMessage` and `runtime.onMessage` provide; there is no typed channel, no `Port`/`connect` binding, and no request-response helper.

### Build pipeline and artifact

The pipeline is the CLI's own, not wasm-pack and not trunk.
It shells out to `rustup target list --installed` and will `rustup target add wasm32-unknown-unknown` if missing (`build.rs:160-178`), which makes `rustup` a hard requirement — on a Nix-managed toolchain without `rustup` the command fails at `.context("failed to run rustup")` before doing anything.
It then reads the desired `wasm-bindgen` version out of `Cargo.lock` by line-scanning and, on a mismatch, runs `cargo install wasm-bindgen-cli --version=<v>` unprompted (`build.rs:180-238`), so a routine build can reach the network and install a binary.
`wasm-opt -Oz` runs only if `wasm-opt` is already on `PATH`, and a failure is swallowed as non-critical (`build.rs:133-149`).
The artifact is `dist/<browser>/` containing `manifest.json`, `background.js`, the optional popup/options pairs, one JS file per content script, `wasm/<crate>.js` and `wasm/<crate>_bg.wasm`, plus anything copied from `static/`.

Bundle size is unmeasured.
The build was refused at this session's permission layer, which declines to compile an externally cloned repository without the founder naming it and authorizing the build; I did not work around that.
What can be said without building: `wasm-bindgen --target web` output plus a Leptos CSR runtime is the floor for any extension with a popup, and `wasm-opt -Oz` is optional rather than guaranteed.
Unblocking a measurement needs one founder instruction authorizing `cargo build` inside `~/ghq/github.com/0xsouravm/oxichrome`.

### Tests, CI, docs, examples, dev loop

38 unit tests, all in `oxichrome-build` (16), `oxichrome-macros` (15) and `oxichrome-cli` (7).
`oxichrome-core` — the entire runtime binding layer — has zero tests, and there is no `wasm-bindgen-test`, no headless browser test and no loaded-extension smoke test anywhere in the tree.
There is no CI: no `.github/` directory exists, so nothing verifies a pull request.
Doc comments are near-absent on the public runtime API: `storage.rs`, `tabs.rs`, `runtime.rs` and `error.rs` carry none, and the 30 doc lines in `oxichrome-macros/src/lib.rs` are `ignore`d usage snippets.
docs.rs pages build and return 200 for all three library crates, and https://oxichrome.dev resolves, but the API reference is effectively empty.
Two examples exist, `counter-extension` and `color-picker`, both wired to `path` dependencies.
There is no hot reload, no watch mode and no dev server — the loop is `cargo oxichrome build`, then reload the unpacked extension by hand (`main.rs:25-37` lists every subcommand).

## Maturity and risk

The project is one burst of work by one author followed by drive-by contributions.
The initial commit landed 2,330 lines across 40 files in a single commit on 2026-02-12, and the crates were published to crates.io roughly eleven minutes before that commit exists in git history (crates.io `created_at` 2026-02-11T19:14Z versus `git log` 2026-02-12 00:55 +0530).
Since then the maintainer has authored four commits, of which only one adds a feature (`feat: add firefox target and clean command`, 2026-02-14); every subsequent code change — content scripts, `run_at`/`all_frames`/`css`, the `wasm-opt` detection fix, the TOML-based crate-name fix, `host_permissions` — came from Jude Ndubuisi, Daniel Segovia or Kayo.
Review latency is long: PR #4 sat from 2026-05-16 to 2026-07-28, and PR #3 ("remove unused dependencies") has been open since 2026-05-09 without a decision.
The maintainer's own last commit was 2026-05-15; the repository's last activity of any kind was 2026-07-28, five weeks before today.

Adoption is very small.
1,304 total downloads across all five crates, of which `cargo-oxichrome` — which every user must install to build anything — has 169.
327 stars against 2 watchers is the shape of a project that got attention on an aggregator rather than users.

The published crate is materially behind the repository.
`v0.2.0` (2026-02-14) is the newest thing on crates.io, and 14 commits sit above it on `main`, including both content scripts and `host_permissions`; `git ls-tree v0.2.0` finds no content-script file and `git grep host_permissions v0.2.0` finds nothing.
Using either feature therefore requires a git dependency pinned to an unreleased commit, with no release cadence to return to.
The workspace version reads 0.2.1, a bump that was never published.

Dependency freshness is a mixed picture: `wasm-bindgen` is pinned at 0.2.108 against 0.2.127 current, `web-sys`/`js-sys` at 0.3.85 against 0.3.104, `leptos` at 0.8.15 against 0.8.20, `syn` at 2.0.114 while 3.0.4 is current, `thiserror` 2.0.18 against 2.0.20, `clap` 4.5.58 against 4.6.6 (all current versions from the crates.io API today).
Nothing is dangerously old; the pins reflect a July snapshot on a project that stopped moving.
No MSRV is declared anywhere, in the manifests or in the published metadata.

Code quality is better than the governance.
Zero `unsafe`.
Of the 35 `unwrap`/`expect` occurrences, 29 are inside `#[cfg(test)]` blocks.
The six in non-test code are `path_start.unwrap()` immediately after an `is_none()` guard (`oxichrome-macros/src/parse.rs:55-65`), `value.as_object_mut().unwrap()` on a value the same function just built (`manifest.rs:127`), and four `Path::to_str().unwrap()` calls that would panic on a non-UTF-8 path (`build.rs:72,75,138,140`).
One `panic!` sits in a proc macro, where a panic is a compile-time diagnostic rather than a runtime failure (`oxichrome-macros/src/parse.rs:400`).
Nothing in the tree is machine-generated in the code-generation sense: there is no `build.rs`, no bindgen step, no vendored output.

On whether the codebase is largely LLM-written, my confidence is low-to-moderate that it is, and the signals are circumstantial.
For: a complete five-crate workspace with tests, README badges, an OG image and a registered domain arriving in one commit; uniformly consistent style with no TODOs and no dead scaffolding; a README that overstates the surface ("Five proc macros" when six exist, `README.md:45`, and a proc-macro section that omits `content_script` and `host_permissions` entirely).
Against: the central design decision — re-parsing the source with `syn` at build time so the manifest can be produced without compiling — is a non-obvious choice with real trade-offs, and the `host_permissions` pattern validator carries specific citations to the Chrome and MDN match-pattern documents (`oxichrome-macros/src/parse.rs:4-5`), though that file came from an outside contributor.
The decision-relevant fact is independent of the answer: whichever way it was written, it was written once and its author has not touched the code in nearly four months.

Licence compatibility is clean at the top level.
oxichrome and all four sibling crates are MIT (`Cargo.toml:16`, `LICENSE:1`), which the project's `deny.toml` allow list admits directly (`deny.toml:5`: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib).
The transitive graph is the open question and is dominated by Leptos, which is itself MIT; a real answer needs `cargo deny check licenses` against a resolved graph, which cannot run here because the build is not authorized.

## Alternatives

`tam-core-wasm`, the pattern the project already ships.
Plain `wasm-bindgen` pinned exactly at `=0.2.121` with no `web-sys`, no `js-sys` and deliberately no `serde-wasm-bindgen`, passing JSON strings in both directions so the browser and the server read one wire format rather than two (`crates/tam-core-wasm/Cargo.toml:10-19`, `src/lib.rs:1-19`).
This is a computation module, not a browser-API consumer — it performs no I/O and touches no `chrome.*` namespace — so it is not an alternative to oxichrome so much as proof that the wasm-bindgen path is already established here, and that an extension could reuse the core rules unchanged.
What plain wasm-bindgen plus web-sys gives over oxichrome: every API oxichrome lacks, reachable by writing the same six-line `extern "C"` block oxichrome writes internally, at the cost of hand-writing `manifest.json`, the loader shims and the build script.

`web-extensions-sys` (https://github.com/web-extensions-rs/web-extensions-sys, examined at `16ff591`, 2025-10-12; MIT).
1,951,600 total downloads and 418,552 recent, though the reverse-dependency list on crates.io holds exactly one crate, so those numbers reflect something other than direct library adoption and should not be read as a user count.
It covers far more surface than oxichrome — `action`, `bookmarks`, `commands`, `context_menus`, `cookies`, `downloads`, `history`, `identity`, `omnibox`, `port`, `runtime`, `scripting`, `sessions`, `storage`, `tabs`, `windows` (`src/lib.rs:6-29`) — including `cookies`, `downloads`, `identity` and `scripting`, all of which oxichrome lacks, and `Port` for long-lived connections.
It still lacks `alarms`, `webRequest`, `declarativeNetRequest`, `offscreen` and `notifications` (`rg` over `src/` finds none).
Firefox is a compile-time feature that swaps the `browser` global for `chrome` and gates Firefox-only namespaces (`src/lib.rs:60-83`), so one build serves one browser, as with oxichrome.
It is bindings only: no macros, no manifest generation, no build tool, no UI story.
Last release 2025-10-11, and before that 2023-04-06 — slow but not abandoned, and the maintainers are three named people rather than one.

`web-extensions` (https://github.com/rvolosatovs/web-extensions, MIT), the higher-level wrapper over the above.
Last published 2022-11-16, 1,945 recent downloads; effectively dormant, and it is the sole reverse dependency of `web-extensions-sys`.

No other Rust extension framework with real activity surfaced.
`gh search repos --topic chrome-extension --language rust --sort stars` returns oxichrome as the only framework in the list; every other Rust result is an application that happens to ship an extension (Harper, ethui, Focuser) or a boilerplate repository, and `crates.io` searches for "chrome-extension", "webextension" and "browser-extension" return only adjacent tools — native-messaging hosts (`native_messaging`, 48,017 recent downloads, updated 2026-06-30), a store-publishing CLI (`wepub`), a manifest parser (`crx-manifest-parser`) and permission auditors — no competing framework.

## Bearing on the decision

Three facts dominate.

The API surface is roughly a tenth of what the product needs and is not extensible through the framework.
No `alarms` means an MV3 service worker has no supported way to run a deterministic cron-shaped schedule, which the project's own non-negotiable requires to run on the seller's device; no `offscreen`, no `scripting`, no `cookies`, no `declarativeNetRequest`.
Everything missing has to be hand-declared with `#[wasm_bindgen]` in our own crate, at which point oxichrome contributes the manifest writer and the macros, not the bindings.

The generated background worker has an MV3 correctness problem.
`background.js` registers every listener inside an `async function start()` after `await init()` (`oxichrome-build/src/shims.rs:35-44`), so registration is asynchronous; Chrome requires MV3 service-worker listeners to be registered synchronously at top level or events that should wake a terminated worker are not delivered.
Compounding it, `#[oxichrome::on]` closures return unit (`oxichrome-macros/src/codegen/event_handler.rs:73-77`), so an `onMessage` handler cannot `return true` and asynchronous request-response messaging cannot be expressed at all.
Both are in the framework's generated code, not in ours, so neither is fixable without forking.

The project is a fork-or-vendor proposition, not a dependency.
The features we would need most, content scripts and `host_permissions`, exist only above the last release; `cargo-oxichrome` has 169 downloads; there is no CI, no test covering the runtime bindings, and no maintainer commit since 2026-05-15.
Its licence is MIT and passes the `deny.toml` allow list, so vendoring is legally straightforward; the question is whether we want to own roughly 2,700 lines of build tooling, or write the manifest and loader shims ourselves against `web-extensions-sys` plus the `tam-core-wasm` pattern already in the tree.

## Questions

1. May I compile the external clone at `~/ghq/github.com/0xsouravm/oxichrome` so the smallest example's bundle size can be measured, and so `cargo deny check licenses` can resolve the transitive graph?
   The permission layer refused it on the grounds that a teammate message does not meet the consent bar for building third-party code; both remaining unmeasured items need it.
2. Is Firefox genuinely in scope for launch, or Chrome first?
   oxichrome's Firefox support is a per-build target with a non-configurable gecko id and no `strict_min_version`, and `web-extensions-sys` puts the browser behind a compile-time feature — either way one build serves one browser, which changes the release matrix.
