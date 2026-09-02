# Leptos as our frontend: framework and migration assessment

Whether to replace the SvelteKit console with Leptos plus rust-ui.com and Tailwind, for the founder's stated reasons "fast, native and simple".

- date: 2026-09-03
- method: read-only inspection of this working tree (no jj or git commands run, no dependency added, no build started) plus read-only retrieval of public primary sources; every external claim carries its URL and the retrieval date 2026-09-03
- scope: the framework and the migration; rust-ui.com component coverage is a separate agent's assignment and is deliberately not assessed here
- inherits: `docs/research/rethink/client-surfaces-and-cross-compile.md` (2026-09-02) for the Tauri and cross-compilation findings, which this note does not re-derive

## 1. Executive summary

Do not switch, and the deciding evidence is not taste but maintenance: on 2026-05-08 the Leptos author opened "Status Update - May 2026" saying "Leptos is not abandoned but will be lightly maintained going forward" and "I consider it feature-complete and do not expect to do significant new development in the future".
The version picture is consistent with that: 0.8.20 is the newest stable, published 2026-06-25, and 0.9.0-beta has sat since 2026-07-18 with the author writing "I don't feel much urgency about it, and it doesn't contain any significant new features".
The founder's word "native" is false as stated: a Leptos app inside Tauri is HTML and CSS in WKWebView, WebView2 or WebKitGTK, exactly the same substrate our SvelteKit console already runs in there, so switching frontend framework changes nothing about how native the product is.
"Fast" is true only for a metric we do not have a problem with — synthetic DOM update throughput — while the Leptos book itself states "WebAssembly binaries are significantly larger than the JavaScript bundles you'd expect for the equivalent application", against our measured 74 KB gzipped first-load payload.
"Simple" is false at system level: better-auth stays a Node service by founder non-negotiable, Turnstile, Paddle and WebAuthn stay JavaScript interop, and we would add trunk, wasm-bindgen CLI version pinning, leptosfmt and an `erase_components` RUSTFLAG to the toolchain rather than removing anything.
The inner development loop degrades by one to two orders of magnitude: Vite hot module replacement in the sub-second range becomes a Rust-to-wasm rebuild plus a full page reload, measured at 5 to 7 seconds on a tuned 20 kLOC Leptos app and 16 seconds untuned.
The ecosystem substitute for our most-used dependency is weak: `@tanstack/svelte-query` appears in 19 import sites here, and the closest Leptos analogue `leptos_query` has not published since 2024-03-09, two breaking Leptos minors ago.
The genuine prize the founder is reaching for — client-side validation and projection preview from our own Rust domain — is available today without Leptos: `tam-types`, `tam-domain`, `tam-taxonomy` and `tam-marketplace` together pull only thirteen external crates, all pure Rust, and compile to `wasm32-unknown-unknown` for a Svelte frontend just as readily as for a Leptos one.
The migration is 90 to 140 engineer-days on the numbers in section 8, against a frontend that was redesigned to an approved mockup two days ago and is not the thing blocking launch.
Verdict: no now, no under the "Leptos for Tauri only" variant, and a defensible yes only if Leptos 0.9 ships stable with a second active maintainer and we independently decide the web console is disposable.

## 2. Leptos maturity today

