# Client surfaces and cross-compilation

What it costs, per delivery surface, to ship the Rust core to web, desktop, iOS, Android and browser extension; what each surface can actually do against a marketplace with no API; and what sequencing produces a working product fastest.

- date: 2026-09-02
- method: read-only inspection of this working tree (no jj or git commands run) plus read-only retrieval of public primary sources; every external claim carries the URL and the retrieval date 2026-09-02
- inherits: `docs/notes/design/client-side-architecture.md` (2026-08-31) and `docs/notes/legal/marketplace-terms-assessment.md`; this note does not re-derive their findings and says where it departs from them
- status: research input to a founder decision, nothing built, no marketplace contacted

## 1. Executive summary

The four surfaces the founder named are not equivalent, and the inequality is structural rather than a matter of effort: a web page cannot originate a marketplace request at all, because a cross-origin `fetch` to TPT or Tes carries no session cookie and is refused by CORS, so the web surface can only ever be a console unless a browser extension supplies its data plane.
Desktop, iOS and Android can each originate the request, and the code that does so is already portable: the pure core is 16,169 lines across six crates whose entire dependency closure is 48 crates, every one of them pure Rust, with no `libc`, no `ring`, no `*-sys` and no `getrandom` — verified by `cargo tree` and by a clean `cargo check` of that set alone.
The transport-coupled production code is 873 lines out of 28,151 that would move to a client, a ratio of about 32:1, and the one prerequisite is a Cargo feature gate that does not exist yet, because `reqwest` is currently an unconditional dependency of both adapters.
Every target the founder needs is Rust Tier 2 or better, and the only non-pure crate in the adapter path is `ring`, whose CI tests `aarch64-apple-ios`, `aarch64-linux-android` and `armv7-linux-androideabi`, so the literal cross-compilation is close to free.
The cost is not compilation; it is the mobile platform itself: iOS grants a background refresh task "up to 30 seconds" at a time the system chooses, and Android defers `JobScheduler` and therefore WorkManager entirely in Doze, so a phone cannot be the deterministic scheduler and "same functionality as web" is achievable for user-initiated work and false for unattended sync.
Session capture works on both mobile platforms — WebKit's `getAllCookies` applies no HttpOnly filter and Chromium's Android WebView cookie manager reads with `MakeAllInclusive()`, so the marketplace session cookie is reachable from the app after a webview login — but Tauri's own cookie API is documented as unsupported on Android, so Android needs a small JNI plugin we write.
Store policy is the sharpest new risk: Apple guideline 5.2.2 requires that you be "specifically permitted" under a third-party service's terms and that "Authorization must be provided upon request", which hands Apple the same lever a cease-and-desist gives the marketplace, and Vendoo and Crosslist both ship automating apps on the App Store today, which is evidence of tolerance rather than of permission.
Tauri v2 is the only framework that reuses both the Rust core and the existing SvelteKit console on all five surfaces, and the console is already `adapter-static` with an `index.html` fallback, which is exactly the shape a Tauri or Capacitor shell consumes.
The recommendation is to sequence by data plane rather than by surface: land the transport feature gate and the engine-to-remote-ledger split first, ship the Tauri desktop client, then the browser extension as the web surface's data plane, then iOS, then Android, and treat mobile "same functionality" as user-initiated parity with an explicitly-stated scheduling difference.
First shippable milestone: the seam work plus a Tauri desktop client on Windows and macOS, roughly 10 to 16 weeks for one senior engineer, after which every later surface is packaging rather than architecture.

## 2. What is actually in the tree, and what that implies

Three layers exist today, and the boundary between them is what makes the portability question answerable rather than speculative.

The pure core is six crates and 16,169 lines: `tam-types` (1,544), `tam-limits` (278), `tam-marketplace` (1,545), `tam-domain` (5,887), `tam-taxonomy` (5,772) and `tam-pipeline` (1,143).
`cargo check -p tam-types -p tam-limits -p tam-marketplace -p tam-domain -p tam-taxonomy -p tam-pipeline` succeeds on its own, and `cargo tree -e normal` over that set yields exactly 48 non-workspace crates: `adler2`, `arrayvec`, `bitflags`, `blake3`, `bumpalo`, `bytemuck`, `byteorder-lite`, `cfg-if`, `color_quant`, `constant_time_eq`, `cpufeatures`, `crc32fast`, `displaydoc`, `equivalent`, `fdeflate`, `flate2`, `gif`, `hashbrown`, `image`, `indexmap`, `itoa`, `log`, `memchr`, `miniz_oxide`, `moxcms`, `num-traits`, `png`, `proc-macro2`, `pxfm`, `quote`, `serde`, `serde_core`, `serde_derive`, `serde_json`, `sha1_smol`, `simd-adler32`, `syn`, `thiserror`, `thiserror-impl`, `unicode-ident`, `uuid`, `weezl`, `zip`, `zmij`, `zopfli`, `zune-core`, `zune-jpeg`.
There is no `libc`, no `ring`, no crate ending in `-sys`, no `getrandom` and no `cc` build dependency in that closure, and `uuid` is pulled with the `v5` feature only, which is SHA-1 based via `sha1_smol` and therefore needs no randomness source.
`flate2` resolves to the `miniz_oxide` backend rather than to a bundled zlib, so `zip`'s deflate path is pure Rust too.
The consequence is precise and unusually strong: this layer compiles to every Rust target including `wasm32-unknown-unknown` with no feature flags, no `js` shim, and no C toolchain.

The adapters are 12,855 lines across `tam-marketplace-tpt` (8,666) and `tam-marketplace-tes` (4,189), of which the transport-coupled production code is 873 lines: `crates/tam-marketplace-tpt/src/live.rs` lines 1 to 573 and `crates/tam-marketplace-tes/src/live.rs` lines 1 to 300, with `#[cfg(test)]` beginning at line 574 and line 301 respectively.
The remaining 11,982 lines — `endpoints.rs`, `flows.rs`, `form.rs`, `classify.rs`, `write_model.rs`, `read_model.rs`, `s3.rs`, `upload.rs`, `identity.rs`, `session.rs`, `schema.rs`, `guard.rs` — are generic over the seam and mention `reqwest` exactly once, in a comment at `crates/tam-marketplace-tpt/src/flows.rs:774`.
One blocking detail the prior note did not surface: `reqwest` is declared as an unconditional dependency in both adapter manifests with no `[features]` table at all, so `live.rs` compiles on every target today and a non-native build fails at the dependency, not at the code.
Adding a `live` feature that gates `live.rs` and moves `reqwest` to `optional = true` is a small, safe, immediately useful change and it is step zero for every path in this note.

The transport seam itself is `pub trait Transport: Send + Sync` at `crates/tam-marketplace/src/transport.rs:293`, with the future bounded `+ Send`, and `HttpRequest`/`HttpResponse` already deriving `Serialize` and `Deserialize`.
The `Send + Sync` bounds are the second portability obstacle, because a browser or wasm transport is single-threaded and its futures are not `Send`; the prior note's `MaybeSend` cfg shim (roughly 20 lines) remains the right fix and is harmless on native.

The server-bound layer is unchanged in kind: `tam-storage` (10,031), `tam-api` (8,205), `tam-session-broker` (4,326) and `tam-engine` (2,634, of which `driver.rs` is 1,581).
The real engineering is still splitting `driver.rs` from concrete `tam-storage` repositories behind a repository trait so the effect interpreter can run against a remote ledger; every client surface needs that same split, which is why it sequences first regardless of which surface ships first.

The web console is `web/`, SvelteKit 2 with `@sveltejs/adapter-static` and `fallback: 'index.html'`, Svelte 5, Tailwind 4 and TanStack Query.
That matters more than it looks: an already-static SPA with a client-router fallback is precisely what a Tauri shell, a Capacitor shell or an extension options page consumes without modification, so the UI reuse question for those paths is answered by the existing build rather than by a port.

One product constant worth carrying into the mobile analysis: `crates/tam-limits/src/lib.rs:105` sets `UPLOAD_BODY_BYTES_MAX` to 256 MiB.

## 3. The cross-compilation matrix

### 3.1 Target tiers

