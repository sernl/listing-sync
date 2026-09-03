# The browser extension as a third client surface: what fits, what fights

How much of this repository a Chrome and Firefox MV3 extension would reuse unchanged, what it must re-implement, and what in the code fights the browser.

- date: 2026-09-03
- method: read-only inspection of the working copy, plus three `cargo check` runs against `wasm32-unknown-unknown` inside the flake devShell; no file in the repository was written and no marketplace was contacted
- caveat on line numbers: the working copy holds another agent's unlanded edits in `crates/tam-engine-driver`, `crates/tam-storage`, `crates/tam-api` and `apps/desktop`, and `crates/tam-engine-driver/src/ports.rs` changed under me mid-read, so its citations are to the current working copy rather than to `main`
- logs: `r3-check-tam-engine-driver.log`, `r3-check-tam-marketplace-tpt-live.log`, `r3-check-jsonwebtoken.log` beside this file

## 0. The headline

Seventy-six percent of the Rust an extension would link already compiles for `wasm32-unknown-unknown` and is proved to on every `just pre-push`.
Two mechanical edits — one `assert!` and one `cfg` shim — raise that to eighty-nine percent.
The three things that actually fight the browser are the `Send` bound on every seam trait, the 302 `Location` read that classifies a TPT create, and the seven browser-forbidden request headers TPT's Cloudflare posture is built out of.
None of the three is a rewrite of the adapters; all three are at the transport boundary.

## 1. The portable proof

`justfile:57` names five portable targets and `wasm32-unknown-unknown` is the first of them.
`justfile:63` names the crates compiled for all five: `tam-types`, `tam-marketplace`, `tam-domain`, `tam-authoring`, `tam-taxonomy`, `tam-marketplace-tpt` and `tam-marketplace-tes`, all under `--no-default-features` (`justfile:89`).
Seven of the eight crates the question asks about are therefore proven on `wasm32` today.
The eighth, `tam-engine-driver`, is not: `justfile:74` puts it in `portable_crates_64`, which `justfile:86` adds only for the non-wasm targets.

The reason is stated at `justfile:65-74` and is a `tam-limits` edge rather than anything in the driver.
`tam-engine-driver/Cargo.toml:11` depends on `tam-limits`, and `crates/tam-limits/src/lib.rs:206-209` is a const-evaluated `assert!(usize::BITS >= 64, "byte bounds are u64 and are converted to usize at the axum boundary")`.
I ran the prescribed check and it fails exactly there, before the driver's own code is reached.

```
error[E0080]: evaluation panicked: byte bounds are u64 and are converted to usize at the axum boundary
   --> crates/tam-limits/src/lib.rs:206:5
error: could not compile `tam-limits` (lib) due to 1 previous error
```

That is the first and only error class in `r3-check-tam-engine-driver.log`; nothing behind it was reached, so it is a gate rather than a bottom line.
The comment at `justfile:66-67` records that widening the claim is a change to a founder-gated limits file, which is why the wasm leg omits the crate rather than relaxing it.
The narrower reading is that the assertion is about an axum boundary that does not exist on a client, so the honest fix is a `cfg(not(target_family = "wasm"))` on that one assertion rather than deleting it — still a founder call, but a small and well-argued one.
`justfile:70-71` already books the driver to join the portable list at step 8 of the engine-driver split, for a different reason.

Two smaller facts about the proof itself.
`flake.nix:158` runs the same loop as a flake check but its crate list omits `tam-authoring`, so the justfile and the flake prove slightly different sets; the justfile is the wider one.
`check-portable` is not in `just check`; it runs under `just pre-push` (`justfile:212`) and `nix flake check`.

The blocking crate is 278 lines and the crate it blocks is 3,569 lines.

## 2. The Send question

Every seam trait an extension would implement or call requires `Send`, in two places each: a `Send + Sync` supertrait on the trait, and a `+ Send` bound on every returned future.