The newest stable release is 0.8.20, published 2026-06-25, and the newest published version of any kind is 0.9.0-beta from 2026-07-18 (<https://crates.io/api/v1/crates/leptos>, retrieved 2026-09-03).
The GitHub repository is alive — 21,250 stars, 891 forks, 131 open issues, last push 2026-09-01, not archived (<https://api.github.com/repos/leptos-rs/leptos>, retrieved 2026-09-03) — so this is not an abandoned project, and 131 open issues on a framework of this size is a healthy rather than alarming number.

Breaking-change cadence over the period the founder cares about, from <https://lib.rs/crates/leptos/versions> and <https://crates.io/api/v1/crates/leptos/versions> (both retrieved 2026-09-03): 0.5.0 on 2023-09-29, the 0.6 line from 2024-01-25, 0.7.0 on 2024-11-30, 0.8.0 on 2025-05-01, 0.9.0-beta on 2026-07-18.
That is four semver-breaking minors in under three years, and the current one has been in beta for six weeks with no stable date.
The 0.9.0-beta release notes describe further API change — deref-based function-call syntax for signals on stable Rust, a migration to `serde_qs` 1.0, typed `ToggleEvent` handling (<https://api.github.com/repos/leptos-rs/leptos/releases>, retrieved 2026-09-03) — so a team adopting 0.8 today is adopting a version with a known breaking successor in flight.

Stated stability is mixed and should be read with the maintenance statement beside it.
The repository README says "The APIs are basically settled" and "I would not expect major breaking changes to your code to adapt to future releases" (<https://raw.githubusercontent.com/leptos-rs/leptos/main/README.md>, retrieved 2026-09-03).
Issue #4707, "Status Update - May 2026", opened by the author gbj on 2026-05-08, says "Leptos is not abandoned but will be lightly maintained going forward", "I consider it feature-complete and do not expect to do significant new development in the future", "I am open to additional maintainers who want to take a more active role", and "I'm very open to anyone who wants to help maintain the project in an active way" (<https://github.com/leptos-rs/leptos/issues/4707>, retrieved 2026-09-03).
Those two statements are compatible — a feature-complete framework is a stable framework — but for a founder choosing a substrate for a product's whole client surface, "lightly maintained" and "open to additional maintainers" describe bus-factor risk that SvelteKit does not carry.

Production users named by the project: none.
The README says only "There are several people in the community using Leptos right now for many websites at work", naming no company, and leptos.dev carries no "who uses Leptos" section (<https://leptos.dev/>, retrieved 2026-09-03).
That absence is not proof of no adoption, but it means we cannot point at a single named company running Leptos in production, which is the kind of reference the founder would want before betting the client on it.

Rendering modes are client-side rendering, server-side rendering with hydration, static site generation, and islands.
Islands is the youngest: the book describes it as reflecting "work at the cutting edge of what frontend web frameworks are exploring right now", routes readers to an out-of-tree `islands_router` example rather than documenting the router inline, and lists persistent islands and view-transition animations as proposed rather than implemented (<https://book.leptos.dev/islands.html>, retrieved 2026-09-03).
For our two surfaces the mode choice is forced and identical in both cases: the Tauri webview must be CSR or SSG, because Tauri's own Leptos guide says "Use SSG, Tauri doesn't officially support server based solutions" (<https://v2.tauri.app/start/frontend/leptos/>, retrieved 2026-09-03); and the public web console is a logged-in dashboard served today by `tam-server --ui-dir` behind a `ServeDir` fallback (`crates/tam-server/src/main.rs:307`), which is static hosting, so it is CSR too.
The practical consequence is that we would take on a framework whose design centre and marketing centre is server-side rendering with server functions, and use neither: Leptos server functions assume Leptos owns the Axum server, and ours is `tam-api`, so the frontend would call our existing HTTP endpoints exactly as the Svelte client does today.
Adopting Leptos therefore buys us the part of Leptos that is least differentiated and skips the part that is.

## 3. Tauri v2 integration

There is an official template: `create-tauri-app` lists Leptos under its Rust frontends alongside Yew and Sycamore, with a non-interactive preset named `leptos` (<https://raw.githubusercontent.com/tauri-apps/create-tauri-app/dev/README.md>, retrieved 2026-09-03).
Tauri also publishes a Leptos integration page, and its content is the first friction signal: the page states it is accurate "as of Leptos version 0.6" — three minors and two and a half years behind 0.8.20 (<https://v2.tauri.app/start/frontend/leptos/>, retrieved 2026-09-03).

The documented configuration is `beforeDevCommand: "trunk serve"`, `devUrl: "http://localhost:1420"`, `beforeBuildCommand: "trunk build"`, `frontendDist: "../dist"`, plus a `Trunk.toml` with `[watch] ignore = ["./src-tauri"]` and `[serve] ws_protocol = "ws"`.
Two of its three checklist items are caveats rather than steps: "Use SSG, Tauri doesn't officially support server based solutions", and use `ws_protocol = "ws"` "so that the hot-reload websocket can connect properly for mobile development".
The third is `withGlobalTauri: true`, so that "Tauri APIs are available in the `window.__TAURI__` variable and can be imported using `wasm-bindgen`" — which is to say the documented path from Leptos to Tauri is through the JavaScript global, reached by hand-written `wasm-bindgen` externs.

Typed Rust bindings exist but are not first-party and not on crates.io.
`tauri-sys` is the community binding, modelling event listeners "as async streams of data using the futures::Stream trait instead of using callbacks", and it is git-only: `https://crates.io/api/v1/crates/tauri-sys` returns 404 (retrieved 2026-09-03), while the repository is live with 124 stars and 25 open issues, last pushed 2026-07-19 (<https://api.github.com/repos/JonasKruckenberg/tauri-sys>, retrieved 2026-09-03).
Alternatives are `tauri-interop`, which generates a wasm function per `#[tauri::command]` and names Leptos in its dependency set, and `tauri-wasm`, a thinner invoke wrapper (<https://docs.rs/crate/tauri-interop/1.2.0> and <https://docs.rs/tauri-wasm>, retrieved 2026-09-03).
None of these is Tauri-maintained, and pinning our IPC layer to a 124-star unpublished crate is a dependency the current SvelteKit path does not have — from TypeScript, `@tauri-apps/api` is a first-party npm package.

The documented friction is `wasm-bindgen` version coupling.
The `wasm-bindgen` CLI schema must exactly match the crate version, and the failure mode is a build error stating that "the bindgen format is unstable enough that these two schema versions must exactly match"; it is reported to appear specifically when a working standalone Leptos CSR project is moved into a Cargo workspace, which is exactly the move we would make (<https://users.rust-lang.org/t/while-using-trunk-getting-error-about-wasm-bindgen-version-only-inside-of-workspace/96073>, retrieved 2026-09-03).
Our repository is a Cargo workspace with a founder-gated `Cargo.toml` lints table and `deny.toml`, so a pinned CLI binary plus a pinned crate version becomes another gated shared surface.

Mobile imposes nothing extra on Leptos specifically beyond what our prior note already recorded for Tauri: the webview is WKWebView on iOS and the Android System WebView, `WebviewWindow::cookies` is "**Android**: Unsupported, always returns an empty `Vec`", and several plugins are untested on mobile (`docs/research/rethink/client-surfaces-and-cross-compile.md` §4.1, 2026-09-02).
Those costs are identical whether the webview renders Svelte or Leptos, so they do not discriminate between the options.

## 4. Developer experience, honestly

Compile times are the material cost, and the primary sources are unusually candid about it.
The Leptos book states that 0.7's renderer "relied more heavily on the type system", which "can lead to slower compile times", and that the mitigation is a custom cfg flag: "Most of the slowdown in compile times can be alleviated by using the custom configuration flag `--cfg=erase_components` during development", at "the expense of additional binary size and runtime cost", auto-enabled in dev by cargo-leptos 0.2.40 and manual everywhere else including Trunk (<https://book.leptos.dev/getting_started/leptos_dx.html>, retrieved 2026-09-03).
A profiled real-world data point, and the best one available: a 20 kLOC Leptos application rebuilt in "~16 seconds" on Linux out of the box, 7 seconds after adding the mold linker, and 5 seconds after tuning dev-profile opt-levels; the backend build was "almost 95% link time"; and cargo-leptos hot reloading "didn't make a perceptible impact on responsiveness" (<https://bitemyapp.com/blog/rebuilding-rust-leptos-quickly/>, 2024-12-01, retrieved 2026-09-03).
A filed issue reports a cold dev build of 32.47 s and an incremental of 18.23 s on Leptos 0.7, proposing targets of under 15 s cold and under 5 s incremental; it was closed as not planned (<https://github.com/leptos-rs/leptos/issues/4290>, opened 2025-09-12, retrieved 2026-09-03).
Issue #3489 remains the maintainers' open collection point for compile-time reports.
Set that beside what we have: `vite dev` with Svelte 5 gives module-level hot replacement that preserves component state, and our own `npm run check` and `vitest` complete in seconds.

Hot reload does not exist for Leptos CSR in the sense a Vite user means.
Trunk's own README describes only that "Trunk watches your application for changes and triggers builds for you, including automatic browser reloading", and the words "hot reload" and "HMR" do not appear in it (<https://raw.githubusercontent.com/trunk-rs/trunk/main/README.md>, retrieved 2026-09-03).
Automatic browser reloading is a full page reload: every in-progress form, every open dialog, every expanded panel resets on each save.
For our largest screen, the 507-line listing creation form, that is a materially worse authoring loop than the one we have.

Debugging is workable but needs setup: the book recommends `console_error_panic_hook` so a wasm panic yields "an actual Rust stack trace that includes a line in your Rust source code" instead of "Unreachable executed" (<https://book.leptos.dev/getting_started/leptos_dx.html>).
Source-mapped Rust in browser devtools is possible but is not the peer of stepping through TypeScript that Vite gives us today.

IDE support is the honest weak point and the book says so.
"Because of the nature of macros (they can expand from anything to anything, but only if the input is exactly correct at that instant) it can be hard for rust-analyzer to do proper autocompletion", and the documented workaround is to add proc macros to rust-analyzer's ignore list — with the acknowledged cost that "this means that rust-analyzer doesn't know about your component props, which may generate its own set of errors or warnings in the IDE".
Feature-gated code needs `rust-analyzer.cargo.features = "all"` to avoid `csr`/`ssr`/`hydrate` code showing as inactive.
`cargo fmt` cannot format inside `view!` — "cargo-fmt has a harder time auto-formatting your code that's inside the `view!` macro" — so `leptosfmt` becomes a required extra tool, wired per editor, and in RustRover via a file watcher.
The book does not document `view!` error-message quality; from the macro architecture, a malformed `view!` reports at the macro call site rather than at the offending line, and I found no primary source measuring this, so I record it as UNVERIFIED rather than asserting it.

## 5. Runtime claims, and what "native" does and does not mean

Bundle size, measured on our side rather than estimated: our current production build in `web/build` is 361,942 bytes of JavaScript across 78 files, 125,120 bytes gzipped, plus a 27,907-byte stylesheet that gzips to 6,643 bytes.
The payload actually fetched for a first paint — the 31 files `index.html` references — is 219,362 bytes raw and 74,153 bytes gzipped.
So the number Leptos would have to beat for the public console is roughly 74 KB gzipped of JavaScript for the shell plus route-level code splitting for everything else.
The Leptos book's own framing is that "WebAssembly binaries are significantly larger than the JavaScript bundles you'd expect for the equivalent application", and its optimisation chapter lists a size-tuned `wasm-release` profile with `opt-level = 'z'`, `lto = true` and `codegen-units = 1`, nightly `build-std` with `panic_immediate_abort`, replacing serde with `miniserde` or `serde-lite`, avoiding `regex` default features because they add "about 500kb to a WASM binary", and code splitting via `cargo leptos --split` with `#[lazy]` (<https://book.leptos.dev/deployment/binary_size.html>, retrieved 2026-09-03).
I could not retrieve a measured wasm byte count for a Leptos CSR application of comparable scope from a primary source, so the comparison stands as: our number is measured, theirs is not available, and the framework's own documentation says to expect it to be larger.

Interaction performance: I attempted the js-framework-benchmark official results at <https://krausest.github.io/js-framework-benchmark/current.html> and <https://krausest.github.io/js-framework-benchmark/2026/chrome150.html> on 2026-09-03 and both are JavaScript-rendered, returning no numeric rows to a fetch.
I therefore have no citable per-row numbers for Leptos versus Svelte 5 and record them as UNVERIFIED.
What is citable is the shape of the claim and its irrelevance: Leptos's fine-grained reactivity updates only the nodes bound to a changed signal, and the README claims "extremely performant code with minimal overhead" and that Leptos "is simply much faster at both creating and updating the UI than Yew is".
Our dashboard's latency is dominated by marketplace round-trips and by the API, not by DOM diffing over the tens-to-hundreds of rows these screens render; nothing in this product creates and swaps ten thousand table rows, which is what that benchmark measures.

On "native", the answer is unambiguous and it is the most important correction in this note.
A Leptos app inside Tauri is HTML, CSS and DOM in the platform webview: WebView2 on Windows, WebKit on macOS, WebKitGTK on Linux, WKWebView on iOS, the Android System WebView on Android (`docs/research/rethink/client-surfaces-and-cross-compile.md` §4.1, citing wry, 2026-09-02).
Our SvelteKit console inside Tauri renders in exactly the same engines.
Switching to Leptos changes which language emits the DOM, not what draws the pixels, so it delivers zero native-ness.
What "native" would actually mean is a Rust UI toolkit that draws its own widgets: `egui` in immediate mode via `eframe`, `iced` in the Elm style, Slint with a declarative DSL, or Dioxus Native over the Blitz renderer.
Blitz is not a candidate: the Dioxus 0.7 announcement (2025-09-08) says "Blitz is still very young and doesn't always produce the best outputs", "Blitz is still considered a 'work in progress'", "Not every CSS feature is supported yet", and "We have not focused on performance" (<https://dioxuslabs.com/blog/release-070/>, retrieved 2026-09-03).
Slint is viable technically and its desktop licensing is workable — a royalty-free licence covering proprietary desktop, mobile and web applications, GPLv3, or a commercial licence (<https://github.com/slint-ui/slint/blob/master/LICENSE.md> and <https://github.com/slint-ui/slint/blob/master/FAQ.md>, retrieved 2026-09-03) — but choosing any of these means we own our own table, dialog, form, chart, text-input and accessibility stack for a data-heavy dashboard, we abandon Tailwind and rust-ui.com entirely, we abandon the web console (none of these render to a browser at production quality), and we abandon the approved mockup.
For a cross-listing dashboard, the webview is the correct choice, and that is a point in favour of the architecture we already have rather than against it.

## 6. Ecosystem fit for a data-heavy dashboard

Data fetching and caching is where the substitution is weakest, and it matters here more than anywhere: `@tanstack/svelte-query` appears in 19 import sites across `web/src`, with 58 `createQuery` and 11 `createMutation` call sites.
The direct Leptos analogue, `leptos_query`, has published nothing since 0.5.3 on 2024-03-09 — before Leptos 0.7 and before 0.8 (<https://crates.io/api/v1/crates/leptos_query>, retrieved 2026-09-03).
The live replacement is `leptos-fetch`, "Async query manager for Leptos", at 0.4.10 published 2026-03-02 with 31,120 recent downloads, maintained by one person (<https://crates.io/api/v1/crates/leptos-fetch> and <https://github.com/zakstucke/leptos-fetch>, retrieved 2026-09-03).
Leptos also ships `Resource` and `Action` in-framework, which cover fetching and mutation but not the cache invalidation, background refetch and shared-key deduplication semantics our 58 query sites rely on.

Routing is in-framework via `leptos_router` and is not a gap.
Forms and validation have no framework-level story comparable to what we would need for the 507-line creation form; validation would be hand-written, which is also true today in TypeScript.
Tables are covered by `leptos-struct-table`, which derives a table from a struct and does offer optional sorting, row virtualisation, pagination, column hiding and drag reordering, with releases through 0.18.0 on 2026-02-03 (<https://github.com/Synphonyte/leptos-struct-table>, retrieved 2026-09-03) — a genuine ecosystem strength.
Charts are `leptos-chartistry` at 0.2.3, an extensible `<Chart>` component covering lines, bars, legends, axes and guides (<https://github.com/feral-dot-io/leptos-chartistry>, retrieved 2026-09-03), or JavaScript interop; our analytics page is 164 lines today, so either is affordable.
Internationalisation is `leptos_i18n` at 0.6.2, published 2026-04-14, 82,888 recent downloads, with a documented integration into `leptos-struct-table` column titles (<https://crates.io/api/v1/crates/leptos_i18n>, retrieved 2026-09-03).
Testing is three layers: plain `#[test]` on logic extracted out of components, `wasm-bindgen-test` under `wasm-pack test` for component and DOM assertions with the caveat that "the reactive system is built on top of the async system, so changes are not reflected synchronously in the DOM" so tests must await a tick, and Playwright or Cucumber for end-to-end (<https://book.leptos.dev/testing.html>, retrieved 2026-09-03).
Our 248 existing Vitest cases across 21 files run in Node against extracted logic modules; the logic-layer half of them ports to `#[test]` cleanly, and the component-level half moves to a browser-driven `wasm-bindgen-test` harness that is slower and heavier than `vitest run`.

Calling better-auth from a Rust WASM frontend is the sharpest integration cost, and it is worth being concrete because `web/src/lib/auth-client.ts` is 433 lines that exist only to sit on this boundary.
Today that module uses `createAuthClient` from `better-auth/svelte` with `passkeyClient()`, `jwtClient()` and `adminClient()`, and performs the documented session exchange: fetch the identity service's login assertion, post it same-origin, discard it, and hold the API's own HttpOnly cookie.
From WASM there is no better-auth Rust client, so we would either call better-auth's HTTP endpoints directly with `gloo-net` or `reqwest`'s wasm backend, or keep the JavaScript SDK and bind to it through `wasm-bindgen`.
The cookie half is fine either way, because the browser attaches the HttpOnly cookie to same-origin requests without the code touching it, and `/api/auth` is already same-origin by proxy in dev and by ingress in production.
Passkeys are the part that cannot be moved: WebAuthn is `navigator.credentials`, and while `web-sys` does expose `PublicKeyCredential` behind its own crate feature, several of its methods including `to_json`, `parse_creation_options_from_json` and `parse_request_options_from_json` additionally require `--cfg=web_sys_unstable_apis` (<https://docs.rs/web-sys/latest/web_sys/struct.PublicKeyCredential.html>, retrieved 2026-09-03).
Reimplementing better-auth's passkey ceremony against unstable web-sys bindings is strictly worse than calling the SDK, so the realistic answer is that the JavaScript identity client survives the migration and Leptos calls into it — which means the "one language" claim already fails at the login screen.
The same holds for Cloudflare Turnstile (`Turnstile.svelte`, 110 lines) and Paddle (`paddle.ts`, 159 lines), both of which are third-party JavaScript widgets loaded into the page.

## 7. Sharing our Rust core with the browser

This is the one argument for a Rust frontend that is real, and it does not require Leptos.
Measured here with `cargo tree --offline -e normal -p tam-types -p tam-domain -p tam-taxonomy`, the closure over those crates plus `tam-marketplace` is thirteen external crates: `itoa`, `memchr`, `proc-macro2`, `quote`, `serde`, `serde_core`, `serde_derive`, `serde_json`, `sha1_smol`, `syn`, `unicode-ident`, `uuid`, `zmij`.
Every one is pure Rust, there is no `libc`, no `*-sys`, no `getrandom` and no `cc`, and `uuid` is present with the `v5` feature only, which is SHA-1 based and needs no randomness source — consistent with, and tighter than, the 48-crate figure recorded for the full pure core in `client-surfaces-and-cross-compile.md` §2 (2026-09-02), which additionally covered `tam-pipeline`'s image and zip dependencies.
The conclusion is the same and now narrower: the crates that carry our domain rules compile to `wasm32-unknown-unknown` with no feature flags, no shim and no C toolchain.

What that buys is not type shapes but behaviour.
Generating TypeScript types from Rust gives us shapes that cannot drift, and nothing more; the rules would still be reimplemented in TypeScript, and today they are — `authoring.ts` is 718 lines, `publish-readiness.ts` 202, `listings-view.ts` 191, `tes-portfolio.ts` 183, `dashboard.ts` 268.
Compiling `tam-domain` and `tam-taxonomy` to wasm instead lets the client run the real publish-readiness predicate, the real vocabulary projection and the real mapping preview, so a listing the client says is publishable is publishable by the same code the server runs.
That is a genuine correctness gain and it is the strongest thing in the founder's instinct.
It is also entirely available from Svelte: `wasm-bindgen` exposes those functions to JavaScript, Vite loads the wasm module, and the Svelte components call it.
The only advantage Leptos adds is removing one serialisation hop at the boundary, which for form validation at human typing speed is not a cost we can measure.
Estimated at 5 to 10 engineer-days to wire a wasm build of `tam-domain` and `tam-taxonomy` into the existing SvelteKit app and delete the duplicated TypeScript rules, against 90 to 140 engineer-days to get the same property by rewriting the frontend.

## 8. Migration cost against our real tree

The surface, counted on 2026-09-03: 12,330 lines across `web/src`, of which 5,450 are Svelte and 6,880 TypeScript; 29 route pages, 2 layouts, 3 route load functions, 11 library components, 25 non-test library modules (4,315 lines, including a 204-line generated `vocab.ts`), and 21 test files carrying 248 test cases in 2,565 lines.
No Svelte transitions or animations are used anywhere, which removes one class of rewrite risk; 53 dialog and ARIA attributes across the components mean accessibility behaviour is hand-built and would have to be rebuilt.

| Bucket | Units | Lines | Difficulty | Notes |
|---|---|---|---|---|
| Stub pages | 6 (`help`, `library`, `notifications`, `purchases`, `templates`, `jobs`) | ~54 | trivial | 13 lines or fewer each |
| Presentational components | 5 (`StatCard`, `Panel`, `PageHead`, `Placeholder`, `ImpersonationBanner`) | 116 | trivial | markup and props only |
| List and table screens | 10 (`listings`, `queue`, `analytics`, `connections`, `status`, `sync`, `admin/*`) | ~1,400 | moderate | query hooks plus tables; `leptos-struct-table` covers most |
| Dialogs and fields | 4 (`PublishDialog`, `DeleteDialog`, `UploadField`, `AxisField`) | 549 | moderate | focus management and file upload rebuilt by hand |
| Live-progress surfaces | 4 sites (`Console.svelte`, `queue`, `sync/[id]`, `listings/[id]`, plus `query.ts`, `ledger.ts`) | ~700 | hard | `EventSource` via `web-sys`, reconnection and cache patching |
| Authoring form | 2 (`listings/new` 507, `listings/[id]` 458) | 965 | hard | vocabulary-driven fields, validation, upload, publish readiness |
| Settings and billing | 1 (`settings` 421) plus `paddle.ts` | 580 | hard | passkey ceremony and Paddle JS interop, both staying JavaScript |
| Auth screens | 4 (`login`, `signup`, `reset`, `reset/confirm`) plus `auth-client.ts`, `Turnstile.svelte` | 1,082 | hard | better-auth SDK interop is unavoidable; see §6 |
| Pure logic modules | ~18 modules | ~2,000 | moderate | translate to Rust, or delete in favour of `tam-domain` |
| Tests | 21 files, 248 cases | 2,565 | moderate | logic half to `#[test]`, component half to `wasm-bindgen-test` |

Estimate, with assumptions stated so the founder can argue with a component rather than a total.
Assumption one: the design is fixed and does not need re-deciding, so this is a port and not a redesign.
Assumption two: sustained throughput of 120 to 200 lines of finished, reviewed target code per engineer-day, which is roughly half a familiar-stack rate, discounted for an unfamiliar framework and a 5-to-30-second edit loop rather than a sub-second one.
Assumption three: better-auth, Turnstile and Paddle remain JavaScript and are reached through `wasm-bindgen` rather than reimplemented.
On those assumptions: view and component rewrite 30 to 45 days; logic modules 12 to 18 days; test suite 10 to 15 days; toolchain, routing, SSE, IPC, Tailwind pipeline, nix and CI integration 15 to 20 days; design-fidelity re-verification against the approved mockup 4 to 6 days; buffer at 30 percent.
Total 90 to 140 engineer-days, that is 18 to 28 engineer-weeks, which for one engineer is four to seven calendar months and even under aggressive agent parallelism is six to ten weeks of wall clock during which the console does not gain a feature.
Set against `client-surfaces-and-cross-compile.md`'s estimate of 10 to 16 weeks for the seam work plus a shipping Tauri desktop client, this migration is comparable in size to the entire desktop programme and delivers no user-visible capability.

Ongoing cost delta.
Build times move from a sub-second Vite hot-replacement loop and a seconds-long `vitest run` to a Rust-to-wasm rebuild plus full page reload, benchmarked at 5 to 7 seconds tuned and 16 seconds untuned on a comparable-size application, with cold builds in minutes; `just check` and `just web-check` collapse into one lane that is slower than either.
Hiring pool: `svelte` drew 22,785,434 npm downloads between 2026-07-31 and 2026-08-29 (<https://api.npmjs.org/downloads/point/last-month/svelte>, retrieved 2026-09-03), against 1,261,156 recent (90-day) downloads for `leptos` on crates.io, roughly 420,000 a month — a ratio near 54 to 1.
Agent fluency, which matters most here because this codebase is written by agents, cuts both ways and I will not pretend otherwise.
In Leptos's favour: the Rust type system rejects far more wrong edits than `svelte-check` does, so an agent's mistake fails loudly rather than at runtime.
Against it: the corpus is 54 times smaller; it is contaminated across four breaking minors, so a plausible-looking 0.6-era signal call no longer compiles under 0.8 and will change again under 0.9's deref-call syntax; rust-analyzer's own documentation concedes it struggles inside proc macros and recommends ignoring `#[component]`, which removes exactly the prop-completion signal an agent leans on; and every failed compile costs 5 to 30 seconds instead of 200 milliseconds, so the agent loop's cost is iteration count multiplied by a latency an order of magnitude worse.
Svelte 5 runes carry their own corpus contamination against Svelte 4 stores, so this is a difference of degree; but our tree already has 248 green tests and a redesigned console as the agents' feedback surface, and that asset is discarded by the rewrite.

## 9. Verdict

Not worth the switch.
The conditions under which it would become worth revisiting are specific: Leptos 0.9 ships stable, a second active maintainer is publicly committed, and we have independently decided the web console is disposable so that a single Rust client is the whole product.
None of those holds on 2026-09-03, and the first two are outside our control.

The "Leptos for the Tauri client only, Svelte for the web console" variant is worse than either pure option and should be rejected outright: it produces two frontends implementing the same screens in two languages, doubling the maintenance of every future feature, while `client-surfaces-and-cross-compile.md` §4.1 already establishes that Tauri consumes our existing `adapter-static` build with an `index.html` fallback unmodified.
The reverse — Leptos for the web console, Svelte for the desktop — has the same defect and additionally puts the immature surface in front of paying customers.

If the founder still wants Rust closer to the client, the ordered path that captures most of the value at a fraction of the cost is: land the transport feature gate and the engine-to-remote-ledger split that every client surface needs anyway; compile `tam-domain` and `tam-taxonomy` to `wasm32-unknown-unknown` and call them from the existing Svelte console, deleting the duplicated TypeScript rules; ship the Tauri desktop client on the existing frontend; and revisit Leptos only if a concrete problem survives all three.

The three stated reasons, each answered directly.

Fast — no, not for us.
Leptos's fine-grained reactivity is genuinely faster at DOM updates, but our first-load payload is a measured 74 KB gzipped and the Leptos book states wasm binaries are "significantly larger than the JavaScript bundles you'd expect for the equivalent application"; inside Tauri the bundle is local so neither figure matters; and our latency is dominated by marketplace round-trips, not by rendering.
I could not retrieve js-framework-benchmark rows to quantify the update-throughput advantage, and I record that as UNVERIFIED, but it would not change the answer because the benchmark measures a workload this product does not have.

Native — no, and this is the reason to correct first.
A Leptos app in Tauri is HTML in WKWebView, WebView2 or WebKitGTK, the identical substrate our SvelteKit console already runs in there; the only genuinely native Rust options are egui, iced, Slint and Dioxus Native, and adopting any of them means owning the entire widget and accessibility stack for a data-heavy dashboard, abandoning Tailwind, rust-ui.com, the web console and the approved mockup — with Dioxus Native additionally described by its own authors as a "work in progress" on which "We have not focused on performance".

Simple — no.
better-auth stays a Node service by founder non-negotiable, and its passkey ceremony, Turnstile and Paddle all stay JavaScript reached through `wasm-bindgen`, so we do not reach one language; we would add trunk, a version-pinned `wasm-bindgen` CLI, `leptosfmt`, an `erase_components` RUSTFLAG, an unpublished IPC crate and a `wasm32` toolchain target, while removing only Vite and Vitest; and we would trade a sub-second edit loop for a multi-second one.
One language across the stack is a real simplification in principle, and it is the version of "simple" the founder is reaching for — but it is not what this change delivers.

## 10. Open questions for the founder

1. Is the objection to the current frontend actually about Svelte, or about the JavaScript toolchain — npm, Vite, the lockfile, supply chain? If the latter, the cheaper answers are the Tailwind standalone binary, which the Tailwind docs confirm is "available as a standalone executable if you want to use it without installing Node.js", and vendoring the web build into the nix flake, neither of which needs a rewrite.
2. Would the founder accept "Rust domain rules running in the browser via wasm, inside the Svelte console" as satisfying the intent behind the Leptos request? That is 5 to 10 engineer-days and captures the correctness argument in §7 entirely.
3. Does the "lightly maintained" status quoted in §2 change the answer on its own, or would the founder still consider Leptos if a second maintainer stepped up? The answer determines whether this is closed or parked.
4. What is the launch date the console is being judged against? A 90-to-140-engineer-day rewrite of a frontend redesigned to an approved mockup two days ago is only arguable if launch is far enough out that the cost is invisible.
5. Was rust-ui.com the actual driver — that is, is the request really "I want that component library" rather than "I want Leptos"? The component-coverage agent's finding should be read against this note before either is acted on.

## 11. Unverified

- js-framework-benchmark numeric rows for Leptos and Svelte 5. Both <https://krausest.github.io/js-framework-benchmark/current.html> and <https://krausest.github.io/js-framework-benchmark/2026/chrome150.html> are JavaScript-rendered and returned no table data to a read-only fetch on 2026-09-03. Geometric-mean duration, startup metrics and per-framework transfer weight are therefore unquantified here.
- A measured wasm byte count for a Leptos CSR application of comparable scope to our console. No primary source found; community figures exist but are estimates and are not cited.
- `view!` macro error-message quality. The Leptos book documents formatting difficulty and rust-analyzer limitations but says nothing about diagnostic quality; no primary source measures it.
- rust-ui.com's framework pinning, Leptos version support and Tailwind version. <https://rust-ui.com/> returned HTTP 403 to a read-only fetch on 2026-09-03; this is the parallel agent's assignment.
- Leptos 0.6.0's exact stable release date. <https://api.github.com/repos/leptos-rs/leptos/releases/tags/v0.6.0> returned 404; lib.rs shows 0.6.0-rc1 on 2024-01-25 and 0.6.1 on 2024-01-26, so the 0.6 line is dated to January 2024 rather than to a precise 0.6.0 timestamp.
- Trunk's serve and watch option surface. <https://trunkrs.dev/commands/> returned HTTP 403 on 2026-09-03; the reload behaviour quoted in §4 comes from the project README instead.
- Cold-build time for a Leptos CSR application inside a Cargo workspace of our size. The 5-to-16-second figures in §4 are incremental rebuilds on a 20 kLOC application, not our tree; no measurement of our tree was attempted, since building was out of scope for this note.