Every target the founder's plan needs is Tier 2 or better, per <https://doc.rust-lang.org/rustc/platform-support.html> (retrieved 2026-09-02).
Tier 1 is "guaranteed to work" with automated testing after every change; Tier 2 is "guaranteed to build", with official standard-library releases and automated builds, but "Automated tests are not always run so it's not guaranteed to produce a working build".

| Target | Tier | Relevance |
|---|---|---|
| `x86_64-unknown-linux-gnu` | 1, host tools | server, CI, Linux desktop |
| `aarch64-unknown-linux-gnu` | 1, host tools | Linux desktop, ARM servers |
| `aarch64-apple-darwin` | 1, host tools | macOS desktop |
| `x86_64-apple-darwin` | 2, host tools | macOS desktop, Intel |
| `x86_64-pc-windows-msvc` | 1, host tools | Windows desktop |
| `aarch64-pc-windows-msvc` | 1, host tools | Windows on ARM |
| `aarch64-apple-ios` | 2, no host tools | iPhone/iPad device |
| `aarch64-apple-ios-sim` | 2, no host tools | iOS simulator on Apple Silicon |
| `x86_64-apple-ios` | 2, no host tools | iOS simulator on Intel |
| `aarch64-linux-android` | 2, no host tools | modern Android device |
| `armv7-linux-androideabi` | 2, no host tools | older 32-bit Android |
| `x86_64-linux-android` | 2, no host tools | Android emulator |
| `i686-linux-android` | 2, no host tools | legacy emulator |
| `wasm32-unknown-unknown` | 2, no host tools | web and extension |

Empirical limitation to record: this machine has no `rustup` (the toolchain comes from `rust-overlay` via the flake) and `rustc 1.97.1`'s `rustlib` directory contains only `x86_64-unknown-linux-gnu`.
No cross-target `cargo check` was attempted, because installing a target was out of scope.
Every portability claim below is therefore evidence from upstream documentation and upstream CI rather than from a local build, and the first action in any implementation is to add the targets to `rust-toolchain.toml` and prove them.

### 3.2 Per-dependency portability

