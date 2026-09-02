# Crux and Dioxus: what, if anything, they apply to here

The founder asks what Crux and Dioxus are for, and whether either belongs in Teachouse.

- date: 2026-09-03
- method: read-only inspection of this working tree (no jj or git command run, no dependency added, no build started) plus read-only retrieval of public primary sources; every external claim carries its URL and the retrieval date 2026-09-03
- inherits: `docs/research/rethink/client-surfaces-and-cross-compile.md` (2026-09-02) for the crate inventory, the cross-compilation matrix and the A1-A4 delivery architectures; `docs/notes/design/client-side-architecture.md` (2026-09-02) for the seam analysis; `docs/research/rethink/leptos-frontend-fit.md` (2026-09-03) for the standing decision that the frontend remains SvelteKit
- decisions held fixed: the marketplace request originates on the seller's device; Windows desktop first, then Android, then iOS, on Tauri v2; the frontend stays SvelteKit

## 1. Executive summary

Crux is a real architecture and we have already built most of it by hand, which is the finding that decides the question: `SyncMachine::step` is Crux's `update`, `tam_domain::Effect` is Crux's `Effect`, and `driver.rs` is the effect interpreter Crux calls the shell.
Crux is not an alternative to Tauri, and treating it as one is the category error to avoid: Crux's own `examples/counter` ships a `tauri` shell beside its `apple`, `Android`, `windows`, `tui` and five web shells, so the real question is Crux-inside-Tauri, not Crux-or-Tauri (<https://api.github.com/repos/redbadger/crux/contents/examples/counter>, retrieved 2026-09-03).
The blocking mismatch is granularity and custody of HTTP: `crux_http` "allows Crux apps to make HTTP requests by asking the Shell to perform them" (<https://docs.rs/crux_http/latest/crux_http/>, 0.20.0, retrieved 2026-09-03), and handing our marketplace envelopes to URLSession, OkHttp or `fetch` discards the live-fire evidence that the reqwest path reproduces captured browser traffic.
There is an escape — the 0.19 effect router lets a "core-local handler resolve a `Request` back through the router" so `reqwest` stays in Rust (<https://docs.rs/crux_core/latest/crux_core/effects/index.html>, retrieved 2026-09-03) — but once HTTP is routed core-locally, Crux is buying us only the ViewModel bridge and the typed bindings.
The price for that is high and recurring: rewriting the most safety-critical 1,581 lines we own, four breaking minors in the eight months to 2026-08-07, and a bet on BoltFFI, the FFI generator Crux switched to in 0.19, which is nine months old with 97 open issues (<https://api.github.com/repos/boltffi/boltffi>, created 2025-12-23, retrieved 2026-09-03).
Against a solo founder with a working SvelteKit console, Crux plus native SwiftUI and Compose shells is the option the prior note already priced and rejected as three UI codebases, and Crux changes the bindings generator underneath that option without changing its arithmetic.
Dioxus has no role at all, and the reason is structural rather than a judgement about quality: `dioxus-desktop` renders "the Dioxus VirtualDom using the platform's native WebView", built on wry and tao, the same substrate Tauri uses, and it offers `with_custom_index` and `with_root_name` but no way to load an external site, so it cannot host our SvelteKit build as the application (<https://docs.rs/dioxus-desktop/latest/dioxus_desktop/> and its `Config` page, 0.7.10, retrieved 2026-09-03).
Blitz, the renderer that would make Dioxus interesting, "is currently in a **beta** state" with "still many bugs and missing features", explicitly excludes websockets and localstorage, and struggles with "pages that require JavaScript to function properly" — which is exactly what a marketplace login page is (<https://raw.githubusercontent.com/DioxusLabs/blitz/main/README.md> and <https://dioxuslabs.com/blog/release-070>, retrieved 2026-09-03).
Recommendation: adopt neither framework, adopt Crux's shell-contract discipline as five to eight engineer-days of naming and hygiene inside the work already scheduled, and defer the typed-bindings question to the phone milestone where UniFFI 0.32 is the mature answer.

## 2. Crux, precisely

Crux is Red Badger's core-and-shell framework: it "splits the application into two distinct parts, a Core built in Rust, which drives as much of the business logic as possible, and a Shell, built in the platform native language", where the Core is "side–effect free" and the Shell "provides all interfaces with the external world" (<https://redbadger.github.io/crux/>, retrieved 2026-09-03).
The division of labour is stated precisely, and the parenthesis is the part worth keeping: "The Core handles the behaviour logic, the Shell handles the presentation layer and effect execution (but not *orchestration*, that is part of the behaviour and therefore in the Core)."

Version and cadence.
`crux_core` 0.20.0 was published 2026-08-07, with 343,228 total downloads and 43,535 recent (<https://crates.io/api/v1/crates/crux_core>, retrieved 2026-09-03).
The breaking-minor cadence over the last year, from the same source and the release notes: 0.16.0 on 2025-07-31, 0.16.1 on 2025-09-01, 0.16.2 on 2025-12-15, 0.17.0 on 2026-03-20 after three release candidates, 0.18.0 on 2026-05-08, 0.19.0 on 2026-06-08, 0.20.0 on 2026-08-07 (<https://api.github.com/repos/redbadger/crux/releases>, retrieved 2026-09-03).
Every one of 0.17 through 0.20 carries a breaking change in its own notes, so that is four breaking minors in eight months.
The repository is healthy and small: 2,714 stars, 114 forks, 20 open issues, 42 watchers, created 2022-10-08, last pushed 2026-08-26, not archived (<https://api.github.com/repos/redbadger/crux>, retrieved 2026-09-03).
Twenty open issues on a framework this size reads as attentive maintenance rather than neglect.

Maturity signals, in the project's own words.
The book says "Crux is used in production apps today, and we consider it production ready", immediately qualified by "we still have a number of things to work on to call it 1.0, with a stable API and excellent DX expected from a mature framework" and "before 1.0, some breaking changes will be unavoidable" (<https://redbadger.github.io/crux/>, retrieved 2026-09-03).
The README repeats it: "Crux is pre-1.0 and under active development. It is production-ready, but occasional breaking changes to the API can be expected."
Named users are referenced rather than listed: PhotoRoom through an engineering blog series on "Building live collaboration in Rust for millions of users", and Proton through a talk, "Scaling Large Organisations: Empowering Independent Teams with Crux Micro-Frontends", linked to a post about next-generation Proton Mail mobile apps (<https://raw.githubusercontent.com/redbadger/crux/master/README.md>, retrieved 2026-09-03).
Two named companies of that calibre is a materially stronger reference set than the Leptos note could find for Leptos, which had none.
Sponsors are Red Badger Consulting Limited and Zulip.

How the core is driven.
The app implements four associated types — `Event`, `Model`, `ViewModel`, `Effect` — and two functions, `fn update(&self, event: Event, model: &mut Model) -> Command<Effect, Event>` and `fn view(&self, model: &Model) -> ViewModel` (<https://redbadger.github.io/crux/print.html>, retrieved 2026-09-03).
The vocabulary is three-layered: an "Effect" is "a request for a type of side-effect (e.g. a HTTP request)", an "Operation" is "carried by the Effect, specifies the data for the effect", and a "Command" is "a bundle of effect requests which execute together, sequentially, in parallel".
The legacy Capability API is gone: 0.17.0 "removes legacy Capability API in favor of the Command API exclusively" (release notes, 2026-03-20).
Published capabilities are Render (built into `crux_core`), Http (`crux_http`), KeyValue (`crux_kv`) and Time (`crux_time`).

What a shell must implement.
Native equivalents of "the `update`, `view` and `resolve` functions", over an FFI where "Crux exposes FFI calls taking and returning the values serialized with `bincode` (by default)" (book, retrieved 2026-09-03).
Effect execution is the shell's job for HTTP, key-value storage and time; `crux_http` states it "allows Crux apps to make HTTP requests by asking the Shell to perform them", and adds "This is still work in progress and large parts of HTTP are not yet supported" (<https://docs.rs/crux_http/latest/crux_http/>, 0.20.0, retrieved 2026-09-03).

Since 0.19 there is a second path, and it matters more to us than anything else in this section.
The `effects` module routes each emitted effect down one of three lanes: Serialized, where "effects are serialized to bytes, sent to the shell, and resolved by id with serialized responses", described as "the default lane and the primary onboarding path"; Parked, for "opaque pointer-style handles" over "a custom, user-owned FFI", tracked by a `ParkedEffectId`; and Buffer, which "collects requests for the caller to drain and handle synchronously".
Beyond the lanes, "effects can also be resolved entirely within the core itself — by a Rust handler, potentially doing async or background work — using the `ResolveSink` trait" (<https://docs.rs/crux_core/latest/crux_core/effects/index.html>, retrieved 2026-09-03).
The middleware module says the same thing from the other side: its purpose is "processing effects requested by the app inside the core, but outside the app itself (which is side-effect free and synchronous)", and "apps using middleware must be `Send` and `Sync`, because the effect middlewares are expected to process effects asynchronously" — "background thread; on WASM it means an async task (e.g. `spawn_local`)" (<https://docs.rs/crux_core/latest/crux_core/middleware/index.html>, retrieved 2026-09-03).

How it ships to platforms.
"On iOS/macOS as a native static library packaged with BoltFFI Swift bindings", "On Android as a dynamic library packaged with BoltFFI Kotlin bindings and native assets", and "In a browser as a WebAssembly module" (README, retrieved 2026-09-03).
The bindings substrate changed recently and this is the single largest maturity caveat in the Crux stack: 0.19.0 (2026-06-08) is a "breaking switch from UniFFI to BoltFFI for FFI bindings", which also "removes `cli` feature/`crux_cli` and rustdoc-based typegen, deprecates UniFFI compat bindgen" (release notes).
The book now states "Crux uses BoltFFI for the small byte-oriented FFI surface" and "Facet-generated Swift, Kotlin, TypeScript, and C# types handle the app's serialized `Event`/`Effect`/`ViewModel` data."
BoltFFI itself was created 2025-12-23 and has 872 stars, 51 forks and 97 open issues, last pushed 2026-09-02 (<https://api.github.com/repos/boltffi/boltffi>, retrieved 2026-09-03); it claims up to 1,542x faster than UniFFI on Apple Silicon for 10,000 `i32` values via "a zero-copy approach where possible" (<https://www.boltffi.dev/>, retrieved 2026-09-03).
For comparison, UniFFI is at 0.32.0 published 2026-06-30 with 11,641,858 total downloads (<https://crates.io/api/v1/crates/uniffi>, retrieved 2026-09-03).
Desktop is not a first-class Crux concept — it is whatever shell you write — and the reference shells in `examples/counter` are `Android`, `apple`, `tauri`, `tui`, `web-dioxus`, `web-leptos`, `web-nextjs`, `web-react-router`, `web-yew` and `windows`.
There is no Svelte shell in that list; we would write the first one, though the `tauri` shell is a Vite plus TypeScript project so the delta is small.

## 3. Crux against our core

Our tree already implements the pattern, and the correspondence is close enough to be checked line by line.
`SyncMachine::step(self, input: Input, now: LogicalInstant) -> Result<Transition, MachineError>` at `crates/tam-domain/src/lib.rs:815` is Crux's `update`, with two differences that favour ours: the input is explicit rather than an `&mut Model`, and time enters as data rather than through a Time capability.
`tam_domain::Effect` at `crates/tam-domain/src/lib.rs:613` is Crux's `Effect` enum.
`crates/tam-engine/src/driver.rs` (1,581 lines) is the effect interpreter, described in its own module doc as pumping "one leased item through the machine, executing each effect in order and feeding the result back as the next input" — which is Crux's core-shell loop written out by hand.
`crates/tam-marketplace/src/transport.rs:293` is a second, lower seam that Crux has no equivalent for.

That second seam is where adoption breaks, because our two seams sit at different altitudes than Crux's one.
Our `Effect` variants are domain-level: `Submit { attempt, key, fields }`, `Revise`, `Remove`, `ReadBack`, `Reconcile`, `ParkItem`, `RequeueBehindGate`, `CaptureDiagnostics`, `Halt`.
Crux's effects are capability-level: Http, KeyValue, Time, Render.
One `Submit` is not one HTTP request; it is a whole adapter flow — form scrape, S3 signing-oracle upload, create, publish — spanning many requests inside 12,855 lines of `tam-marketplace-tpt` and `tam-marketplace-tes`.

Three mappings are available and only one survives.

Making our `Effect` the Crux effect boundary means the shell implements `Submit` and `Revise`, so the adapters move into Swift, Kotlin and TypeScript.
That inverts the entire point of the exercise and is dismissed immediately.

Making `Transport` the Crux effect boundary means adopting `crux_http`, and it fails on evidence rather than taste.
Our value is that the captured browser envelopes replay faithfully; the M7 note records the TPT write path as live-fired and survived a 35-agent review, and `docs/notes/design/client-side-architecture.md` §4 states the native process runs the adapters "envelope byte-identical to what was captured".
Routing those requests through the shell's HTTP stack — URLSession on iOS, OkHttp on Android, `fetch` on web — changes header ordering, cookie jar semantics, redirect policy and TLS fingerprint, and invalidates both the live-fire evidence and the cassette fixtures in one move.
Two secondary objections stand on their own: `crux_http` says "large parts of HTTP are not yet supported", and the Serialized lane bincodes bodies across the bridge, which for `UPLOAD_BODY_BYTES_MAX` at 256 MiB (`crates/tam-limits/src/lib.rs:105`) is a serialization of the whole upload body on every crossing.

Routing HTTP core-locally is the only viable shape.
The 0.19 effect router and the `ResolveSink` trait allow exactly this, so `reqwest` stays in Rust and only Render crosses the FFI.
The `Send + Sync` requirement the middleware module imposes is already satisfied: `pub trait Transport: Send + Sync` with the future bounded `+ Send` at `transport.rs:293`.
Our async survives, relocated from `driver.rs` into router handlers, which is a move rather than a loss.

So, concretely, what adopting Crux changes.
Our adapters do not become Crux capabilities; they become core-local route handlers behind an effect the router never serializes.
`driver.rs` becomes two pieces: the pure part folds into `update` returning `Command<Effect, Event>`, and the impure part — ledger writes through `append_event` and the storage repositories, lease epochs, budget grants — becomes either further routed effects or middleware, which is a redistribution of 1,581 lines of the most correctness-sensitive code we own.
`HttpRequest` and `HttpResponse` already derive `Serialize` and `Deserialize`, and `Effect` and `Input` are serde types, so the data-shape prerequisites are met today.

What it buys.
Typed Swift, Kotlin and TypeScript bindings generated from our Rust types, which is genuine and which we have not yet priced: in a client-side world the SvelteKit console stops talking to `tam-api` over HTTP and starts talking to a local core, and someone has to own that interface's types.
A shell contract with nine reference implementations, including a Tauri one.
A WASM web shell that would serve the browser-extension path in `client-surfaces-and-cross-compile.md` §9 A3 and A4.

What it costs.
The driver rewrite above, against a live-fire suite that must be re-run to restore the evidence it currently carries.
Four breaking minors in eight months, absorbed by a solo founder, on a framework whose own book promises more before 1.0.
A dependency on BoltFFI, nine months old, adopted by Crux three months ago, replacing UniFFI which has eleven million downloads.
Bincode on every Event and ViewModel crossing, which is cheap for a ViewModel and irrelevant for effects we route core-locally.

## 4. The decisive comparison, and the hybrid

Crux with native SwiftUI and Compose shells versus Tauri v2 with one webview is not a close call for this founder, and the reason is codebase count rather than technology.

Tauri v2 gives one UI codebase — the existing SvelteKit console, redesigned to an approved mockup on 2026-09-01 — rendered in five shells, with the Rust core compiled in and first-party `@tauri-apps/api` bindings.
The prior note priced that path (A2) at 18 to 28 weeks including three desktop OSes, iOS and Android, and Tauri is at 2.11.5 published 2026-07-01 with 28,557,437 total downloads (<https://crates.io/api/v1/crates/tauri>, retrieved 2026-09-03).

Crux with native shells gives three UI codebases — SwiftUI, Jetpack Compose, and a web shell for the console — plus the core rewrite, plus the bindings layer.
`client-surfaces-and-cross-compile.md` §4.5 already assessed that shape under "KMP or native shells + UniFFI" and concluded it is "the best native user experience and the highest ongoing cost", recommending against it because our mobile surface is "a control surface over a server-mediated workflow" rather than the product.
Crux swaps UniFFI for BoltFFI underneath that conclusion and leaves it standing.
The recurring cost is the one that decides it: every new field, every new marketplace, every new status state has to be rendered three times forever, by one person.

The hybrid the founder asks about is not only available, it is nearly free, and it is what I recommend.
Tauri desktop now, with UniFFI-style bindings for phones later, costs almost nothing to keep open because the enabling conditions are already true or already scheduled.
The pure core is free of tokio, `libc` and platform I/O (§2 of the prior note), the effect stream and the transport types are serde-serializable today, and the prior note's step 1 — the `live` feature gate and the `MaybeSend` shim — is already on the plan.
The insight that makes it cheap is that a phone shell does not need the effect stream at all: it needs a coarse API — start a sync, poll status, list items, present a login webview — which is four or five functions across a UniFFI boundary, not a bridged interpreter.
Crux's value proposition is fine-grained effect bridging, and fine-grained is precisely what a phone shell over our workflow does not want.

## 5. Dioxus, precisely

Dioxus is a Rust UI framework, currently 0.7.10 published 2026-07-30, with 0.8.0-alpha.1 published 2026-07-31 and no announced stable date; 2,507,726 total downloads and 891,619 recent (<https://crates.io/api/v1/crates/dioxus>, retrieved 2026-09-03).
The 0.7 line shipped 2025-09-08 (<https://dioxuslabs.com/blog/release-070>, retrieved 2026-09-03).
It is a large and active project: 38,944 stars, 1,863 forks, 761 open issues, created 2021-01-15, last pushed 2026-09-01 (<https://api.github.com/repos/DioxusLabs/dioxus>, retrieved 2026-09-03).

Renderers, with the project's own status language.
Web compiles to WASM.
Desktop is `dioxus-desktop`, whose crate documentation says it will "Render the Dioxus VirtualDom using the platform's native WebView implementation" and states "Dioxus Desktop is built off Tauri... you'll want to leverage Tauri - mostly Wry and Tao directly", depending on wry ^0.53.5 and tao ^0.34.0 (<https://docs.rs/dioxus-desktop/latest/dioxus_desktop/>, 0.7.10, retrieved 2026-09-03).
Mobile is the same substrate: "Mobile apps are rendered with either the platform's WebView or experimentally with WGPU", and "native Android animations and widgets aren't currently supported, CSS-based animations and styling provide a powerful alternative"; the same page calls mobile "a first-class target for Dioxus apps, with a robust WebView implementation" (<https://dioxuslabs.com/learn/0.7/guides/platforms/mobile/>, retrieved 2026-09-03).
Fullstack and server functions exist, and static site generation is documented (<https://dioxuslabs.com/learn/0.7/>, retrieved 2026-09-03).
LiveView and TUI are not named as renderers in either the 0.7 release post or the 0.7 learn index; the 0.7 post mentions Ratatui only as a third-party demonstration of hot-patching, so their current status is UNVERIFIED here.

Blitz, the native path, is the interesting one and it is not ready.
Its README says "Blitz is currently in a **beta** state", "It can already render many popular no-JS websites (Wikipedia, (Old) Reddit, etc)", it "is usable for making apps if you are an early adopter and willing to live on the bleeding edge", "there are also still many bugs and missing features", and "We are actively working on bringing it up to production quality" (<https://raw.githubusercontent.com/DioxusLabs/blitz/main/README.md>, retrieved 2026-09-03).
Its scope is deliberately narrower than a browser: "Notably we *don't* provide features like webrtc, websockets, bluetooth, localstorage, etc."
The repository has 4,095 stars, 189 forks and 176 open issues, last pushed 2026-09-02 (<https://api.github.com/repos/DioxusLabs/blitz>, retrieved 2026-09-03).

Temporal note, flagged rather than silently reconciled.
`client-surfaces-and-cross-compile.md` §4.2 (2026-09-02) quotes the 0.7 release post of 2025-09-08 saying "Blitz is still very young", "still considered a 'work in progress'" and "We have not focused on performance".
The Blitz README as retrieved today says "beta" and "actively working on bringing it up to production quality", which is a year newer and a step up.
The newer source wins on the label; both agree it is pre-production, so §4.2's conclusion is unchanged and only its adjective is stale.

Hot reload is Dioxus's strongest differentiator: Subsecond hot-patching "works across all major platforms: Web (WASM), Desktop (macOS, Linux, Windows), and even mobile (iOS, Android)", with the caveat that "Subsecond does not automatically migrate your state for you" when struct fields change (0.7 post).
Mobile issue volume is the counterweight: 44 open issues mention Android and 22 mention iOS, including "Meta-issue: Android and IOS build" opened 2026-08-28 and "Android: button onclick handlers don't fire (desktop works, oninput works)" opened 2026-08-26 (<https://api.github.com/search/issues>, retrieved 2026-09-03).
An open meta-issue on the mobile build one week old is a different maturity profile from Tauri's dozen packaging papercuts recorded in the prior note.

Against Tauri v2 as a shell, Dioxus adds nothing for us and removes several things.
Both use wry and tao for desktop and mobile, so the webview substrate is identical by construction.
Tauri brings a first-party plugin workspace our A2 plan depends on — updater, store, http, deep-link, notification, dialog, clipboard — and `create-tauri-app` templates; Dioxus brings `dx serve` and hot-patching, which help a Rust UI author and do not help a Vite user who already has module-level hot replacement.

Can Dioxus host an existing SvelteKit build?
No.
`Config::with_custom_index` will "Use a custom index.html instead of the default Dioxus one" but warns "Dioxus injects some loader code into the closing body tag"; `with_root_name` sets "the name of the element that Dioxus will use as the root", explicitly "akin to calling React.render() on the element with the specified name"; `with_custom_protocol` and `with_asynchronous_custom_protocol` serve assets; `with_navigation_handler` gates "non-dioxus URLs" (<https://docs.rs/dioxus-desktop/latest/dioxus_desktop/struct.Config.html>, 0.7.10, retrieved 2026-09-03).
There is no `with_url`.
The VirtualDom is the application, and the index.html is a container for it, so a SvelteKit SPA cannot be the app under Dioxus.
Serving our static build through a custom protocol handler in a Dioxus window with an empty VirtualDom would technically render it, but at that point we have reimplemented wry badly and thrown away Tauri's plugin surface; that is a curiosity, not an option.

## 6. Dioxus against our decisions

There is no role for Dioxus, and no niche worth reserving.

The frontend stays SvelteKit and the shell is Tauri, so Dioxus would be a second UI framework maintained by one person alongside the first, for a substrate identical to the one Tauri already provides.
The tray or menubar niche does not survive contact either: Tauri ships tray and menu APIs first-party, and a native-rendered tray does not need a UI framework.
The Blitz path is the only argument with any force, and it fails on our specific requirement rather than on Blitz's quality: our design turns on hosting a marketplace login in a real browser engine so a session cookie can be captured, and Blitz's own scope excludes localstorage and websockets while the 0.7 post records that it struggles with "pages that require JavaScript to function properly".
A renderer that cannot run a marketplace's login page is disqualified from the one job we would need a renderer for.

For Dioxus to displace Tauri plus Svelte, three things would all have to become true.
We would have to decide the SvelteKit console is disposable, which is a strictly larger decision than the framework choice and which the Leptos note already answered no on stronger grounds.
Blitz would have to reach production quality while we simultaneously retained a system webview for marketplace login, since Blitz cannot serve that role — so the native-rendering win would be partial by construction.
And Dioxus mobile packaging would have to close the open Android and iOS build meta-issue and reach parity with Tauri's plugin surface for updater, store and deep-link.
None of those is near, and the first is a product decision rather than a technical one.

## 7. The one thing to take from each

From Crux, take the shell contract as a discipline, without the framework.
Crux's real contribution is insisting that the boundary be named, narrow, serializable and testable: a core exposing `update`, `view` and `resolve`, effects that are data, and a shell that executes them and nothing else.
We have that shape and have never written the contract down, which means it is currently defended by convention rather than by a type.
Naming it — a `ClientCore` façade with coarse operations, the effect stream serde-serializable end to end, and an in-process Buffer-style executor for tests — is the whole benefit, and it is what makes a UniFFI or BoltFFI binding layer a later addition rather than a later rewrite.
Take the typed-bindings idea too, and defer the tool choice: when phones arrive, UniFFI 0.32.0 (2026-06-30, 11.6 million downloads) is the conservative answer and BoltFFI is the fast one that will be about eighteen months old by then.

From Dioxus, take nothing.
The near-miss worth naming so it is not rediscovered: Dioxus 0.7's unified `dioxus.toml` for mobile — setting permissions, entitlements, `AndroidManifest.xml` and `Info.plist` fields from one config file — is a genuinely good idea, and Tauri already does it in `tauri.conf.json`, so there is nothing to port.

## 8. Verdict, recommendation and cost

Adopt neither Crux nor Dioxus.
Adopt three Crux disciplines inside work that is already scheduled, and revisit Crux only if a native phone UI becomes a product requirement.

The recommended work, in order.
Write the shell contract down as a `ClientCore` façade over the driver and adapters, with coarse operations and no leaked storage types: three to five engineer-days, and it is a prerequisite for the `driver.rs` split the prior note already scheduled as step 2 rather than an addition to it.
Formalise the in-process effect executor so the effect stream can be driven and asserted without a database, which is most of the way there already through the cassette harness: two to three engineer-days.
Defer bindings entirely, and record the decision so it is not relitigated: zero days now, a three-to-five-day spike at the iOS milestone.

Incremental cost of the recommendation: five to eight engineer-days, on top of the prior note's steps 1 and 2, with no change to the A2 sequence or its 18-to-28-week envelope.
Assumptions: one senior engineer, full-time, already fluent in this tree; the `live` feature gate and `MaybeSend` shim land first as step 1 already requires; confidence roughly ±40 per cent, because the façade's surface is a design question rather than a mechanical one.

For contrast, the cost of the path not taken.
Adopting Crux properly means redistributing `driver.rs` into `update` plus router handlers plus middleware, re-running the live-fire evidence on both platforms to restore what the rewrite invalidates, and writing the first Svelte shell over the bincode bridge: six to ten weeks on the same assumptions, buying nothing on the desktop-first path and deferring the milestone that proves the client-side thesis.
That is the trade the founder would be making, and it is not close.

## 9. Open questions for the founder

1. Will the phone surfaces ever be natively rendered, or is a webview acceptable indefinitely? Recommended answer: webview indefinitely, revisit only on user evidence; a native answer reopens the Crux question and nothing else does.
2. Does the local core expose a loopback HTTP surface to the console, or a Tauri IPC command surface? Recommended answer: loopback HTTP, because the console speaks it today and it keeps the web console and the desktop client on one client codebase; this is the interface Crux would otherwise force us to design its way.
3. Do we accept a nine-month-old FFI generator anywhere in the stack, now or later? Recommended answer: no; prefer UniFFI 0.32 when bindings are actually needed, and re-evaluate BoltFFI at the iOS milestone.
4. Should the shell-contract façade be its own crate, or a module in `tam-engine`? Recommended answer: its own crate, so that the dependency direction is enforced by cargo rather than by discipline.

## 10. Unverified

- Whether a Crux Parked or Buffer lane can carry a 256 MiB upload body without a full copy; BoltFFI's zero-copy claim covers primitives and structs across its own FFI, not bincode-serialized bodies through the Serialized lane.
- Whether facet and BoltFFI TypeScript typegen produces types consumable directly by Svelte 5 and our TanStack Query layer without a hand-written wrapper; no Svelte shell exists in the Crux examples to check against.
- The Tauri major version used by Crux's `examples/counter/tauri` shell; the directory listing shows `src-tauri`, `package.json` and a Vite config, but `src-tauri/Cargo.toml` was not read.
- Whether PhotoRoom and Proton are using Crux in production today and at what scale; the README references a blog series and a talk, which are historical artefacts rather than a current-state attestation.
- The current status of Crux's `tui` example shell and of Dioxus's LiveView and TUI renderers; neither project's current documentation names them as supported targets, and absence from a page is not a deprecation.
- Dioxus 0.8's stable timeline; only 0.8.0-alpha.0 (2026-05-19) and 0.8.0-alpha.1 (2026-07-31) are published and no date was found.
- Whether Blitz can render a TPT or Tes login page at all; the 0.7 post's statement about JavaScript-dependent pages is the closest evidence and no direct test was run.

## 11. Sources

All retrieved 2026-09-03.

- <https://redbadger.github.io/crux/> and <https://redbadger.github.io/crux/print.html> — the Crux book
- <https://raw.githubusercontent.com/redbadger/crux/master/README.md> — production references, sponsors, platform linking
- <https://api.github.com/repos/redbadger/crux>, <https://api.github.com/repos/redbadger/crux/releases>, <https://api.github.com/repos/redbadger/crux/contents/examples/counter>
- <https://crates.io/api/v1/crates/crux_core>
- <https://docs.rs/crux_core/latest/crux_core/effects/index.html>, <https://docs.rs/crux_core/latest/crux_core/middleware/index.html>, <https://docs.rs/crux_http/latest/crux_http/>
- <https://www.boltffi.dev/>, <https://api.github.com/repos/boltffi/boltffi>, <https://crates.io/api/v1/crates/uniffi>
- <https://dioxuslabs.com/blog/release-070>, <https://dioxuslabs.com/learn/0.7/>, <https://dioxuslabs.com/learn/0.7/guides/platforms/mobile/>
- <https://docs.rs/dioxus-desktop/latest/dioxus_desktop/>, <https://docs.rs/dioxus-desktop/latest/dioxus_desktop/struct.Config.html>
- <https://crates.io/api/v1/crates/dioxus>, <https://api.github.com/repos/DioxusLabs/dioxus>, <https://api.github.com/repos/DioxusLabs/blitz>, <https://raw.githubusercontent.com/DioxusLabs/blitz/main/README.md>
- <https://crates.io/api/v1/crates/tauri>
- Local, read-only: `crates/tam-domain/src/lib.rs:613` and `:815`, `crates/tam-engine/src/driver.rs`, `crates/tam-marketplace/src/transport.rs:293`, `crates/tam-limits/src/lib.rs:105`, `docs/research/rethink/client-surfaces-and-cross-compile.md`, `docs/notes/design/client-side-architecture.md`, `docs/research/rethink/leptos-frontend-fit.md`