Ten traits carry the supertrait.
`crates/tam-marketplace/src/transport.rs:293` is `pub trait Transport: Send + Sync`, and its one method at `:294-297` returns `impl core::future::Future<Output = Result<HttpResponse, TransportError>> + Send`.
`crates/tam-marketplace/src/lib.rs` carries five more: `MarketplaceAdapter` at `:514`, `FirstPartyExport` at `:681`, `FileSource` at `:728`, `Pause` at `:742` and `FaultPlan` at `:775`, with eleven `+ Send` future bounds between them.
`crates/tam-engine-driver/src/ports.rs` carries three: `Cancellation` at `:30`, `IdSource` at `:37` and `ItemLedger` at `:46`, with twenty `+ Send` future bounds across `ItemLedger`'s twelve methods and `LedgerInspector`'s seven.
`crates/tam-engine-driver/src/driver.rs:40` carries the tenth, `NowSource`.
Thirty-two future bounds and ten supertrait declarations across four files, and `driver.rs:496-503` requires all six of `MarketplaceAdapter`, `NowSource`, `Pause`, `ItemLedger`, `Cancellation` and `IdSource` on `run_item`.

The verdict is that a browser implementation does not compile, and the reason is narrower than "wasm futures are not `Send`".
`wasm-bindgen` 0.2.121, which `crates/tam-core-wasm/Cargo.toml:19` pins exactly, unsafely implements both marker traits for its own handle type: `~/.cargo/registry/src/index.crates.io-*/wasm-bindgen-0.2.121/src/lib.rs:173-176` reads `#[cfg(not(target_feature = "atomics"))] unsafe impl Send for JsValue {}` and the same for `Sync`.
So a struct holding `web_sys` handles satisfies the `Send + Sync` supertrait on a default single-threaded wasm build.
The future is what fails.
`js-sys-0.3.98/src/futures/mod.rs:110-112` defines `pub struct JsFuture<T = JsValue> { inner: Rc<RefCell<Inner<T>>> }`, and there is no unsafe `Send` impl beside it, so `Rc` makes every `fetch`-derived future `!Send`.

I confirmed this empirically rather than by reading alone, by checking the TPT adapter with its `live` feature against `wasm32`:

```
error[E0277]: `Rc<RefCell<js_sys::futures::Inner>>` cannot be sent between threads safely
error: future cannot be sent between threads safely
   --> crates/tam-marketplace-tpt/src/live.rs:516:51
    = help: the trait `Send` is not implemented for `(dyn FnMut() + 'static)`
note: future is not `Send` as this value is used across an await
494 |     let response = builder
    |         -------- has type `Response` which is not `Send`
note: required by a bound in `Transport::send::{anon_assoc#0}`
   --> crates/tam-marketplace/src/transport.rs:297:85
```

The bound named in the error is the one quoted above, so this is the exact failure a browser transport would hit.

Two ways out, and they differ in blast radius.
The `MaybeSend` cfg shim that `docs/notes/design/client-side-architecture.md:203` books as migration step 1 has never been written — `rg MaybeSend` over `crates` and `apps` returns nothing — and its edit surface is the forty-two sites counted above, in crates every server binary also compiles.
The alternative touches no shared crate: a browser `Transport` whose `send` spawns the `!Send` `JsFuture` on `spawn_local` and returns a `futures::channel::oneshot::Receiver`, which is `Send` when its payload is, so the impl satisfies the existing bound and the `!Send` part never crosses it.
That bridge is perhaps forty lines per seam and is the cheaper first move; the shim is the cleaner end state.
Neither is free, and the choice is a founder call because the shim edits founder-gated shared surface.

Nothing else in the adapters fights a runtime.
`crates/tam-marketplace-tpt/Cargo.toml:12-14` and the identical `tes` lines make `live` mean `dep:reqwest` and nothing else, and `tokio` appears in both crates only under `[dev-dependencies]` (`tpt` `:32`, `tes` `:31`).
The only `std::thread` and `std::net` in either crate are inside `#[cfg(test)]` modules — `tpt/src/live.rs:788-794` sits after the `#[cfg(test)]` at `:574`, and `tes/src/live.rs:391-397` likewise.
`just purity` (`justfile:47-51`) fails the build if `tokio`, `tokio-util`, `reqwest`, `sqlx` or `tam-storage` reaches `tam-engine-driver`'s normal dependency graph, so the interpreter's freedom from a runtime is enforced rather than remembered.
The desktop's own seam traits are `Pin<Box<dyn Future + Send>>` (`apps/desktop/src-tauri/src/heartbeat.rs:141-142`, `control_plane.rs:104`), but those are files the extension rewrites anyway, so relaxing them costs nothing.

## 3. HTML parsing and files

No Rust HTML parser is in the workspace at all.
A grep for `scraper`, `html5ever`, `kuchiki`, `tl`, `lol_html`, `select` and `markup5ever` across every `Cargo.toml` returns nothing, and `crates/tam-marketplace-tpt/src/form.rs:6-11` says why: "The scrape is anchored-substring rather than a parser", each value found by its `name="…"` anchor, bounded to the tag carrying it.
That is 712 lines of pure string work with no dependency to port, and it is the single largest piece of luck in this assessment — a `scraper` edge would have dragged `html5ever` and `cssparser` into the bundle, and `cssparser` is one of the MPL-2.0 crates `docs/notes/design/desktop-client.md:186` already had to write a licence exception for.
`pulldown-cmark` (`tpt/Cargo.toml:19`) is markdown rendering rather than parsing and is already proven on `wasm32` by `check-portable`.

Nothing in the adapters or the driver touches a filesystem.
`crates/tam-marketplace-tpt/src/guard.rs:16-19` states that its sources are held as `include_str!` pairs "because the contents must be readable without a filesystem" and that "`std::fs::read_to_string` is banned crate-wide besides".
The grep for `std::fs`, `tokio::fs`, `std::time::Instant` and `std::time::SystemTime` across all eight portable crates returns only that comment.

Payload bytes reach the adapters entirely in memory, and that is the constraint the browser will feel.
`crates/tam-marketplace/src/lib.rs:713-718` defines `FileContent { file_name, content_type, bytes: Vec<u8> }` and `:728-733` defines `FileSource::fetch(&self, file: FileId) -> impl Future<Output = Result<FileContent, FileSourceError>> + Send`, which returns the whole file by value.
There is no streaming variant, so a streaming upload is not possible without changing that trait.
The ceiling is real: `crates/tam-limits/src/lib.rs:105` sets `UPLOAD_BODY_BYTES_MAX = 256 * 1024 * 1024`, and `crates/tam-marketplace-tpt/src/s3.rs:25` sets `PART_SIZE = 5 * 1024 * 1024`, so a payload at the cap is fifty-two parts and 256 MiB resident in one heap.
An MV3 service worker is terminated after thirty seconds of inactivity and its heap is not a place to hold that, which makes the payload path the sharpest re-design in the whole surface — the bytes want to live in IndexedDB or an `OffscreenDocument`, read back one 5 MiB part at a time, which is exactly the streaming `FileSource` does not offer.

blake3 hashing happens in two places and both port.
`crates/tam-engine-driver/src/seed.rs:98` computes the intent hash over the rendered field set, `ContentHash(*blake3::hash(bytes).as_bytes())`, and `apps/desktop/src-tauri/src/payload.rs:255` verifies the fetched payload against the envelope's manifest digest.
Both use `blake3` with the `pure` feature (`tam-engine-driver/Cargo.toml:16`, `apps/desktop/src-tauri/Cargo.toml:44`), which `justfile:72-73` records as a founder decision taken on 2026-09-03 precisely because it drops the `cc` and `ml64.exe` paths, so there is no C build to cross.
`docs/notes/design/desktop-data-plane.md:121` states the verify order — length first, then hash — and that unverified bytes are never cached; both rules survive a move to IndexedDB unchanged.

## 4. The ports to re-implement

`apps/desktop/src-tauri/src` is 7,859 lines, of which 4,176 are production and 3,683 are tests, split at each file's first `#[cfg(test)]`.
Every count below is production lines.

`control_plane.rs` (440) is the device's own HTTP to our registry and is deliberately the only network module in the crate (`:4-10`).
An extension's is smaller: `:201` sets `.header("cookie", format!("{SESSION_COOKIE}={session}"))` by hand, and a `fetch` with `credentials: "include"` under a `host_permissions` grant does that itself, so the whole session-threading parameter at `:117-120` disappears.

`ledger.rs` (460) is the wire form of `ItemLedger` over a `LedgerTransport` seam (`:52-53`) and adds no operation of its own (`:6-8`).
It is the largest single piece that ports essentially unchanged: swap the transport impl and the rest is serde.

`payload.rs` (311) is the interim payload fetch and cache.
It shrinks in one direction and grows in another: no `tokio::fs` and no start-up sweep, but the 256 MiB-in-one-heap problem above lands here.

`work.rs` (336) is the pull, run and settle sequence, and is the module that most directly encodes D1's custody line (`:1-11`).
It ports as-is; its dependencies are the traits, not the platform.

`run.rs` (238) is the four capabilities the interpreter needs and is the most platform-bound file in the crate.
`:29` reads `std::time::SystemTime::now()`, which has no source on `wasm32-unknown-unknown`; `:66-67` implements `Pause` as `tokio::time::sleep`; and `:108`, `:144`, `:218` and `:229` build the run deadline on `tokio::time::Instant`.
All four become `Date.now()`, `setTimeout` and `performance.now()` — a small file, entirely rewritten, and the one place a JavaScript bridge is unavoidable.

`scheduler.rs` (221) is the local timer and the four refusals.
`:186` is the tick and `:119` sets `DEFAULT_CADENCE: Duration = Duration::from_hours(1)`.
The refusal logic stays in Rust; the timer becomes `chrome.alarms`.
Worth correcting the prior assessment here: `docs/notes/design/client-side-architecture.md:167` says an extension "cannot run cron at all, because `chrome.alarms` has a 30-second floor", but the cadence we actually ship is one hour, so the floor never binds — the binding constraint is the second half of that sentence, that alarms fire only while the browser is open.

`heartbeat.rs` (341) carries metadata only and structurally cannot carry a credential (`:7-12`); it ports unchanged.

`session/` (`mod.rs` 182, `keychain.rs` 89, `memory.rs` 47) mostly disappears, and this is where the extension is genuinely better than the desktop.
The desktop exists in part to hold a cookie jar the browser already holds.
What survives is one narrow need: `crates/tam-marketplace-tpt/src/session.rs:5-11` records that TPT's CSRF is a double submit, the `csrfToken` cookie value copied verbatim into `x-csrf-token`, and `docs/notes/design/desktop-client.md:57` notes `document.cookie` cannot see an `HttpOnly` cookie.
An extension with the `cookies` permission reads it through `chrome.cookies.get`, which is JavaScript, roughly forty lines.

`console_session.rs` (64) and `webview_session.rs` (66) delete outright.
Both exist only to read the `HttpOnly` console cookie out of a webview (`console_session.rs:1-8`), which an extension never needs to do.

`marketplace.rs` (340) is the join between the stored session and the transport (`:1-10`) and becomes the new `fetch` transport.
Two of its three enforced properties survive and matter: the per-request store read at `:92` becomes free, and the origin binding at `:96-98` — a session-authenticated request may reach exactly one host, and the S3 upload is exempt because it signs its own body — must be re-implemented, because in a browser it is also what keeps `host_permissions` honest.

`connect.rs` (108), `device.rs` (125), `state.rs` (223), `commands.rs` (232) and `lib.rs` (146) each shrink or move.
`connect.rs` keeps its per-marketplace logged-in condition and loses the login window entirely.
`device.rs` writes `device.json` into app data (`docs/notes/design/desktop-client.md:61`) and becomes `chrome.storage.local`.
`state.rs` holds `Arc<Mutex>` state that a service worker's eviction makes a durability problem rather than a line-count one.
`commands.rs` becomes `chrome.runtime.onMessage` and is mostly JavaScript.

`entitlement.rs` (207) ports unchanged, and I checked rather than assumed.
`apps/desktop/src-tauri/Cargo.toml:49` takes `jsonwebtoken = { version = "9.3", default-features = false }`, and an out-of-repo probe of exactly that dependency checks clean on `wasm32-unknown-unknown` in 14 seconds, compiling `ring 0.17.14`, `getrandom 0.2.17` and `jsonwebtoken 9.3.1` (`r3-check-jsonwebtoken.log`).
Two caveats: `cargo check` is not `cargo build` and does not exercise ring's wasm verification path at runtime, and `getrandom` on `wasm32-unknown-unknown` needs its `js` backend feature to have any entropy source, which verification may not reach but signing would.

## 5. The wire and the server

The server does not distinguish device surfaces, and adding one is an enum that does not exist yet rather than a migration.
`crates/tam-storage/migrations/0042_device_registry.sql:27-49` creates `device` with `org_id`, `id`, `name`, `os`, `arch`, `app_version`, `first_seen_at`, `last_seen_at` and `revoked_at`, and nothing else.
There is no surface, kind or platform column, and a grep for `DeviceSurface` or `DeviceKind` across `tam-types`, `tam-domain` and the migrations returns nothing.
`:20-25` says the device identifier is text because the device mints it itself, and `crates/tam-api/src/devices.rs:76-82` takes `RegisterBody { name, os, arch, app_version }` as free strings, bounded only by `bounded(...)` at `:228-230`.
So an extension registers today with `os: "chrome"` and `arch: "wasm32"` and needs no migration.
Whether that is right is a separate question: a seller's "Your devices" page cannot tell a laptop from a browser profile without one, and the closed `status` vocabulary at `0042:56-63` shows the house style would want a closed surface enum rather than free text, which is a founder call and a small migration.

Authentication is a plain cookie and works from an extension without the desktop's machinery.
`crates/tam-api/src/session.rs:18` names the cookie `tam_session`, `:50-58` reads it out of the `Cookie` header and nothing else, and `:69-75` makes `OrgContext` the extractor every device route takes (`devices.rs:247`, `:270`, `:286`).
`crates/tam-api/src/lib.rs:202-220` routes `/v1/devices`, `/heartbeat`, `/revoke`, `/work`, `/settle`, `/ledger` and `/payload/{file}`, all under that one extractor.
An extension holding `host_permissions` for our origin sends that cookie automatically on a `credentials: "include"` fetch and never reads it, which is strictly better custody than the desktop's route of reading it out of a webview cookie store.
The residual risk is `SameSite`: `session.rs:16-17` says the attribute is "the serving deployment's to set when it mints", so what an extension-origin request actually carries depends on a value this repository does not fix, and it must be verified against the deployed cookie rather than assumed.

The entitlement token is bound to the desktop by name and would need a second audience or a widened one.
`apps/desktop/src-tauri/src/entitlement.rs:33` fixes `AUDIENCE = "tam-desktop"` and `:36` fixes `ISSUER = "tam-server"`, and `docs/notes/design/desktop-client.md:82` records that both are in code rather than configured "because a build able to widen either would accept a token minted for something else".
An extension is a different build, so it is a different audience, and the server's minting side gains a case.
The device binding at `:118-121` is per device id and needs no change.

## 6. The wasm precedent

`crates/tam-core-wasm` is 337 lines across `src/lib.rs` (308) and a fixture bin (29), with `crate-type = ["cdylib", "rlib"]` (`Cargo.toml:8`) and four exported functions: `core_version` (`lib.rs:99`), `selection_caps` (`:111`), `check_draft` (`:144`) and `project_preview` (`:261`).
`justfile:228-232` is the whole build: `cargo build --target wasm32-unknown-unknown --release --lib -p tam-core-wasm`, then `wasm-bindgen --target web --out-name core --out-dir web/src/lib/core/generated`, honouring `CARGO_TARGET_DIR`.
`flake.nix:190-194` puts `pkgs.wasm-bindgen-cli` in the dev shell and records that the crate pins `wasm-bindgen` to exactly what nixpkgs carries, because the CLI refuses a mismatched crate version.
Measured on the artefact in the tree today: 449,880 bytes raw, 119,479 gzipped, with 9,758 bytes of glue that gzips to 2,834 — matching `docs/notes/design/wasm-core.md:121` to within a rebuild.

An extension build adds a second `cdylib` crate and a second `wasm-bindgen` invocation to that pipeline, with `--target web` replaced by a service-worker-appropriate target, and nothing else structural.
The boundary discipline transfers directly: `wasm-core.md:39-46` uses JSON strings in both directions, refuses `serde-wasm-bindgen`, and returns `{"error":{"kind":...}}` rather than panicking, because a trap poisons the module instance — which matters more in a service worker that is expected to be evicted and revived than it does on a page.

One trap the prior assessment left open is now closed.
`docs/notes/design/client-side-architecture.md:171` flagged that MV3's CSP grants `wasm-unsafe-eval` but not `unsafe-eval`, that wasm-bindgen's glue can require the latter via `new Function`, cited `rustwasm/wasm-bindgen#3098`, and marked it "**unverified** on our own build".
It is verifiable now, and it passes: `web/src/lib/core/generated/core.js` contains zero occurrences of `new Function` and zero of `eval(`, and instantiates through `WebAssembly.instantiateStreaming` at `:217-219` with a plain `WebAssembly.instantiate` fallback at `:231`.
So `wasm-unsafe-eval` suffices for our glue as generated by wasm-bindgen 0.2.121, on our own build, measured rather than reasoned.

## 7. What has changed since the prior assessment

`docs/notes/design/client-side-architecture.md` sections 7 and 9 are the prior assessment, and four of its five migration steps have moved.

Step 1, the `MaybeSend` shim at `:203`, was never done, and section 2 above is the first measurement of what it would cost: forty-two sites, not "roughly 20 lines".
Step 2, the driver split at `:204`, is done — `crates/tam-engine-driver` exists at 3,569 lines with the three ports section 3 of `engine-driver-split.md` specified.
Step 3, the native client at `:206`, is done — `apps/desktop/src-tauri` at 7,859 lines, running the adapters against their own reqwest transport.
Step 4, deleting the broker at `:207`, has not happened: `crates/tam-session-broker` is still 4,326 lines in the tree, the figure `engine-driver-split.md:96` quotes.
Step 5, revisiting the extension at `:208`, is this note.

Three of the note's specific claims should be updated.
The redirect concern at `:164` was a prediction and is now a compile error: `no method named 'redirect' found for struct 'ClientBuilder'` and `cannot find 'redirect' in 'reqwest'` at `crates/tam-marketplace-tpt/src/live.rs:145`, plus `no method named 'is_connect' found for reference '&reqwest::Error'` at `:345`.
The underlying dependency is real and specific: `crates/tam-marketplace-tpt/src/classify.rs:488` says "A 302 whose `Location` yields a product id is the only shape any capture" and `:499` reads that header, `flows.rs:820` branches on it, and `live.rs:8-11` explains that a transport chasing the redirect "would discard the identifier".
Tes has no such dependency — grep finds no `ResponseHeader::Location` and no redirect policy anywhere in `crates/tam-marketplace-tes` — so this is a TPT-create problem, not a general one.
The `wasm-unsafe-eval` trap at `:171` is resolved in our favour, per section 6.
The `chrome.alarms` claim at `:167` overstates: the floor does not bind at our one-hour cadence, only the browser-must-be-open half does.

One obstacle the prior note did not name at all, and it is the third-biggest.
`crates/tam-marketplace-tpt/src/live.rs:47-67` and `:109-130` build a per-hop header envelope that `:18-24` says is the difference between TPT answering with the form and answering with a Cloudflare interstitial.
Seven of those header names are forbidden to set from browser `fetch`: `user-agent` (`:110`), `accept-language` (`:114`), the four `sec-fetch-*` (`:57-60`, set at `:124-126` and `:551-555`), and `origin` (`:286`, `:289`).
In an extension the browser sets them itself, which is either the whole point or the whole problem: they will be the real browser's values, which is better than ours for detectability, but an extension-origin `fetch` sends `Origin: chrome-extension://<id>` and a `Sec-Fetch-Site` that is not `same-origin`, which is precisely what `live.rs:61` hardcodes.
`docs/research/rethink/vendoo-architecture-and-market.md:10` records that Vendoo solves exactly this with a `declarativeNetRequest` rule set rewriting `Origin`, `Referer` and `Access-Control-Allow-Origin`, and `:82` names it "the client-side equivalent of what our Rust adapters do with an explicit header map".
Whether DNR can also rewrite `Sec-Fetch-*` is the single thing I could not settle from this repository, and it decides whether the TPT write path works from an extension at all.

## The table

Estimates are scaled from the desktop's measured production line counts (section 4), not guessed.

| Component | Verdict | Est. lines | Language | Riskiest unknown |
|---|---|---|---|---|
| `tam-types`, `tam-marketplace`, `tam-domain` | reuse unchanged | 10,824 | Rust wasm | none; proven by `justfile:63` on every `pre-push` |
| `tam-marketplace-tpt` minus `live.rs` | reuse unchanged | 7,705 | Rust wasm | none; the scrape is substring work with no parser edge |
| `tam-marketplace-tes` minus `live.rs` | reuse unchanged | 3,733 | Rust wasm | none |
| `tam-limits` | adapt (1 line) | 278 | Rust wasm | whether the founder will `cfg` off the `usize::BITS` assert at `lib.rs:206` |
| `tam-engine-driver` | reuse unchanged once `tam-limits` moves | 3,569 | Rust wasm | nothing behind the `tam-limits` gate has ever been compiled for wasm |
| `MaybeSend` shim across 4 files | new | ~25 (+42 edit sites) | Rust wasm | it edits shared crates every server binary compiles |
| `ledger.rs` equivalent | reuse, transport swapped | ~460 | Rust wasm | none |
| `work.rs` equivalent | reuse | ~336 | Rust wasm | service-worker eviction mid-run leaves a lease held |
| `heartbeat.rs` equivalent | reuse | ~341 | Rust wasm | none |
| `entitlement.rs` equivalent | reuse, new audience | ~207 | Rust wasm | ring's wasm path is `check`-clean but not run-proven |
| `control_plane.rs` equivalent | adapt, smaller | ~330 | Rust wasm | whether `tam_session`'s deployed `SameSite` rides an extension fetch |
| `marketplace.rs` equivalent (fetch transport) | new | ~250 | Rust wasm | the seven forbidden headers, and whether DNR can set `Sec-Fetch-*` |
| Redirect classification shim | new | ~120 | JavaScript | `chrome.webRequest.onHeadersReceived` is the only route to the 302 `Location` |
| `payload.rs` equivalent | adapt, re-designed | ~230 | Rust wasm | 256 MiB (`tam-limits:105`) in a service worker heap; `FileSource` cannot stream |
| `run.rs` equivalent (clock, pause, deadline) | new | ~170 + JS bridge | Rust wasm + JS | `SystemTime::now()` has no source on `wasm32-unknown-unknown` |
| `scheduler.rs` equivalent | adapt; timer to `chrome.alarms` | ~150 + ~40 JS | Rust wasm + JS | alarms fire only while the browser runs |
| `state.rs` equivalent | adapt | ~200 | Rust wasm | service-worker eviction makes in-memory state a durability question |
| `commands.rs` equivalent | new (`runtime.onMessage`) | ~90 + ~130 JS | Rust wasm + JS | none |
| `session/` (jar, keychain) | delete | −318 | — | only `csrfToken` survives, via `chrome.cookies` |
| `console_session.rs`, `webview_session.rs` | delete | −130 | — | none |
| `connect.rs` equivalent | adapt, much smaller | ~70 | Rust wasm | the logged-in conditions are still unverified against a live login |
| MV3 manifest, DNR rules, SW entry | new | ~200 | JavaScript / JSON | store review is a takedown lever the desktop does not have |
| Device surface enum + migration | new, optional | ~60 | Rust + SQL | none; `0042` has no surface column and needs one only for the UI |

Totals: about 26,109 lines of existing Rust link into the extension, of which 22,262 (76 percent) compile for `wasm32` today and 26,109 (89 percent of the extension's ~29,400 Rust lines) do so after the `tam-limits` assert and the `Send` bounds move.
The device loop is about 3,330 Rust lines against the desktop's 4,176 — roughly twenty percent smaller, because the browser holds the session the desktop had to capture and store.
About 530 lines are irreducibly JavaScript: the alarms timer, the cookie read, the message surface, the manifest, and the redirect-header listener.