The pure core needs no analysis: 48 pure-Rust crates, no conditional compilation, no C.
`blake3` ships optimised SSE2, SSE4.1, AVX2, AVX-512, NEON and WASM implementations with runtime CPU feature detection on x86 (<https://github.com/BLAKE3-team/BLAKE3>, retrieved 2026-09-02); in our tree it appears with `cpufeatures` and no `cc`, so it is taking the intrinsics path rather than a C build.
`image 0.25` is configured `default-features = false` with `png`, `jpeg` and `gif`, which resolves to `png`, `zune-jpeg`, `gif`, `weezl`, `moxcms`, `pxfm` — all pure Rust.
`zip 2.4` is configured `default-features = false, features = ["deflate"]`, which resolves to `flate2` over `miniz_oxide` plus `zopfli`, again pure Rust.
Neither `chrono` nor `time` appears anywhere in the closure, which is consistent with the charter's "time enters as data" discipline and removes the usual wasm timezone-database problem before it arises.

The adapter path adds `reqwest 0.12.28` with `default-features = false, features = ["rustls-tls", "json", "multipart"]`, and through it `hyper 1.11`, `tokio 1.53`, `rustls 0.23.43`, `ring 0.17.14`, `webpki-roots 1.0.9`, `libc 0.2.189`, `mio`, `socket2` and `cc 1.4.4` as a build dependency of `ring`.

`ring` is the only compiled-from-C component and it is fine on mobile.
Its CI matrix (<https://raw.githubusercontent.com/briansmith/ring/main/.github/workflows/ci.yml>, retrieved 2026-09-02) tests `aarch64-apple-ios`, `aarch64-linux-android` and `armv7-linux-androideabi` — the two Android entries with `--no-run`, so they are build-verified rather than test-verified — plus a dedicated `test-wasm32-browser` job on `wasm32-unknown-unknown`.
`x86_64-linux-android` does not appear in that matrix, which matters only for the Android emulator on x86 hosts; `mk/cargo.sh` (retrieved 2026-09-02) does configure `aarch64-linux-android` and `armv7-linux-androideabi` toolchains explicitly.
`BUILDING.md` (retrieved 2026-09-02) states that "*ring* currently requires a C (but not C++) toolchain" and that for cross-compiling "`TARGET_CC` and `TARGET_AR` (or equivalents) must be set", which is exactly what `cargo-ndk` sets for us.

The alternative rustls provider, `aws-lc-rs`, would also work: its platform support matrix (<https://aws.github.io/aws-lc-rs/platform_support.html>, retrieved 2026-09-02) lists `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `aarch64-linux-android` and `x86_64-linux-android` as building, and for non-FIPS builds states that "Bindgen is never required, CMake is never required, and Go is never required", leaving only a C/C++ compiler.
`wasm32-unknown-unknown` does not appear in that matrix at all.
We are on `ring` today, and there is no reason to switch: `ring` covers iOS, Android and wasm, `aws-lc-rs` does not cover wasm, and the FIPS story is irrelevant to us.

`webpki-roots` being in the tree is a quiet advantage for mobile: we carry our own trust anchors rather than reaching for a platform certificate store, so there is no `security-framework` or Android keystore integration to write.

`tokio` with `mio` and `socket2` is native-only and is the reason `wasm32-unknown-unknown` needs a different transport rather than the same one.
`reqwest`'s own documentation (<https://docs.rs/reqwest/latest/reqwest/>, retrieved 2026-09-02, showing version 0.13.4 published 2026-07-28) states that "The Client implementation automatically switches to the WASM one when the target_arch is wasm32", and that "Some of the features are disabled in wasm : tls, cookie, blocking, as well as various ClientBuilder methods such as timeout() and connector_layer()", with the `redirect` module likewise unavailable on that target.
That confirms and sharpens the prior note's redirect finding: `crates/tam-marketplace-tpt/src/live.rs` sets `Policy::none()` and classifies the TPT create by reading the 302 `Location` header, and on wasm there is no redirect policy to set.
Note also that reqwest 0.13 renamed the TLS features (`rustls`, `rustls-no-provider` rather than `rustls-tls`), so any upgrade past 0.12 is a manifest change in both adapters.

`getrandom` appears only under the adapter path, at version 0.2.17.
Its documentation (<https://docs.rs/getrandom/0.2.15/getrandom/>, retrieved 2026-09-02) records `CCRandomGenerateBytes` on iOS and the `getrandom` syscall on Android, both automatic, and states that `wasm32-unknown-unknown` "is not automatically supported" and needs `getrandom = { version = "0.2", features = ["js"] }`, with the caveat that "This feature should only be enabled for binary, test, or benchmark crates."
So the `js` feature is a concern for a wasm binary crate we would write, never for our library crates.

| Dependency root | Linux x64/arm64 | macOS | Windows | iOS + sim | Android arm64/armv7 | Android x86_64 | wasm32-unknown-unknown |
|---|---|---|---|---|---|---|---|
| pure core (48 crates) | yes | yes | yes | yes | yes | yes | yes, no features needed |
| `blake3` | yes | yes | yes | yes | yes | yes | yes, WASM SIMD path |
| `image`, `zip`, `flate2` | yes | yes | yes | yes | yes | yes | yes |
| `serde`, `serde_json`, `uuid` (v5) | yes | yes | yes | yes | yes | yes | yes, no `js` needed |
| `reqwest` (native backend) | yes | yes | yes | yes | yes | yes | no — swaps to fetch backend |
| `tokio`, `mio`, `socket2` | yes | yes | yes | yes | yes | yes | no |
| `rustls` + `ring` | yes | yes | yes | ring CI-tested | ring CI-tested (build only) | not in ring CI | ring browser job exists, but reqwest wasm uses browser TLS |
| `webpki-roots` | yes | yes | yes | yes | yes | yes | n/a |
| `getrandom 0.2` | yes | yes | yes | yes | yes | yes | needs `js` in the binary crate |

The honest reading of that table: our code is not the obstacle on any target.

### 3.3 Tooling

`cargo-ndk` (<https://github.com/bbqsrc/cargo-ndk>, retrieved 2026-09-02) "handles all the environment configuration needed for successfully building libraries or binaries for Android from a Rust codebase, with support for generating the correct `jniLibs` directory structure", supports `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` and `i686-linux-android`, auto-detects the most recent installed NDK, and runs on Linux, macOS and Windows hosts.
Android builds therefore run on cheap Linux CI.

`cross` (<https://github.com/cross-rs/cross>, retrieved 2026-09-02) supports the six Android triples via Docker or Podman images, and lists no Apple targets at all, explaining that MSVC and Apple Darwin images are ones "we cannot ship pre-built images of".

`cargo-zigbuild` (<https://github.com/rust-cross/cargo-zigbuild>, retrieved 2026-09-02) states in its caveats that "Currently only Linux and macOS targets are supported"; it can cross-build `x86_64-apple-darwin` from Linux given `SDKROOT`, and iOS is not mentioned anywhere.

`cargo-mobile2` (<https://github.com/tauri-apps/cargo-mobile2>, retrieved 2026-09-02) generates Xcode and Android Studio projects and is what Tauri consumes as a library; it "is currently supported on macOS, Linux and Windows", with iOS targeting only possible from macOS.

The conclusion is unavoidable and it is the single largest fixed cost in the mobile plan: **iOS builds require a macOS host**, no cross-compilation tool bridges it, and Tauri's own prerequisites page (<https://v2.tauri.app/start/prerequisites/>, retrieved 2026-09-02) says plainly that "iOS development requires Xcode and is only available on macOS".

### 3.4 CI and distribution cost

GitHub-hosted runner rates (<https://docs.github.com/en/billing/concepts/product-billing/github-actions>, retrieved 2026-09-02): Linux 2-core x64 $0.006/min, Linux 2-core arm64 $0.005/min, Windows 2-core $0.010/min, macOS 3- or 4-core (M1 or Intel) $0.062/min.
macOS is roughly six times Windows and ten times Linux per minute, and the rate is the same for Apple Silicon and Intel.
Included minutes are 2,000/month on Free and 3,000/month on Team, and macOS minutes consume that allowance fastest.

A worked estimate, with its assumptions stated: a combined macOS-plus-iOS job of 25 minutes, run 20 times a week, is $1.55 per run, about $31 a week, about $1,600 a year before any caching; halve it with `sccache` and a warm Cargo cache and it is still the dominant CI line item.
Linux Android jobs and wasm jobs at 15 minutes each are cents.

Fees: the Apple Developer Program is "99 USD" a year (<https://developer.apple.com/support/compare-memberships/>, retrieved 2026-09-02) and is required for App Store distribution, TestFlight, notarization and Safari web extension distribution; Google Play charges "a US$25 one-time registration fee" (<https://support.google.com/googleplay/android-developer/answer/6112435>, retrieved 2026-09-02).
A trap for a solo founder: personal Google Play accounts created after 13 November 2023 must run a closed test with "a minimum of 12 testers who have been opted in continuously for at least 14 days" before applying for production access, and "the 14 days must be consecutive" (<https://support.google.com/googleplay/android-developer/answer/14151465>, retrieved 2026-09-02).
That is a two-week wall-clock gate that must start early; the article's scope is personal accounts, so registering as an organisation may avoid it.

Windows code signing could not be priced: the Azure Trusted Signing pricing page (<https://azure.microsoft.com/en-us/pricing/details/trusted-signing/>, retrieved 2026-09-02) renders both tier prices as "$-" and lists only quotas of 5,000 and 100,000 signatures a month.
The prior note's figure of about $120 a year is carried forward as UNVERIFIED.

Tauri's distribution docs (<https://v2.tauri.app/distribute/>, retrieved 2026-09-02) cover App Store (iOS and macOS), Google Play, Microsoft Store, DMG, Windows installer, AppImage, AUR, Debian, RPM, Snapcraft and Flathub, and state that "Both methods requires code signing, and distributing outside the App Store also requires notarization" and that "Signing is required on most platforms."

## 4. Framework options for one Rust core across surfaces

The comparison axis that matters is not "does it work" but "how many codebases does the team maintain, and how much of what exists survives".
Two assets exist: 28,151 lines of Rust that must reach every client, and a working SvelteKit console.

### 4.1 Tauri v2

Tauri 2.0 stable was released 2 October 2024 (<https://v2.tauri.app/blog/tauri-20/>, retrieved 2026-09-02), and the crate on docs.rs is 2.11.5 published 2026-07-01, so it has been stable for nearly two years and is actively released.
The release announcement extends "a single UI code base for desktop operating systems" to iOS and Android, and is candid about the state of it: "We are not completely happy about the developer experience at the moment but are actively improving to bring it up to par with the desktop experience", and "On mobile not all of the official plugins are supported."

The plugin support table (<https://raw.githubusercontent.com/tauri-apps/plugins-workspace/v2/README.md>, retrieved 2026-09-02, legend: ✅ "(Partially) Supported", ❌ "Not supported", ? "Unknown/Untested or Planned") makes the gaps concrete.
`updater` is ❌ on both iOS and Android, which is correct — the stores own updates — but it means the desktop and mobile update stories are different code paths.
`autostart`, `single-instance`, `positioner` and `window-state` are ❌ on mobile.
`fs`, `global-shortcut`, `persisted-scope`, `process`, `shell`, `stronghold`, `websocket` and `localhost` are `?` on mobile, i.e. untested.
`http`, `store`, `sql`, `notification`, `dialog`, `deep-link`, `log`, `os`, `opener`, `clipboard-manager` and `upload` are ✅ everywhere, which covers most of what we need.

The webview per platform, from wry's README (<https://github.com/tauri-apps/wry>, retrieved 2026-09-02): "WebView2 provided by Microsoft Edge Chromium" on Windows, "WebKit is native on macOS", "WebKitGTK is used to provide webviews on Linux which requires GTK".
iOS is WKWebView and Android is the Android System WebView; wry's README describes the Android setup through `WryActivity` and JNI without naming the engine, so the engine identity on mobile is inference from the platform rather than a quoted claim.

The decisive mobile finding is the cookie API.
`WebviewWindow::cookies` (<https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindow.html>, retrieved 2026-09-02, tauri 2.11.5) "Returns all cookies in the runtime's cookie store for all URLs including HTTP-only and secure cookies", with two annotations that matter: "**Android**: Unsupported, always returns an empty `Vec`", and "On Windows, this function deadlocks when used in a synchronous command or event handlers".
So the login-in-webview-then-read-the-cookie design works out of the box on macOS, Windows, Linux and (by omission from the unsupported list) iOS, and needs a hand-written Android plugin.
That plugin is small — Android's own `CookieManager.getCookie(url)` is the whole API — but it is work we own and it is a maturity signal about mobile Tauri generally.

Open Tauri issues carrying the mobile platform labels, as displayed on 2026-09-02, are about a dozen each: eleven or twelve under `platform: Android` and eleven or twelve under `platform: iOS`.
Reading the titles rather than the count is more informative: they are packaging and signing papercuts ("iOS export options computed from the pbxproj miss provisioningProfiles", "Android AAB build compiles the first Rust target twice", "iOS: WKWebView shrinks to ~half screen width after returning from background"), not architectural holes.
That is the profile of a young-but-working mobile story: it will cost days of yak-shaving per platform, not a rewrite.

The `http` plugin is a `reqwest` re-export and is ✅ on Android and iOS (<https://v2.tauri.app/plugin/http-client/>, retrieved 2026-09-02), which independently confirms that our native transport runs unchanged on both mobile OSes.
Minimum OS versions are `iOS 13.0` by default via `iOS.minimumSystemVersion` and Android `minSdkVersion` 24; both are from a search of Tauri configuration docs and pull requests rather than a single retrieved page, so they are UNVERIFIED at the exact number.

Verdict: one codebase, our SvelteKit console reused verbatim, the Rust core compiled in as a crate rather than shipped as a sidecar, five surfaces from one repository.
The tax is Android cookie access, Linux WebKitGTK, per-store signing, and mobile plugin gaps discovered by hitting them.

### 4.2 Dioxus

Dioxus 0.7 shipped 8 September 2025 (<https://dioxuslabs.com/blog/release-070/>, retrieved 2026-09-02) with hot-patching across web, desktop and mobile, iPad support, and `dx serve --platform android` against physical devices.
The mobile renderer remains the system webview — "currently a thin wrapper on top of the desktop renderer since both renderers use the webview" — with the new WGPU renderer, Dioxus Native over Blitz, as the alternative path.
The maturity language is explicit and self-limiting: "Blitz is still very young and doesn't always produce the best outputs", "Blitz is still considered a 'work in progress'", "Not every CSS feature is supported yet", and performance is a known gap.

Verdict: strong Rust story, but the UI is RSX rather than Svelte, so the console is rewritten.
Choosing Dioxus trades a working console for a framework whose differentiating renderer is not ready.
Not recommended.

### 4.3 Flutter with flutter_rust_bridge

flutter_rust_bridge v2 (<https://cjycode.com/flutter_rust_bridge/>, retrieved 2026-09-02) generates Dart bindings from ordinary Rust — "even with arbitrary types, closure, &mut, async, traits" — and claims "Support Android, iOS, Windows, Linux, MacOS, and Web."
It is the most polished Rust-to-app-framework bridge available and Flutter's mobile maturity is not in question.

Verdict: full Rust reuse, zero UI reuse, and a second language and toolchain for the team.
The console is rewritten in Dart, and the web build is Flutter Web, which is a worse web console than the Svelte one we have.
Not recommended while the SvelteKit console exists.

### 4.4 React Native or Expo over UniFFI

UniFFI (<https://mozilla.github.io/uniffi-rs/latest/>, retrieved 2026-09-02) "automatically generates foreign-language bindings targeting Rust libraries", supports "consolidating business logic in a single Rust library while targeting multiple platforms", and ships "full support for Kotlin, Swift and Python", with third-party bindings that "may require older versions of UniFFI and may have partial or non-existant support for some features".
It states plainly that it "will not help you ship a Rust library to these platforms", only generate the bindings.

The React Native route goes through `uniffi-bindgen-react-native` (<https://github.com/jhugman/uniffi-bindgen-react-native>, retrieved 2026-09-02), which generates TypeScript bindings using "JSI C++ to call Rust from TypeScript and back again, and a Turbo-Module that installs the bindings".
It is led by James Hugman with Mozilla as one of several funders rather than as maintainer, and it is mid-rename — "In the near future, we'll change the name to `uniffi-bindgen-javascript`" — which signals a transitional API surface.

Verdict: full Rust reuse, no Svelte reuse, and the bindings layer is a third-party project in flux.
Not recommended as the primary path; worth remembering if a native-feeling mobile UI later becomes a product requirement.

### 4.5 Kotlin Multiplatform, or plain native shells, over a UniFFI core

Compose Multiplatform (<https://kotlinlang.org/compose-multiplatform/>, retrieved 2026-09-02) labels iOS "Stable" and web "Beta", with Android via Jetpack Compose and desktop on Windows, macOS and Linux, and names production users including Wrike, Physics Wallah, Markaz and Bilibili's China app.
That is a genuinely mature mobile stack, and the same shape applies to hand-written SwiftUI plus Jetpack Compose shells over a UniFFI core.

Verdict: the best native user experience and the highest ongoing cost, because it is two or three UI codebases plus the Rust core plus the bindings layer, and the web console is still separate.
It is what you build when the mobile experience is the product; ours is a control surface over a server-mediated workflow.
Not recommended now.

### 4.6 Capacitor over the existing SvelteKit build

Capacitor (<https://capacitorjs.com/docs>, retrieved 2026-09-02, current major version 8) is "a cross-platform native runtime that makes it easy to build performant mobile applications that run natively on iOS, Android, and more using modern web tooling", with "a Plugin API for Swift on iOS, Java on Android, and JavaScript for the web".
Our `adapter-static` build drops straight in.

Verdict: the fastest possible path to an app-store presence for the console, and the wrong tool for the data plane, because getting our Rust core into a Capacitor app means writing the same Swift and Kotlin plugin work Tauri already did, without Tauri's Rust-first ergonomics.
Its honest use is as a fallback if Tauri mobile proves unworkable and we decide mobile is catalogue-and-analytics only.

### 4.7 Summary

| Option | Codebases | SvelteKit reuse | Rust core reuse | Honest tax |
|---|---|---|---|---|
| Tauri v2 | one | full | full, compiled in | Android cookie plugin, WebKitGTK on Linux, mobile plugin gaps, per-store signing |
| Dioxus 0.7 | one | none | full | console rewritten in RSX; Blitz explicitly work-in-progress |
| Flutter + flutter_rust_bridge | one app, plus web console | none | full | second language and toolchain; Flutter Web is a downgrade for the console |
| React Native + UniFFI | one app, plus web console | none | full | bindings project mid-rename, third-party; new-architecture-only |
| KMP or native shells + UniFFI | two or three | none | full | best native UX, highest maintenance |
| Capacitor + WASM | one | full | poor — needs the same native plugin work | good for a console app, wrong for the data plane |

## 5. What a mobile app can actually do for a no-API marketplace

### 5.1 Session capture works, on both platforms

On iOS, `WKHTTPCookieStore` is "An object that manages the HTTP cookies associated with a particular web view", retrieved from `WKWebsiteDataStore` in the web view's configuration, exposing `getAllCookies(_:)`, `setCookie(_:completionHandler:)` and `delete(_:completionHandler:)` (<https://developer.apple.com/tutorials/data/documentation/webkit/wkhttpcookiestore.json>, retrieved 2026-09-02; available iOS 11.0+).
WebKit's own implementation of `getAllCookies` forwards directly to the cookie store's `cookies()` with no HttpOnly filtering and no `httpOnly` mention anywhere in the file (<https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebKit/UIProcess/API/Cocoa/WKHTTPCookieStore.mm>, retrieved 2026-09-02).
So an iOS app that hosts a marketplace login in a WKWebView can read the resulting HttpOnly session cookie and hand it to `reqwest`.

On Android, `CookieManager.getCookie(url)` is backed by Chromium's `android_webview/browser/cookie_manager.cc`, whose `GetCookieListAsyncHelper` — used by both `GetCookie` and `GetCookieInfo` — builds `net::CookieOptions options = net::CookieOptions::MakeAllInclusive();` and passes it to `GetCookieList` (<https://chromium.googlesource.com/chromium/src/+/refs/heads/main/android_webview/browser/cookie_manager.cc>, retrieved 2026-09-02).
`MakeAllInclusive` includes HttpOnly, so the embedding app can read the session cookie too.
This contradicts the widespread belief that Android's `getCookie` strips HttpOnly, and it is worth stating because the belief would otherwise kill the Android design on paper.
The gap is Tauri-side rather than platform-side: `WebviewWindow::cookies` is documented "**Android**: Unsupported, always returns an empty `Vec`", so we write a small Tauri Android plugin that calls `CookieManager` through JNI.

Both mechanisms have the same caveat: they require the login to happen in a webview we host, not in the user's own browser.
That is unchanged from the desktop design and is already the accepted consequence of the Chrome 136 finding in the prior note.

### 5.2 Scheduling does not work, on either platform

iOS gives a `BGAppRefreshTask` — "An object representing a short task typically used to refresh content that's run while the app is in the background", requiring the `fetch` `UIBackgroundModes` capability (<https://developer.apple.com/tutorials/data/documentation/backgroundtasks/bgapprefreshtask.json>, retrieved 2026-09-02).
Apple's strategy guidance is explicit about both the budget and who controls it: "The system decides the best time to launch your background task, and provides your app up to 30 seconds of background runtime. Complete your work within this time period and call `setTaskCompleted(success:)`, or the system terminates your app" (<https://developer.apple.com/tutorials/data/documentation/backgroundtasks/choosing-background-strategies-for-your-app.json>, retrieved 2026-09-02).
`BGProcessingTask` gets longer, but again "the system decides the best time to launch your background task".
A background push wakes the app for "up to 30 seconds", and "If you send background pushes more frequently than three times per hour, the system imposes rate limitations."
Foreground background-time via `beginBackgroundTask(withName:expirationHandler:)` is "a finite amount of time" discoverable through `backgroundTimeRemaining`, and "If you don't call endBackgroundTask(_:) for each task before time expires, the system kills the app" (<https://developer.apple.com/tutorials/data/documentation/uikit/uiapplication/beginbackgroundtask(withname:expirationhandler:).json>, retrieved 2026-09-02).
There is no cron on iOS, and 30 seconds does not cover one TPT create with a multipart upload.

Android is no better for unattended work.
WorkManager's minimum periodic interval is 15 minutes, and "The exact time that the worker is going to be executed depends on the constraints that you are using in your WorkRequest object and on the optimizations performed by the system" (<https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work>, retrieved 2026-09-02).
In Doze the system "Suspends network access", "Ignores wake locks", "Defers standard `AlarmManager` alarms", "Doesn't let `JobScheduler` run", and the documentation adds the line that settles it: "`WorkManager` uses `JobScheduler` internally, so `WorkManager` tasks don't run" (<https://developer.android.com/training/monitoring-device-state/doze-standby>, retrieved 2026-09-02).
The escape hatches are all closed to us: `setExactAndAllowWhileIdle` cannot "fire alarms more than once per nine minutes, per app", and requesting a battery-optimisation exemption is barred by policy — "Google Play policies prohibit apps from requesting direct exemption from Power Management features—Doze and App Standby—in Android 6.0 and above unless the core function of the app is adversely affected."
A foreground service would keep running, but apps targeting Android 12 or higher "can't start foreground services while the app is running in the background, except for a few special cases" and otherwise get a `ForegroundServiceStartNotAllowedException` (<https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start>, retrieved 2026-09-02), so a foreground service cannot be the thing that starts itself on a schedule.

The conclusion is firm and should be stated to users rather than engineered around: a phone can run a sync the seller starts, and cannot be the deterministic scheduler the charter requires.

### 5.3 Large files

`UPLOAD_BODY_BYTES_MAX` is 256 MiB (`crates/tam-limits/src/lib.rs:105`), and the prior note records that TPT's S3 multipart cuts at 5 MiB, so a large resource is dozens of signed parts.
On mobile that is a long-running upload on a possibly-metered connection, and it must survive the app going to background — which is exactly the case `beginBackgroundTask` does not cover for minutes-long work, and which Apple's own guidance redirects to `URLSession` background transfers ("If the task is one that takes some time, such as downloading or uploading files, use `URLSession`").
`reqwest` is not `URLSession`, so a long mobile upload either runs while the app is foregrounded and the screen is on, or it needs a native `URLSession` background-transfer path we would have to write and which would not use our transport at all.
Practically: mobile publishing should be scoped to resources the seller is uploading from the phone now, with the app open, and the 200 MB PDF should be a desktop or web-console job.

### 5.4 What "same functionality as web" honestly means

| Capability | Web console alone | Web + extension | Desktop app | iOS app | Android app |
|---|---|---|---|---|---|
| Browse catalogue, edit copy, see analytics | yes | yes | yes | yes | yes |
| Connect a marketplace account (webview login) | no | yes, the browser's own session | yes | yes | yes, via our JNI plugin |
| Originate a marketplace read | no, CORS | yes | yes | yes | yes |
| Originate a marketplace write or publish | no, CORS | yes | yes | yes, foreground | yes, foreground |
| Large multipart upload | no | yes, while the tab is open | yes | fragile, foreground only | fragile, foreground only |
| Unattended scheduled sync | no | no, `chrome.alarms` needs the browser open | yes | no | no |
| Photo capture into a listing | no | no | no | yes, natural | yes, natural |

Read down the "web console alone" column: it is empty for everything that touches a marketplace.
That is the finding that reshapes the plan.

## 6. App store policy risk

### 6.1 Apple

Guideline 5.2.2 is the one that matters and it is short: "If your app uses, accesses, monetizes access to, or displays content from a third-party service, ensure that you are specifically permitted to do so under the service's terms of use. Authorization must be provided upon request." (<https://developer.apple.com/app-store/review/guidelines/>, retrieved 2026-09-02; the page shows no last-updated date.)
This is the legal memo's cease-and-desist pivot with a second actor attached: a complaint from TPT or Tes to Apple triggers a demand for authorisation we cannot produce, and the app comes down.
Nothing in the client-side architecture changes that, because 5.2.2 is about permission and not about request origin.

Guideline 2.5.2 constrains how the adapter's knowledge is delivered: apps "may not download, install, or execute code which introduces or changes features or functionality of the app".
This is the same discipline Manifest V3's remote-code rule imposes, and the prior note's observation holds on both platforms at once: ship selectors and route maps as signed *data*, never as code, and one discipline satisfies Apple, Google and the S1/S2 legal line together.
Guideline 4.7.2 adds "Your app may not extend or expose native platform APIs or technologies to the software without prior permission from Apple", which is another reason the marketplace webview must never reach `invoke()`.

Guideline 4.2 minimum functionality is a real but manageable risk, since our app "should include features, content, and UI that elevate it beyond a repackaged website", and 4.2.2 bars apps that are "primarily … web clippings, content aggregators, or a collection of links".
A Tauri app whose UI is our SvelteKit console needs native affordances — photo capture into a listing, push notifications on sync completion, the on-device connection vault — to be comfortably clear of this, and those are worth building for their own sake.

Guideline 5.1.1(v) is worth reading closely even though marketplaces are not social networks: "An app may not store credentials or tokens to social networks off of the device and may only use such credentials or tokens to directly connect to the social network from the app itself while the app is in use."
The first clause is exactly our new architecture and cuts in our favour; the second clause, "while the app is in use", is another reason not to promise unattended mobile sync.

On money, 3.1.1 requires in-app purchase to "unlock features or functionality within your app" and bans "their own mechanisms to unlock content or functionality, such as license keys", which collides head-on with the Ed25519 entitlement design if that artifact is what gates the app's features.
The 2025-2026 US position is a genuine relief valve and is stated in the guidelines themselves: 3.1.3 says apps "cannot, within the app, encourage users to use a purchasing method other than in-app purchase, except for apps on the United States storefront", and 3.1.1(a) says the external-link entitlements "are not required for developers to include buttons, external links, or other calls to action in their United States storefront apps."
3.1.3(b) Multiplatform Services permits access to content acquired on other platforms "provided those items are also available as in-app purchases within the app", which is a constraint rather than a clean exemption.
Practice diverges from the strict reading: Vendoo's App Store listing shows Free with no in-app purchases displayed, and Crosslist's likewise (<https://apps.apple.com/us/app/vendoo-a-sellers-best-friend/id1612168777> and <https://apps.apple.com/us/app/crosslist/id6756124351>, both retrieved 2026-09-02), so both are running sign-in-to-a-web-subscription without IAP today.
That is observed behaviour, not a sanctioned pattern, and the safe plan is to assume IAP will be required outside the US and to price for a 15 to 30 per cent cut on non-US mobile signups.

### 6.2 Google Play

The Device and Network Abuse policy prohibits apps that "interfere with, disrupt, damage, or access in an unauthorized manner the user's device" and, in its enumerated list, "Apps that access or use a service or API in a manner that violates its terms of service" (<https://support.google.com/googleplay/android-developer/answer/9888379>, retrieved 2026-09-02; the page shows no last-updated date, only a "©2026 Google" footer).
That is the same exposure as Apple 5.2.2 in different words.
The same policy bars downloading "executable code, such as dex files or native code, from a source other than Google Play", which again points at signed data rather than shipped logic.

Payments: Google Play's billing system is required for "Subscription services" and "App functionality or content (such as an ad-free version of an app)" (<https://support.google.com/googleplay/android-developer/answer/10281818>, retrieved 2026-09-02).
User-choice billing exists in eligible countries; South Korea and India permit alternative billing "with fees reduced by 4%"; the EEA has DMA-driven programs; and in the United States, "due to a court order, developers enrolled in applicable programs may offer users in the U.S. additional alternative billing options or lead users in the U.S. to external content outside of the app."
The shape mirrors Apple's: the US is comparatively open, the rest of the world is not.

### 6.3 Evidence of practice

Vendoo ships an iOS app, "Vendoo: A Seller's Best Friend", seller Vendoo, Inc., Productivity, free, version 3.2.7, whose listing bills it as "The #1 Crossposting App for Resellers" and says "List your items once, crosspost to 8 top marketplaces, and manage everything in one place" and "Delist, relist, and mark items as sold or not listed", naming eBay, Etsy, Poshmark, Mercari, Grailed, Depop, Vestiaire Collective and Whatnot.
Crosslist ships iOS and Android apps (App Store id6756124351, Play `com.crosslist.app`), seller Crosslist BV, Productivity, free, 23 MB, version 0.4.4, 4.7 stars from 242 ratings, with a listing that promises "Auto-post your listings in the background without manual intervention", bulk relisting and delisting, and integration with "11+ different marketplaces", and a product page claiming "Anything you can do on a computer with Crosslist®, you can do on your phone or tablet" and "no mobile-only restrictions" (<https://crosslist.com/features/mobile-app>, retrieved 2026-09-02).
PrimeLister ships "Poshmark Bot: PrimeLister" on the App Store (id6478108527) with auto-share, auto-relist and auto-offer features, while its cross-listing product remains a Chrome extension.
List Perfectly appears to remain extension-only with no native app.

Two readings, both worth carrying.
The permissive one: apps that automate third-party marketplaces without those marketplaces' permission are on both stores today, some of them named "Bot", and review is not catching them.
The cautious one: none of those marketplaces has complained to Apple or Google yet, and 5.2.2 exists precisely so that the store can act the day one does.
Crosslist's claim that its mobile app has full parity including background auto-posting is also a claim I could not verify technically, and given section 5.2 it is either using foreground-only execution, or push-triggered short bursts, or the claim is marketing.

## 7. Browser extensions as the mobile-adjacent path

Chrome for Android does not support extensions, and the position has not changed as of 2026; Chromium's tracker carries a long-standing feature request (issue 40150046) and separate work to enable extensions on "experimental desktop android builds" (issue 356905053) where recent commits explicitly note the code "is currently experimental and has no production behavior changes" (retrieved 2026-09-02 via search over `issues.chromium.org` and `developer.chrome.com`; no single developer.chrome.com page states the exclusion, so this rests on the tracker rather than on documentation).

Firefox for Android supports open extensions.
Mozilla opened the ecosystem on 14 December 2023 with "more than 450 new extensions", and passed 1,000 fewer than five months later (<https://blog.mozilla.org/addons/2024/05/02/1000-firefox-for-android-extensions-now-available/>, retrieved 2026-09-02).
Distribution is through addons.mozilla.org with an Android-compatible flag on the listing.

Safari supports web extensions on iOS from iOS 15, but only inside an app.
Apple states: "You implement a Safari web extension as a macOS, visionOS, or iOS app extension to provide a safe and secure distribution and usage model. You can distribute a Safari web extension with a Mac app, a visionOS app, an iOS app, or a Mac app created using Mac Catalyst. Use Xcode to package your extension for testing and distribution, and join the Apple Developer Program to distribute Safari web extensions", and "Safari web extensions are available in macOS with Safari 14 and later, visionOS 1 and later, and iOS 15 and later" (<https://developer.apple.com/tutorials/data/documentation/safariservices/safari-web-extensions.json>, retrieved 2026-09-02).
So a Safari extension is an App Store submission with all of section 6's exposure, plus a converter step, plus a second review surface.

Orion for iOS and iPadOS supports Chrome and Firefox extensions in beta: "the first web browser to offer preliminary support for Chrome and Firefox browser extensions on iOS and iPadOS", with "further limitations in the scope of APIs we can support which are imposed by Apple", so "a smaller number of extensions that are currently fully functional on iOS and iPadOS", and it notes that "Apple uses closed, proprietary APIs for Safari extensions rather than WebExtensions APIs" (<https://help.kagi.com/orion/browser-extensions/ios-ipados-extensions.html>, retrieved 2026-09-02).
Orion's install base is small enough that it is a curiosity rather than a channel.

Edge for Android's extension support and Kiwi Browser's current status were not verified and are recorded as unknown.

The scheduling limit on any extension is the same everywhere: `chrome.alarms` states "Chrome limits alarms to at most once every 30 seconds but may delay them an arbitrary amount more", "setting `delayInMinutes` or `periodInMinutes` to less than `0.5` will not be honored", and "Alarms continue to run while a device is sleeping. However, an alarm will not wake up a device" (<https://developer.chrome.com/docs/extensions/reference/api/alarms>, retrieved 2026-09-02).
An extension only runs while the browser runs, so it is not a scheduler either.

Net: extensions solve the web surface's data plane on desktop, add Firefox for Android as a bonus, and do nothing useful for iOS or for scheduling.

## 8. Where the scheduler and the session live

Three viable placements, each mapped to what it costs and how it sits with the S1 line from the prior note (server sends declarative intent; the client composes and originates; the user owns the schedule).

The desktop app as the always-on agent is the only placement that preserves determinism.
A local timer fires, the client pulls pending declarative work, the local adapters compose and issue, and the server never says "do it now".
It is the prior note's recommendation, it is unchanged by anything in this note, and its weakness is that freshness depends on the seller's machine being on — a cost Vendoo concedes publicly.

The extension with a native companion moves the request into the seller's real browser session and keeps the scheduler in the native host, which is the strongest detectability position and the weakest distribution position, because the store becomes a takedown lever we do not control.
It also carries the redirect-classification rework: `Policy::none()` and the 302 `Location` read in `crates/tam-marketplace-tpt/src/live.rs` have no equivalent in a browser `fetch`, where `redirect: "manual"` yields an opaque status-0 response, recoverable through the final URL or `chrome.webRequest.onHeadersReceived` but only after re-verifying against the captures.

The mobile "run now" that wakes a desktop agent is the honest answer to the founder's mobile requirement.
The phone shows state, lets the seller author and photograph, and dispatches an intent; a desktop agent or the seller's own next foreground session executes it.
This is not a compromise on the legal position — the request still originates on the seller's device, just a different one of the seller's devices — and it is the only design in which the phone's presence never weakens the deterministic-schedule guarantee.
What it costs is a clear statement in the product that unattended sync requires the desktop agent, which is precisely the statement Vendoo already makes about scheduled work.

The session lives with the surface that captured it, in the OS keychain or Keystore, and is never uploaded.
Two consequences the founder should decide on rather than discover: a seller who logs in on the phone has a session the desktop agent cannot use, so either each surface authenticates separately (simplest, most defensible, more friction) or we build an end-to-end-encrypted session relay between the seller's own devices (better UX, reintroduces a custody question our whole architecture exists to remove).
The recommendation is per-surface login, and to say so plainly in the UI.

## 9. Candidate delivery architectures

Effort figures assume one senior engineer, full-time, already fluent in this tree; calendar weeks at roughly four focused days; excluding legal review, marketing, the live login-challenge probe, and store-account waiting time; including CI wiring, signing setup and one submission cycle per store.
Confidence is roughly ±30 per cent on the seam work, which is well-understood, and ±50 per cent on anything crossing a store or a webview we have not yet driven.
Every option shares two prerequisites: a `live` feature gating `reqwest` in both adapter manifests, and the `driver.rs` split behind a repository trait.

### A1 — Web console plus Tauri desktop, mobile deferred

Surfaces: web console (read-only over the marketplace), Windows, macOS, Linux desktop.
Rust reuse: 100 per cent of the 28,151 portable lines; `ReqwestTransport` runs unchanged.
UI reuse: 100 per cent of `web/`.
New code: the `live` feature gate; the `MaybeSend` shim; the `driver.rs` split; login webview and `cookies_for_url` capture; local scheduler; entitlement verification; OS keychain storage; installers and signing for three OSes; updater with its irreplaceable minisign key.
Effort: 10 to 16 weeks.
Ongoing tax: Apple $99/yr, Windows signing (UNVERIFIED, about $120/yr), a five-job CI matrix with an unavoidable macOS runner, WebKitGTK breakage on Linux.
What a teacher can do: everything, from the desktop; from a phone, nothing.
This is the prior note's recommendation and it does not satisfy the founder's brief.

### A2 — Web console plus Tauri everywhere: desktop, iOS, Android

Surfaces: web console, three desktop OSes, iOS, Android.
Rust reuse: 100 per cent, same crate on every target.
UI reuse: 100 per cent, one SvelteKit build in five shells.
New code: everything in A1, plus a Tauri Android plugin wrapping `CookieManager` through JNI (because `WebviewWindow::cookies` is Android-unsupported); mobile-appropriate navigation over the existing console; camera capture into a listing; `BGAppRefreshTask` and WorkManager wiring for opportunistic refresh only; push for sync-completion notification; App Store and Play submissions including the 12-tester, 14-consecutive-day Play gate for a personal account.
Effort: A1 plus 4 to 6 weeks for iOS plus 4 to 6 weeks for Android, so 18 to 28 weeks total.
Ongoing tax: A1's, plus two store review cycles per release train, plus mobile plugin gaps discovered by hitting them, plus a macOS runner that is now on the critical path for every release.
What a teacher can do: full parity for anything they start themselves, on every surface; unattended scheduled sync only where the desktop agent runs.
This is the closest honest match to the founder's brief.

### A3 — Web console plus browser extension plus Tauri desktop, mobile as a Capacitor catalogue companion

Surfaces: web console with an extension data plane on Chrome, Edge, Firefox desktop and Firefox for Android; Tauri desktop as the scheduling agent; a Capacitor-wrapped console on iOS and Android for catalogue, photos, analytics and "run now".
Rust reuse: 100 per cent on desktop; on the extension, the pure core compiles to wasm unchanged and the adapters need the redirect-classification rework; on mobile, none, because the Capacitor app originates no marketplace request.
UI reuse: 100 per cent everywhere.
New code: A1's, plus MV3 extension (service worker, `host_permissions`, wasm build with `wasm-unsafe-eval` verified, redirect classification rework, Chrome and Firefox store submissions, Mozilla's reproducible-source requirement for wasm), plus a thin Capacitor shell.
Effort: A1 plus 6 to 8 weeks for the extension plus 2 to 3 weeks for Capacitor, so 18 to 27 weeks.
Ongoing tax: three app stores plus two extension stores, and the extension stores are a takedown surface.
What a teacher can do: publish from any desktop browser without installing an app; from the phone, manage inventory and dispatch work but not originate a marketplace write.

### A4 — The different approach: web console plus browser extension only

Surfaces: web console, and a browser extension as its data plane, on Chrome, Edge and Firefox desktop plus Firefox for Android.
No desktop app, no iOS app, no Android app.
Rust reuse: the 16,169-line pure core compiles to wasm with no changes at all; the 11,982 lines of adapter logic compile once the `live` feature gate exists; only the 873 transport lines are replaced by a `fetch`-backed transport, and the redirect classification is reworked.
UI reuse: 100 per cent; the extension's options and popup pages are the same SvelteKit build.
New code: the `live` gate, the `MaybeSend` shim, the `driver.rs` split, a wasm transport, the redirect rework, MV3 packaging and two extension-store submissions.
Effort: 10 to 14 weeks, the fastest path to a client-side product.
Ongoing tax: two extension stores, no code signing, no notarization, no macOS runner, no Apple or Google developer fee, no per-OS installers.
What a teacher can do: everything except unattended scheduled sync, and only while a browser is open on a computer they are at.
Why it is worth taking seriously: it abandons the mobile and desktop scope entirely, is roughly half the cost of A2, has no gatekeeper in the payment path, and is the architecture both extension-based incumbents chose.
Why it fails the brief: no iOS, no Android, no cron, and the store takedown lever the prior note called disqualifying.

### A5 — Comparison

| | Surfaces covered | Rust reuse | UI reuse | Effort (weeks) | Store surfaces | Deterministic cron |
|---|---|---|---|---|---|---|
| A1 | web console, 3 desktop OSes | 100% | 100% | 10-16 | 0 | yes, desktop |
| A2 | web console, 3 desktop OSes, iOS, Android | 100% | 100% | 18-28 | 2 | yes, desktop only |
| A3 | + extension, mobile as companion | 100% desktop, partial wasm, none mobile | 100% | 18-27 | 5 | yes, desktop |
| A4 | web console + extension only | core 100%, transport replaced | 100% | 10-14 | 2 | no |

## 10. Recommended sequence

The ordering principle is that the data plane, not the surface, is the unit of work, and that every step be independently useful.

Step 1, one week: add a `live` Cargo feature to both adapter manifests, move `reqwest` to `optional = true`, gate `live.rs` behind it, and add the `MaybeSend` cfg shim at `crates/tam-marketplace/src/transport.rs:293`.
Add `aarch64-apple-ios`, `aarch64-linux-android` and `wasm32-unknown-unknown` to `rust-toolchain.toml` and prove `cargo check --target` for the pure core plus the un-gated adapters on each, so the whole portability argument in section 3 becomes a CI job rather than an argument.
This is cheap, reversible, and it converts the largest unverified claim in this note into a green check.

Step 2, four to six weeks: split `crates/tam-engine/src/driver.rs` from concrete `tam-storage` repositories behind a repository trait so the effect interpreter runs against a remote ledger.
Every later surface needs this and none of it is wasted if the surface choice changes.

Step 3, six to ten weeks, the first shippable milestone: **a Tauri v2 desktop client for Windows and macOS**, webview-for-login-only, `ReqwestTransport` unchanged, client-side scheduler, OS-keychain session storage, Ed25519 entitlement verification, signed installers and the minisign-keyed updater.
Linux best-effort, as the prior note recommended, because WebKitGTK is the largest maintenance liability and it is confined there.
Size: roughly 3,000 to 5,000 new lines, almost all of it shell and packaging, against 28,151 lines of unchanged engine.
It is shippable because it is a complete product for a seller with a computer, and it is the milestone that proves the whole client-side thesis against real marketplaces.

Step 4, six to eight weeks: the MV3 browser extension as the web surface's data plane, which is the step this note promotes relative to the prior note's ordering.
The reason for the promotion is section 5.4: without it, "web app" cannot mean a surface that publishes, and the founder listed the web app first.
The redirect-classification rework against the existing captures is the technical risk and should be spiked in week one of this step.

Step 5, four to six weeks: iOS, reusing the same Tauri project and SvelteKit build, WKWebView login plus `WKHTTPCookieStore`, foreground-only publishing, camera capture, "run now" dispatch to the desktop agent, App Store submission.

Step 6, four to six weeks: Android, identical to step 5 plus the JNI `CookieManager` plugin, WorkManager for opportunistic refresh, Play submission with the 12-tester gate started at the beginning of step 5 so it is not on the critical path.

If the founder wants a mobile presence sooner than step 5 allows, the cheapest honest move is a Capacitor wrap of the existing console shipped alongside step 3, marketed as inventory, photos, analytics and "run now" — two to three weeks, no Rust, no new architecture, and it converts to the real app later without wasting the UI work.

## 11. Open questions for the founder

1. Does "same functionality" survive the finding that no phone can run a deterministic schedule?
   Recommendation: yes, redefined as full parity for user-initiated work, with unattended sync stated in the product as requiring the desktop agent.
   This needs founder words, because it is a public product claim.
2. Is the browser extension promoted from "later addition" to "the web surface's data plane", ahead of mobile?
   Recommendation: yes, step 4, on the grounds that without it the web app cannot publish at all.
3. Per-surface marketplace login, or an end-to-end-encrypted session relay between the seller's own devices?
   Recommendation: per-surface login, stated plainly in the UI; the relay reintroduces a custody question the architecture exists to remove.
4. Do we plan for in-app purchase outside the US storefront, at a 15 to 30 per cent cut, or do we ship US-first and defer?
   Recommendation: ship US-first with an external link, and treat non-US IAP as a funded later milestone.
   This needs founder words, because it changes pricing.
5. Which platform pair launches first — Windows and macOS desktop, or iOS and Android?
   Recommendation: desktop, because it is the only surface that satisfies the deterministic-schedule non-negotiable and it is the milestone that proves the thesis.
6. Do we accept two app-store gatekeepers on top of the marketplace cease-and-desist risk, given Apple 5.2.2's "Authorization must be provided upon request"?
   Recommendation: yes, but with a documented contingency — the desktop client and the direct web console must remain fully functional if either store removes us, so a takedown is a channel loss and not a product loss.
7. Is the Google Play developer account registered as an organisation rather than personally, to avoid the 12-tester, 14-consecutive-day production gate?
   Recommendation: organisation, registered now, because the gate is wall-clock and cannot be compressed.

## 12. Unverified

- No cross-target compilation was performed: this machine has only `x86_64-unknown-linux-gnu` std installed and no `rustup`, so every non-native portability claim rests on upstream documentation and upstream CI rather than on a local build.
- `ring` on `x86_64-linux-android` does not appear in ring's CI matrix or in `mk/cargo.sh`; the Android emulator on x86 hosts is therefore unproven for us.
- Tauri's minimum mobile OS versions (iOS 13.0, Android `minSdkVersion` 24) come from a search across Tauri configuration docs and pull requests rather than from one retrieved page.
- The webview engine identity on Tauri iOS (WKWebView) and Android (Android System WebView) is inferred from the platform; wry's README names the engine only for Windows, macOS and Linux.
- `WebviewWindow::cookies` is documented Android-unsupported, and iOS is not listed as unsupported; that iOS therefore works has not been tested by us.
- Chrome for Android's lack of extension support rests on the Chromium issue tracker, not on a developer.chrome.com statement.
- Edge for Android and Kiwi Browser extension support were not investigated.
- Azure Trusted Signing prices render as "$-" on Microsoft's own pricing page; the prior note's roughly $120 a year is carried forward unverified.
- The Apple App Store Review Guidelines page displays no last-updated date, and the Google Play policy pages display only a "©2026 Google" footer, so neither can be dated.
- Crosslist's claim of full mobile parity including background auto-posting could not be reconciled with the iOS and Android background-execution limits in section 5.2, and is recorded as a marketing claim.
- Whether Vendoo and Crosslist hold any authorisation from the marketplaces they automate is unknown; their presence on the stores is evidence of non-enforcement, not of permission.
- The blocking gap from the prior note is unchanged and still gates everything: whether the TES and TPT login pages carry a Cloudflare or DataDome challenge an embedded webview cannot clear, on desktop or on mobile.
- The wasm redirect-classification rework has not been spiked against the existing captures, so its cost is estimated rather than measured.
- MV3's `wasm-unsafe-eval` versus wasm-bindgen's `new Function` glue (`rustwasm/wasm-bindgen#3098`) remains unverified against our own build.

## 13. Sources

All retrieved 2026-09-02.

| Area | Reference |
|---|---|
| Rust target tiers | <https://doc.rust-lang.org/rustc/platform-support.html> |
| reqwest wasm limits, version 0.13.4 | <https://docs.rs/reqwest/latest/reqwest/> |
| getrandom targets and `js` feature | <https://docs.rs/getrandom/0.2.15/getrandom/> |
| rustls crypto providers | <https://docs.rs/rustls/latest/rustls/crypto/index.html> |
| aws-lc-rs platform matrix | <https://aws.github.io/aws-lc-rs/platform_support.html> |
| ring build requirements | <https://github.com/briansmith/ring/blob/main/BUILDING.md> |
| ring CI target matrix | <https://raw.githubusercontent.com/briansmith/ring/main/.github/workflows/ci.yml> |
| ring cross-compile toolchains | <https://raw.githubusercontent.com/briansmith/ring/main/mk/cargo.sh> |
| blake3 SIMD backends | <https://github.com/BLAKE3-team/BLAKE3> |
| Tauri 2.0 stable release | <https://v2.tauri.app/blog/tauri-20/> |
| Tauri mobile prerequisites | <https://v2.tauri.app/start/prerequisites/> |
| Tauri mobile dev workflow | <https://v2.tauri.app/develop/> |
| Tauri distribution targets | <https://v2.tauri.app/distribute/> |
| Tauri http plugin, mobile support | <https://v2.tauri.app/plugin/http-client/> |
| Tauri plugin platform matrix | <https://raw.githubusercontent.com/tauri-apps/plugins-workspace/v2/README.md> |
| `WebviewWindow::cookies`, Android unsupported | <https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindow.html> |
| wry webview engines | <https://github.com/tauri-apps/wry> |
| cargo-mobile2 | <https://github.com/tauri-apps/cargo-mobile2> |
| cargo-ndk | <https://github.com/bbqsrc/cargo-ndk> |
| cross supported targets | <https://github.com/cross-rs/cross> |
| cargo-zigbuild caveats | <https://github.com/rust-cross/cargo-zigbuild> |
| Dioxus 0.7 | <https://dioxuslabs.com/blog/release-070/> |
| flutter_rust_bridge | <https://cjycode.com/flutter_rust_bridge/> |
| UniFFI | <https://mozilla.github.io/uniffi-rs/latest/> |
| uniffi-bindgen-react-native | <https://github.com/jhugman/uniffi-bindgen-react-native> |
| Compose Multiplatform status and users | <https://kotlinlang.org/compose-multiplatform/> |
| Capacitor | <https://capacitorjs.com/docs> |
| BGAppRefreshTask | <https://developer.apple.com/tutorials/data/documentation/backgroundtasks/bgapprefreshtask.json> |
| iOS background strategies, 30-second budget | <https://developer.apple.com/tutorials/data/documentation/backgroundtasks/choosing-background-strategies-for-your-app.json> |
| `beginBackgroundTask` | <https://developer.apple.com/tutorials/data/documentation/uikit/uiapplication/beginbackgroundtask(withname:expirationhandler:).json> |
| WKHTTPCookieStore API | <https://developer.apple.com/tutorials/data/documentation/webkit/wkhttpcookiestore.json> |
| WebKit `getAllCookies` implementation | <https://raw.githubusercontent.com/WebKit/WebKit/main/Source/WebKit/UIProcess/API/Cocoa/WKHTTPCookieStore.mm> |
| Chromium Android WebView cookie manager | <https://chromium.googlesource.com/chromium/src/+/refs/heads/main/android_webview/browser/cookie_manager.cc> |
| WorkManager periodic work | <https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work> |
| Doze and App Standby | <https://developer.android.com/training/monitoring-device-state/doze-standby> |
| Foreground service background-start restrictions | <https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start> |
| Apple App Store Review Guidelines | <https://developer.apple.com/app-store/review/guidelines/> |
| Apple Developer Program fee | <https://developer.apple.com/support/compare-memberships/> |
| Safari web extensions, iOS 15+ | <https://developer.apple.com/tutorials/data/documentation/safariservices/safari-web-extensions.json> |
| Google Play Device and Network Abuse | <https://support.google.com/googleplay/android-developer/answer/9888379> |
| Google Play Payments policy | <https://support.google.com/googleplay/android-developer/answer/10281818> |
| Google Play registration fee | <https://support.google.com/googleplay/android-developer/answer/6112435> |
| Google Play 12-tester requirement | <https://support.google.com/googleplay/android-developer/answer/14151465> |
| chrome.alarms 30-second floor | <https://developer.chrome.com/docs/extensions/reference/api/alarms> |
| Firefox for Android extensions, 1,000+ | <https://blog.mozilla.org/addons/2024/05/02/1000-firefox-for-android-extensions-now-available/> |
| Orion iOS extension support | <https://help.kagi.com/orion/browser-extensions/ios-ipados-extensions.html> |
| Chrome for Android extensions | <https://issues.chromium.org/issues/40150046>, <https://issues.chromium.org/issues/356905053> |
| GitHub Actions runner pricing | <https://docs.github.com/en/billing/concepts/product-billing/github-actions> |
| Azure Trusted Signing pricing (prices not rendered) | <https://azure.microsoft.com/en-us/pricing/details/trusted-signing/> |
| Vendoo App Store listing | <https://apps.apple.com/us/app/vendoo-a-sellers-best-friend/id1612168777> |
| Crosslist App Store listing | <https://apps.apple.com/us/app/crosslist/id6756124351> |
| Crosslist mobile claims | <https://crosslist.com/features/mobile-app> |
| PrimeLister App Store listing | <https://apps.apple.com/us/app/poshmark-bot-primelister/id6478108527> |
| This tree | `Cargo.toml`; `crates/*/Cargo.toml`; `crates/tam-marketplace/src/transport.rs:293`; `crates/tam-marketplace-tpt/src/live.rs:574`; `crates/tam-marketplace-tes/src/live.rs:301`; `crates/tam-marketplace-tpt/src/flows.rs:774`; `crates/tam-limits/src/lib.rs:105`; `web/svelte.config.js`; `cargo tree` and `cargo check` output, 2026-09-02 |
