# oxichrome, and the browser extension as an additional client

The founder asks whether oxichrome would make a better client for sellers who prefer a browser extension to an installed program, and whether the extension is a surface worth having at all now that it ports to Chrome and Firefox.
Two questions are tangled there and they separate cleanly: oxichrome is a build tool, and the extension is a surface, and the answers run in opposite directions.

- date: 2026-09-03
- method: read-only inspection of this working tree, plus four commissioned research reports copied verbatim into `oxichrome-extension/` beside this file; no jj or git command was run, no build was started, and no marketplace was contacted
- evidence, copied verbatim and cited throughout as r1 to r4: [r1-anatomy.md](oxichrome-extension/r1-anatomy.md) on what oxichrome is, [r2-platform.md](oxichrome-extension/r2-platform.md) on the Chrome and Firefox platform including its F1 to F5 follow-up, [r3-fit.md](oxichrome-extension/r3-fit.md) on reuse of this repository's code, and [r4-comparables.md](oxichrome-extension/r4-comparables.md) on how cross-listers ship extensions
- corrects: `docs/notes/design/client-side-architecture.md` sections 7 and 9, which is the prior extension assessment, in four places — the `chrome.alarms` claim at `:167`, the `wasm-unsafe-eval` trap at `:171`, the `MaybeSend` sizing at `:203`, and section 7's promise that the adapters transpose one-to-one with no DOM puppetry
- decisions held fixed: D1 (the two-branch rule), D2 (Windows desktop first, extension deferred), D10 and D11 (the entitlement token and its kill switch)

## 1. The verdict

oxichrome is not usable as a dependency, and the reason is not maturity alone: its WebExtension surface is fourteen functions and five reachable events, it has no `alarms`, no `cookies`, no `webRequest`, no `offscreen` and no `declarativeNetRequest`, its generated service worker registers listeners after an `await` so a terminated worker cannot be woken by them, its message handlers return unit and therefore cannot answer a message asynchronously, and the two features we would need most reach only unreleased commits on a repository whose maintainer last committed on 2026-05-15 (r1).
A browser extension as a client surface is a different matter and it is structurally the purest expression of D1: the marketplace session never leaves the browser, nothing is captured into a jar we then have to guard, and every incumbent cross-lister in the reseller market has converged on exactly that shape for exactly the marketplaces that publish no API (r4, F1, F2 and F10).
It is nonetheless an additional surface rather than a better one, because it runs only while the browser is open, because the header envelope our TPT adapter is built out of is structurally unreachable from a `fetch()` in any extension context and no extension API closes that gap, because Firefox is unserved by every competitor and adds a second build and a second reviewer, and because store review sits between us and the shell of every update — so D2 stands, the desktop stays first, and the extension becomes a candidate for after the desktop runs live, with the gates in section 6 settled first.

Three places where the evidence moved against the framing this memo was commissioned under, and one where it moved in our favour.

The large upload is a smaller problem than it looked.
Chrome's own follow-up documentation says the lifetime of an offscreen document "is independent of the service worker that created it", and the thirty-second and five-minute rules are written against a service worker rather than against an extension (r2, F1(a)); an S3 multipart upload whose parts each answer inside thirty seconds does not trip the fetch rule in the first place (r2, F4).
The sharper residue is not the worker at all: it is that `FileSource::fetch` returns `FileContent { bytes: Vec<u8> }` by value with no streaming variant (`crates/tam-marketplace/src/lib.rs:713-733`), so a payload at `UPLOAD_BODY_BYTES_MAX` — 256 MiB, `crates/tam-limits/src/lib.rs:105` — is resident in one heap, and neither vendor documents a memory ceiling for an extension background context at all (r2 section 6).

The header envelope is a much larger problem than it looked, the instrument we expected to solve it is documented not to work, and the constraint turns out to be structural rather than a matter of permissions.
`declarativeNetRequest` requires host permissions for a request's initiator on every non-navigation request, and the initiator of the extension's own fetch is its own `chrome-extension://` or `moz-extension://` origin, for which an extension cannot hold a host permission — so dNR cannot match the extension's own requests at all, which makes the per-header question moot; Chrome states separately that extensions "can't change the `request origin` or initiator" (r2, F3 and F5(b)).
Our own competitive research records Vendoo shipping sixteen dNR rules that rewrite `Origin` and `Referer` (`docs/research/rethink/vendoo-architecture-and-market.md` section 1), so observed behaviour and published constraint disagree, and that disagreement is recorded here rather than resolved.
The addendum goes further and settles the question r3 could not: three of the four `sec-fetch-*` headers are fixed by what a `fetch()` structurally is — `Sec-Fetch-Dest` is `empty` because `document` is reserved for a user-initiated top-level navigation, `Sec-Fetch-Mode` is `cors` because `navigate` is not a settable request mode, and `Sec-Fetch-User` is omitted entirely rather than sent wrong — while the four headers an extension can rewrite are precisely the four that are not `sec-fetch` (r2, F5(a) and F5(b)).
So no extension context reproduces a navigation envelope, and a content-script fetch does not either, because a content-script `fetch()` is still a `fetch()` and carries the page's envelope rather than a navigation's.
The only shape for which the evidence says the headers come out right is navigating a real tab to the form and submitting from page context, which is a heavier architecture than the one-to-one adapter transposition `client-side-architecture.md` section 7 promised — it reintroduces exactly the DOM puppetry that note said the extension avoided — while being, notably, a closer reading of D1's own words than a background fetch ever was.
The unknown that decides which architecture applies is whether TPT's gate actually keys on the `sec-fetch` family or on the `user-agent` and `accept-language` pair, since those two are rewritable on requests dNR can match and would not force tab-driving (r2, F5(c)); that is what G1 exists to answer.

Store review gates less than the framing assumed.
MV3's remote-code rule permits an entitlement check and signed data while banning shipped adapter logic, which is the discipline this repository already committed to (`docs/notes/design/client-side-architecture.md` section 8), and Vendoo serves its marketplace logic rather than shipping it precisely so selector drift is fixed without a review cycle (`vendoo-architecture-and-market.md` section 1).
Review is a gate on the shell and a takedown lever; it is not a gate on every selector fix.

In our favour: the `wasm-unsafe-eval` trap that `client-side-architecture.md:171` marked unverified is now closed and it passes.
Our own generated glue at `web/src/lib/core/generated/core.js` contains zero occurrences of `new Function` and zero of `eval(`, instantiating through `WebAssembly.instantiateStreaming` with a plain fallback, so the grantable CSP token suffices on our own build, measured rather than reasoned (r3 section 6).

Two smaller corrections belong on the record.
`client-side-architecture.md:167` says an extension "cannot run cron at all, because `chrome.alarms` has a 30-second floor" — the floor never binds, because `scheduler.rs:119` sets `DEFAULT_CADENCE` to one hour; only the second half of that sentence, that alarms fire while the browser is open, is a real constraint (r3 section 7).
And the prior assessment lives at `docs/notes/design/client-side-architecture.md`, not under `docs/research/rethink/`.

## 2. What oxichrome is, and what to use instead

oxichrome is a build tool and a proc-macro framework that compiles one Rust `cdylib` to wasm and writes a complete MV3 extension directory around it; it is not a bindings crate and it drives no browser from outside (r1).
The whole binding layer is one 77-line file of hand-written `extern "C"` declarations at `oxichrome-core/src/js_bridge.rs:1-65`, and its complete surface is `runtime.getURL`, `sendMessage`, `onInstalled` and `onMessage`; `storage.local` get, set and remove plus `onChanged`; and `tabs.query`, `create`, `sendMessage`, `onUpdated` and `onActivated`.
Absent entirely are `alarms`, `cookies`, `webRequest`, `declarativeNetRequest`, `offscreen`, `notifications`, `identity`, `downloads`, `scripting`, `permissions`, `contextMenus`, `windows`, `action`, and both `storage.sync` and `storage.session` — which is to say all four of the APIs the device loop most needs.
The reachable event set is exactly five, because `#[oxichrome::on]` formats an identifier as `chrome_{namespace}_{event}_add_listener` and can only name a bridge function that already exists (`oxichrome-macros/src/codegen/event_handler.rs:29,79`).

Two defects sit in generated code rather than in ours, so neither is fixable without forking.
`background.js` registers every listener inside an `async function start()` after `await init()` (`oxichrome-build/src/shims.rs:35-44`), while Chrome requires MV3 service-worker listeners to be registered synchronously at top level or the events that should wake a terminated worker are not delivered — and waking a terminated worker on a timer is the entire scheduling mechanism.
Event closures return unit and are detached through `spawn_local` (`oxichrome-macros/src/codegen/event_handler.rs:73-80`), so an `onMessage` handler cannot `return true`, and asynchronous request-response messaging is not expressible at all.

The governance is the second half of the answer.
Nineteen commits from five authors, of which the maintainer's last is 2026-05-15 and the repository's last activity of any kind is 2026-07-28; 327 stars against 2 watchers; 1,304 total downloads across all five crates, of which the CLI every user must install has 169; 38 unit tests, none of them touching `oxichrome-core`, which is the entire runtime; and no CI, because no `.github` directory exists (r1).
The published crate is materially behind the repository: `v0.2.0` is the newest on crates.io and fourteen commits sit above it, including both content scripts and `host_permissions`, which are the two features we would need first — so using either means a git dependency pinned to an unreleased commit with no release cadence to return to.
Two further constraints would bind us immediately: only `src/lib.rs` is parsed for macro invocations (`oxichrome-cli/src/commands/build.rs:240-246`), so every declaration reaching the manifest lives in one file; and the generated manifest cannot express `icons`, `commands`, `declarative_net_request`, `minimum_chrome_version` or a merged user fragment at all (`oxichrome-build/src/manifest.rs:6-27`), while hardcoding `web_accessible_resources` to expose `wasm/*` to `<all_urls>` (`manifest.rs:118-121`).
The licence is clean — MIT, admitted directly by our `deny.toml:5` allow list — so vendoring would be legally straightforward; the question is whether we want to own roughly 2,700 lines of build tooling to obtain a manifest writer, and the answer is no.

The alternative is `web-extensions-sys` for the namespaces it covers plus our own manifest and loader shims on the `tam-core-wasm` pattern.
`web-extensions-sys` (MIT, examined at `16ff591`, 2025-10-12) covers `cookies`, `downloads`, `identity`, `scripting`, `port` and eleven more namespaces oxichrome lacks, but it too lacks `alarms`, `webRequest`, `declarativeNetRequest` and `offscreen` (r1), so three of the four APIs the loop needs are hand-declared whichever crate is chosen.
That is what makes the bindings-crate question minor: each missing API is the same six-line `extern "C"` block oxichrome writes internally, and the colour-picker example in oxichrome's own tree already demonstrates the unassisted path.
The pipeline it plugs into exists: `crates/tam-core-wasm` is 337 lines with `crate-type = ["cdylib", "rlib"]`, built by two lines of `justfile:228-232`, with `wasm-bindgen` pinned to exactly what nixpkgs carries (`flake.nix:190-194`), producing 449,880 bytes raw and 119,479 gzipped today (r3 section 6).
An extension build adds a second `cdylib` crate and a second `wasm-bindgen` invocation to that pipeline and nothing else structural, and the boundary discipline transfers directly — JSON strings in both directions, no `serde-wasm-bindgen`, and an error return rather than a panic, which matters more in a worker expected to be evicted and revived than it does on a page.

## 3. What an extension reuses, and what it must add

Seventy-six percent of the Rust an extension would link already compiles for `wasm32-unknown-unknown` and is proved to on every `just pre-push`; two mechanical edits raise that to eighty-nine percent (r3).
`justfile:63` names the seven crates proven on all five portable targets, and the eighth, `tam-engine-driver` at 3,569 lines, is blocked by a 278-line crate rather than by anything in itself: `crates/tam-limits/src/lib.rs:206` is a const-evaluated `assert!(usize::BITS >= 64, "byte bounds are u64 and are converted to usize at the axum boundary")`, and it fails there before the driver's own code is reached.
The honest fix is a `cfg(not(target_family = "wasm"))` on that one assertion, because the axum boundary the assertion names does not exist on a client — still a founder call on a gated file, but a small and well-argued one, and `justfile:70-71` already books the crate to join the portable list for an unrelated reason.

The `Send` bound is the real fight and it is narrower than "wasm futures are not `Send`".
`wasm-bindgen` 0.2.121 unsafely implements both marker traits for `JsValue` on a single-threaded build, so a struct holding `web_sys` handles satisfies the `Send + Sync` supertrait; the future is what fails, because `JsFuture` holds an `Rc<RefCell<_>>` (r3 section 2).
r3 confirmed this by compiling the TPT adapter's `live` feature against `wasm32` and getting the exact bound named in the error: `required by a bound in Transport::send`, which is `crates/tam-marketplace/src/transport.rs:293-297`, verified in the tree.
Ten traits carry a `Send + Sync` supertrait and thirty-two returned futures carry a `+ Send` bound, across four files.
Two ways out with very different blast radius: the `MaybeSend` cfg shim that `client-side-architecture.md:203` booked as migration step 1 has never been written, and its true edit surface is forty-two sites in crates every server binary compiles, not the "roughly 20 lines" that note estimated; or a browser `Transport` whose `send` spawns the `!Send` future on `spawn_local` and returns a `oneshot::Receiver`, which is `Send` when its payload is, so the existing bound is satisfied and the `!Send` part never crosses it — about forty lines per seam, touching no shared crate.

Three pieces of luck are worth naming because each could have been a wall.
There is no Rust HTML parser anywhere in the workspace, because `crates/tam-marketplace-tpt/src/form.rs:6-11` records that the scrape is anchored-substring work rather than a parser, so 712 lines port with no dependency to drag — and a `scraper` edge would have pulled in `cssparser`, one of the MPL-2.0 crates that already needed a licence exception.
Nothing in the adapters or the driver touches a filesystem or a wall clock, enforced rather than remembered: `guard.rs:16-19` bans `std::fs::read_to_string` crate-wide, and `just purity` fails the build if `tokio`, `reqwest` or `sqlx` reaches the driver's normal dependency graph.
And `blake3` runs under its `pure` feature by founder decision on 2026-09-03, which drops the `cc` and `ml64.exe` paths there is no way to cross.

Against the desktop's measured 4,176 production lines, the extension's device loop is about 3,330 Rust lines plus about 530 lines of irreducible JavaScript — roughly twenty percent smaller, because the browser holds the session the desktop had to capture and store (r3 sections 4 and the table).
`ledger.rs` (460), `work.rs` (336), `heartbeat.rs` (341) and `entitlement.rs` (207) port essentially unchanged.
`session/` (318 lines), `console_session.rs` and `webview_session.rs` (130) delete outright, and only TPT's double-submit CSRF token survives, read through `chrome.cookies` in about forty lines of JavaScript.
`run.rs` (238) is rewritten entirely because `SystemTime::now()` has no source on `wasm32-unknown-unknown` and the pause and deadline are `tokio::time`; `scheduler.rs` keeps its refusal logic in Rust and moves only its timer; `marketplace.rs` becomes the new fetch transport and must re-implement the origin binding at `:96-98`, which in a browser is also what keeps `host_permissions` honest.
The 530 JavaScript lines are the alarms timer, the cookie read, the message surface, the manifest and the redirect-header listener.

The server needs no migration to accept an extension.
`crates/tam-storage/migrations/0042_device_registry.sql` has no surface, kind or platform column, and `crates/tam-api/src/devices.rs:76-82` takes `RegisterBody { name, os, arch, app_version }` as free strings, so an extension registers today as `os: "chrome"`, `arch: "wasm32"` (r3 section 5).
Authentication is a plain `tam_session` cookie read out of the `Cookie` header and nothing else, which an extension holding host permissions for our origin sends automatically on a `credentials: "include"` fetch and never reads — strictly better custody than the desktop's route of reading it out of a webview cookie store.
Two things do change: the entitlement token fixes `AUDIENCE = "tam-desktop"` in code deliberately, so an extension is a second audience the server must mint for; and whether a seller's "Your devices" page should distinguish a laptop from a browser profile is a small closed-vocabulary migration and a founder call, not a blocker.

## 4. The platform constraints that shape the design

Chrome terminates an MV3 service worker when a `fetch()` response takes more than thirty seconds to arrive, when one event or API call takes more than five minutes to process, and after thirty seconds idle; none of the documented keep-alive tricks lifts the per-fetch rule, they only reset the idle timer (r2 section 1).
Read per request, that is survivable: an S3 multipart upload whose parts each answer inside thirty seconds does not trip the fetch rule, and at TPT's 5 MiB part size a 256 MiB payload is roughly fifty-two parts needing about 1.4 Mbps sustained upstream (r2, F4, with `crates/tam-marketplace-tpt/src/s3.rs:25`).
The trap is the other rule, which is scoped to the event dispatch rather than to the fetch: one `alarms.onAlarm` handler looping over fifty parts is a single request and dies at five minutes however fast each part completes, and driving parts across separate alarm dispatches collides with the thirty-second alarm floor.

That is what argues for a host other than the worker.
An offscreen document's lifetime is stated to be "independent of the service worker that created it", only `AUDIO_PLAYBACK` carries a lifetime limit, and messages from it reset the worker's idle timers, so it is a lifetime tool as well as a DOM tool (r2, F1(a)).
Its costs are exact: one per installed extension, `chrome.runtime` is the only extensions API available inside it, Chrome has reserved in writing the right to add per-reason lifetime restrictions, and no source addresses closure under memory pressure.
An extension page in a tab is the alternative — created without a permission or a gesture, subject to no documented idle rule — but it is a prime discard candidate precisely because it is inactive, and `autoDiscardable: false` is a strong hint rather than a guarantee, at the product cost of a visible tab (r2, F1(b)).
Firefox is not the safe harbour: its event page is killed at thirty seconds by deliberate design, active ports do not prevent it, and only a promise returned from a WebExtension API listener buys one bounded extension whose value is unpublished (r2, F2).

Cookies are the part that works, and it is the reason the surface is attractive at all.
Chrome treats requests from an extension to a third party as same-site where the extension holds host permissions for it, so `SameSite=Strict` cookies ride along — qualified twice, that it covers network requests rather than `document.cookie`, and that it does not apply where third-party cookies are blocked (r2 section 3).
The same rule is written against the extension rather than against a context, so an offscreen document and a runner tab get identical treatment (r2, F1(c)).
`credentials` defaults to `same-origin` and every marketplace fetch must set `include` explicitly.
Third-party-cookie blocking is a live product risk rather than a footnote: a seller who has blocked them loses the whole mechanism, and there is no fallback that keeps the session in the browser.

The header envelope is the open question that decides whether the TPT write path works from an extension at all.
`crates/tam-marketplace-tpt/src/live.rs:18-24` records that the same document navigation was answered with a Cloudflare interstitial under a truncated user agent and with the form under a real browser's headers, and the per-hop envelope built at `:47-67` and `:109-130` includes seven names a browser forbids `fetch` to set: `user-agent`, `accept-language`, the four `sec-fetch-*`, and `origin`.
In an extension the browser supplies them, which is better for detectability and worse for fidelity, because an extension-origin fetch sends `Origin: chrome-extension://<id>` and a `Sec-Fetch-Site` that is not the `same-origin` our adapter hardcodes at `live.rs:57-62`.
The `sec-fetch` half is not recoverable by any rewrite, and the reason is structural: `Sec-Fetch-Dest` is `empty` for a `fetch()` because `document` is reserved for a user-initiated top-level navigation, `Sec-Fetch-Mode` is `cors` because `navigate` is not a settable request mode, and `Sec-Fetch-User` is omitted entirely rather than sent with a wrong value, so a gate looking for `?1` sees nothing (r2, F5(a)).
Only `Sec-Fetch-Site` varies and the browsers disagree on it: Chrome sends `none`, while Firefox sends `same-origin` where the extension has host access, having shipped `cross-site` in Firefox 90, broken real sites, and fixed it in 92.
The `Sec-` prefix is a forbidden-request-header category, no `sec-fetch` header appears on any `webRequest` modifiable list while `user-agent`, `origin`, `referer` and `accept-language` all do, and `declarativeNetRequest` cannot match the extension's own requests to try in the first place (r2, F5(b)) — so the published position rewrites exactly the wrong half of the envelope.
The one shape the evidence says produces the right headers is a real navigation in a real tab, because the browser is then genuinely doing what those headers describe; a content-script fetch carries the page's origin, which answers `Origin`, but its envelope is still a fetch's rather than a navigation's, and it lost host-permission CORS privileges in Chrome 73 and Firefox 101 so it succeeds only where the marketplace's own front end would (r2, F3 and F5(c)).
Firefox adds its own wrinkle on the `Origin` half: its background fetch sends `Origin: moz-extension://<uuid>`, a stable per-installation identifier that is a ready-made detection signal, and Mozilla has already tried suppressing it and backed the change out (r2, F3).

The 302 `Location` read stays the classification contract and stays a JavaScript problem, but it is no longer an open one.
`crates/tam-marketplace-tpt/src/classify.rs:488-500` says a 302 whose `Location` yields a product id is the only shape any capture contains and the only shape read as a landing, and everything else is `NoDurableIdentifier`; a browser's `redirect: "manual"` resolves to an opaque response with no headers, so `chrome.webRequest.onHeadersReceived` is the route to that header (r3).
The addendum confirms it works on paper: non-blocking `webRequest` survives MV3 in full — "aside from `webRequestBlocking`, the webRequest API is unchanged and available for normal use" — `onHeadersReceived` exposes `responseHeaders`, `Location` is on no restricted list (only `Set-Cookie` and `X-Frame-Options` need `extraHeaders`), and the extension's own asynchronous fetches are not among the requests hidden from its own listeners (r2, F5(b)).
The cost is the `webRequest` permission, which Chrome names among those that lengthen review.
Tes carries no such dependency, so this is a TPT-create problem rather than a general one.

Firefox is a second product rather than a second target.
It does not support `background.service_worker` at all and the tracking bug is open with no milestone, so it runs an event page; one manifest may carry both keys and current versions of both browsers tolerate that, which is the cheap way to keep Firefox reachable without serving it (r2 section 1).
It treats `host_permissions` as optional and user-revocable ad hoc, and permissions added by an update are still not shown to the user, so an extension must call `permissions.contains` then `permissions.request` at runtime rather than assume the grant.

Two things are settled and favourable.
`wasm-unsafe-eval` is required by both browsers and grantable by both, `object-src` is required by Chrome and optional in Firefox from 106, and one manifest supplying both satisfies both; package size caps are 2 GB and 200 MB, nowhere near a Rust wasm binary (r2 section 4).
Ed25519 is a first-class Web Crypto algorithm shipped in Chrome 137 and Firefox 129, so the D10 entitlement token verifies in the background context without wasm, with wasm remaining the portability floor.

Distribution is where the extension gives something up that the desktop keeps.
Chrome review runs "within a few days, but it can take up to a few weeks" and explicitly longer for extensions requesting broad host permissions, which this one has by construction; obfuscation is banned while minification is allowed, and the code-readability policy does not mention WebAssembly or compiled binaries at all, so whether our wasm blob reads as concealment is undetermined and rests on reviewer discretion (r2 section 8).
Mozilla requires a reviewable source package with build instructions, environment, tool versions, the full command list and lockfiles, sufficient to rebuild the extension — real per-release work, and non-compliance risks takedown.
Auto-update makes the propagation channel slow in a way that matters: Chrome installs an update only when the extension is idle, which an extension running a scheduled work loop may not be until the browser restarts, and Firefox checks once a day after up to 24 hours of signing latency (r2 section 9).
The consequence is that the kill switch must never depend on shipping a version, which is already the architecture: the server withholds work orders and the D11 token fails closed on expiry, one hour plus a 24-hour grace.

Those constraints compose into one design rather than leaving a choice open, and it is worth stating as the thing the gates test.
The background context — service worker on Chrome, event page on Firefox — becomes the scheduler and nothing else, because it is the only context `chrome.alarms` can wake and the one context that cannot issue a usable marketplace request.
The driver runs in an offscreen document on Chrome, whose lifetime is documented independent of the worker, so eviction of the scheduler does not abort a run in progress; on Firefox there is no offscreen API and the event page is force-stopped at thirty seconds regardless of in-flight fetches, so the marketplace tab has to be the long-running host as well, which is the sharpest structural divergence between the two browsers in this whole assessment.
The content script in a tab on the marketplace's origin is the transport shim: it executes each request the driver describes, carrying the marketplace's own origin and the browser's own `user-agent`, `accept-language` and `sec-fetch-*` — which is to say exactly what the marketplace's own front end sends.
That dissolves r3's first obstacle, the seven forbidden headers, rather than solving it: we stop trying to reproduce a browser's envelope because the browser is now genuinely producing it.
The S3 part uploads go the same way, cross-origin from the marketplace page exactly as the marketplace's own uploader issues them, which is consistent with the desktop's own rule that a session-authenticated request may reach exactly one host while the signed upload is exempt because it signs its own body (`apps/desktop/src-tauri/src/marketplace.rs:20`).
The create form is reached by a real navigation of that tab rather than by an XHR, so the page arrives under a navigation envelope, and the content script reads it from there.

Two details of that shim are worth fixing now because they decide whether it is self-sufficient.
The request must be issued from the content script's isolated world rather than from page context, because the isolated world keeps the CSRF token and any header we set unreadable by the page, and a content script may set `x-csrf-token` freely since it is not a forbidden header.
Whether the shim can obtain that token by itself is genuinely unknown: `crates/tam-marketplace-tpt/src/session.rs:5-8` records that TPT's CSRF is a classic double submit, the `csrfToken` cookie value copied verbatim into `x-csrf-token`, and that the token "appears nowhere in the page markup, so a connector obtains it only by reading its own jar" — and `docs/notes/design/desktop-client.md:57` establishes that the *session* cookie is HttpOnly on both marketplaces without saying anything about `csrfToken`.
If `csrfToken` is readable from `document.cookie` the shim reads it in place and needs nothing from the background; if it is HttpOnly the token must come from `chrome.cookies.get` in the background context and be messaged in on every request, which adds a hop and puts a credential-shaped value on the message bus.
That is one line of the G1 probe to settle, and it is worth settling there rather than discovering it during a build.

## 5. What the market does

Every incumbent cross-lister ships the same architecture, and it is the one D1 already commits us to: the server holds the catalogue, the inventory, the analytics and the subscription, and the extension is the actuator that touches the marketplace (r4, F1).
Not one vendor holds a marketplace credential and every one says so on the record — Crosslist, PrimeLister, List Perfectly, Closo and Vendoo each state in their own documentation that they never ask for a marketplace password and that the seller logs in themselves (r4, F2).
The extension is mandatory exactly where the marketplace is unsanctioned and optional everywhere else: Crosslist needs it only "when an API isn't available", Closo needs it for Poshmark, Mercari, Depop and Vinted but not for OAuth-connected eBay and Shopify (r4, F10) — which is D1's two-branch line, drawn independently by the market.
Host permissions are an enumerated marketplace allowlist in every case examined and never a wildcard, roughly 25 named domains for Crosslist, and no extension examined declares `<all_urls>` (r4, F3).
Google accepts the category: both Crosslist BV and PrimeLister carry the store's "no history of violations" badge, one search returns ten live cross-listers with more behind a "Load more" control, and no public record was found of a cross-lister removed for policy violation or of a marketplace forcing one out (r4, F4 and F5).

Two findings cut the other way.
Firefox is empty — searches of addons.mozilla.org return no product from Vendoo, List Perfectly, Crosslist, PrimeLister, Flyp, SellerAider, Closo or Nifty, and List Perfectly documents its boundary as Chrome and Edge on desktop only (r4, F9).
And the platform rather than the marketplaces is what has actually forced this category to change: PrimeLister rebuilt against a Chrome deadline and used the occasion to move its Poshmark automation entirely server-side while keeping crosslisting in the extension, which is the same split D1 draws and the one durable pattern in the set (r4, F8).

In our own market there is nothing to copy and nothing hostile to defend against.
No tool was found anywhere that posts to TeachersPayTeachers or Tes on a seller's behalf; the ten TPT extensions in the store all read, score or scrape, and the search for Tes returns literally nothing (r4, education market).
That is a positive search result rather than an absence of searching, and it means no incumbent has established either a precedent or a reaction.

## 6. Kill gates before any build

Four bounded spikes, each with a pass condition that would actually fail under a plausible negative answer.
G1 to G3 run against an unpacked extension loaded locally and touch no store; G4 is the only one that does, and it is placed first in wall-clock because its latency is external.

G1, the header envelope against a live TPT, and the gate the whole question turns on.
A founder-gated live action on the founder's own machine and account, modelled on `tools/login-probe`, which gates Phase 2 today.
Which header TPT's gate actually keys on cannot be recovered from our captures — `crates/tam-marketplace-tpt/src/live.rs:18-24` records that the envelope as a whole is the difference between the form and an interstitial, not which header carries it — so the probe tries both shapes rather than assuming the expensive one.
The cheap shape is a background-context `fetch()` with the four rewritable headers set, `user-agent`, `accept-language`, `origin` and `referer`, accepting that the `sec-fetch` family will be wrong by construction.
The expensive shape is the design in section 4: navigate a tab to the create form, let it arrive under a navigation envelope, and issue the first same-origin request from a content script in that tab.
Pass if the form is served rather than a Cloudflare interstitial and the CSRF pair is accepted, and the cheapest shape that passes is the design.
Record two things beyond pass or fail, because both are free once the probe is standing: what each shape actually sent, and whether `document.cookie` in the TPT tab exposes `csrfToken`, which decides whether the transport shim is self-sufficient or needs a token messaged in per request.
If the cheap shape passes, the adapters transpose close to one-to-one and the extension is genuinely cheap; if only the tab-navigation shape passes, the surface is a DOM-driving client and should be re-decided rather than absorbed.
Failing both ends the extension question outright, and that is the point of running it before anything is written.

G2, the 302 `Location` through `webRequest`, now a confirmation rather than an unknown.
Register `chrome.webRequest.onHeadersReceived` without `webRequestBlocking`, issue a request from the extension that is answered with a redirect, and read the `Location` header back.
Pass if the header arrives and can be joined to the request that produced it, which is what `classify_submit` needs.
F5(b) answers every sub-question this gate was originally posed to settle — non-blocking `webRequest` is intact in MV3, `responseHeaders` is exposed, `Location` needs no `extraHeaders`, and the extension's own async fetches are observable — so this is now an hour of confirmation against our own host rather than a research question, and it need not wait on G1 or touch a marketplace.

G3, the payload path at the cap.
Drive a 256 MiB multipart upload at the 5 MiB part size from an offscreen document, and measure three things rather than one: whether the offscreen document survives the whole upload, whether the service worker survives or is revived cleanly mid-upload, and whether 256 MiB can be held resident at all given that `FileSource` returns bytes by value and neither vendor documents a memory ceiling.
Pass if the upload completes and the process is repeatable across a browser restart.
If the offscreen document is closed under memory pressure — the case r2 could not answer from any source — the runner-tab variant with `autoDiscardable: false` is the fallback to measure in the same spike, and if neither holds, `FileSource` needs a streaming variant and that is a change to a shared trait rather than to the extension.

G4, the store's view of a wasm blob.
Submit a minimal functional wasm-bearing extension — not the product — as unlisted to the Chrome Web Store and through AMO unlisted signing, and record whether the compiled artefact draws a code-readability objection and what Mozilla's source-code submission actually demands in practice.
Pass if both accept it without an objection specific to the compiled binary.
This gate is added because r2 section 8 finds Chrome's readability policy silent on WebAssembly, which makes it reviewer discretion rather than a published rule, and it is the single item in the whole assessment that cannot be measured without submitting something.
It is cheap, it is independent of G1 to G3, and its latency is days to weeks, so it should start first if the extension is taken up at all.

## 7. Effort

Week counts are refused here, because the estimates in the rethink memo were built on a conversion rate rather than a measurement and the founder has said so.
The one measured yardstick is this session's own: the desktop scaffold plus its data plane, `apps/desktop/src-tauri` at 4,176 production lines by r3's count, landed between 03:18 and 11:35 on 2026-09-03 in agent time — a figure taken from the orchestrator's record of this session rather than from anything in the tree, since no version-control command was run for this memo.

Against that yardstick the code is roughly one unit.
r3 sizes the extension's device loop at about 3,330 Rust lines plus about 530 lines of JavaScript, which is eighty percent of the desktop's production count and close to ninety with the JavaScript, and the `Send` bridge adds about forty lines per seam on the cheaper route.
So the writing of it is one yardstick, give or take.

Everything that is not writing is where the estimate actually lives, and three unknowns can move it by a multiple rather than a margin.

The first is the transport, and it is the largest.
The desktop's 4,176 lines were written against a reqwest transport that already existed, already compiled and had already been proven live against both marketplaces; the extension's transport is precisely the thing G1 exists to test, and G1's two shapes carry very different costs.
If the cheap shape passes, `marketplace.rs`'s successor is r3's estimated ~250 lines, the adapters are untouched, and the yardstick holds.
If only the tab-navigation shape passes, the design is no longer a transport swap at all: it is navigate a tab, wait for load, read the form through a content script and issue every request from that tab's isolated world, with a message hop per request and the driver hosted in an offscreen document on Chrome and in the tab itself on Firefox.
That discards the plain-HTTP adapter transposition which is what makes this surface cheap, and reintroduces the DOM coupling we have been avoiding.
It is not knowable from here and should be re-estimated when G1 answers rather than guessed at now, which is the error the founder already called out.

Two costs belong in the comparison whichever shape wins, because they are properties of the surface rather than of the transport.
A marketplace tab is visible in the seller's tab strip while work runs, which is a product decision rather than a platform limit, and that tab is a prime discard candidate precisely because it is inactive — `autoDiscardable: false` is the documented mitigation and r2 is explicit that it is a strong hint rather than a guarantee under severe memory pressure.
Against that, the desktop keeps two advantages the extension cannot acquire: it runs with the browser closed, which is the whole of the deterministic-schedule non-negotiable, and it composes requests through a native transport whose envelope we control completely rather than one the browser controls for us.

The second is that the gates are wall-clock rather than agent time.
G1 needs the founder at their own machine on their own account, as `tools/login-probe` does today; G4 is days to weeks of someone else's review queue.
Neither compresses, and neither is work we can do faster by doing more of it.

The third is the release matrix.
Chrome and Edge share one artefact; Firefox adds a second build, a second store, a per-release reviewable source package with rebuild instructions and lockfiles, and its own event-page lifetime to design against.
That is not a proportional addition to a one-yardstick build.

Stated as a decision aid rather than a forecast: if the gates pass on the first route tried, this is a one-to-two-yardstick build plus external store latency, and it is genuinely cheaper than the desktop was because the browser holds the session and 89 percent of the Rust already compiles.
If G1 forces the content-script inversion, it is a new design and should be re-estimated then rather than now.

## 8. Open items, and the decisions to take

The research could not settle the following, recorded as the reports left them.

From r2, unanswerable from primary sources: what `Origin` and `Referer` a marketplace sees from an MV3 background-context fetch, and whether `declarativeNetRequest` `modifyHeaders` can `set` or `remove` them; whether Firefox supports `ReadableStream` request bodies with `fetch`, and under what transport conditions; any documented memory ceiling for a Chrome extension service worker or a Firefox event page; Firefox's minimum honoured `alarms` period, which MDN attributes only to Chrome; whether a Rust-compiled wasm binary satisfies Chrome's code-readability policy, which does not mention compiled code; and whether either store has an unpublished or case-law position on automating a user's actions against a third-party site's terms.

From r2's follow-up, still open: whether Chrome closes an offscreen document under memory pressure, and whether `autoDiscardable: false` is honoured under severe pressure; the value of Firefox's "extended timeout" before a background context with pending listener promises is forcibly terminated; what `Referer` a background-context fetch sends in either browser; empirical confirmation of the `Origin` value Chrome sends on a POST from worker, offscreen document and extension tab, which no primary source states; and whether `declarativeNetRequest` `set`/`remove` on `Origin` or `Referer` is silently rejected as well as being blocked by the initiator check.

From r2's F5 addendum, one unknown, and it is the one that decides the architecture: whether TPT's gate actually keys on the `sec-fetch` family or on the `user-agent` and `accept-language` pair, since the latter two are rewritable on requests dNR can match and would not force the tab-driving architecture.
Two of its findings are medium confidence and rest on tracker comments rather than vendor documentation, and both should be treated as provisional: that Chrome sends `Sec-Fetch-Site: none` from a WebExtension `fetch()`, and that `onBeforeRedirect` does not fire under `redirect: "manual"`.

From r1, two measurements blocked by a permission layer that declined to compile an externally cloned repository: oxichrome's bundle size for its smallest example, and its transitive licence graph under `cargo deny check licenses`.
Both need one founder instruction authorising a build inside `~/ghq/github.com/0xsouravm/oxichrome`, and neither changes this memo's verdict, which rests on API coverage and governance rather than on size or licence.

From r3, three items, of which two remain open and one is now answered.
Open: whether the founder will `cfg` off the `usize::BITS` assertion at `crates/tam-limits/src/lib.rs:206`, and which `Send` route to take, since the shim edits founder-gated shared surface that every server binary compiles — both are decisions 3 and 4 below rather than research gaps.
Answered: r3 named "whether DNR can also rewrite `Sec-Fetch-*`" as the single thing it could not settle and the thing deciding whether the TPT write path works from an extension at all; r2's F5 settles it negatively on two independent grounds, that `Sec-` headers are a forbidden-request-header category absent from every `webRequest` modifiable list, and that dNR cannot match the extension's own requests at all.
The question that replaces it is not whether we can forge the envelope but whether TPT requires it, which is F5(c)'s unknown and G1's purpose.
Three smaller items ride along: whether the deployed `tam_session` cookie's `SameSite` attribute — set by the serving deployment rather than fixed in this repository — rides an extension-origin fetch; that `ring`'s wasm path is `check`-clean but not run-proven; and whether TPT's `csrfToken` cookie is HttpOnly, which this repository records for the session cookie but not for the CSRF one, and which decides whether the content-script transport shim is self-sufficient or needs the token messaged in on every request.

Four decisions follow.
Recommendations 1, 2 and 4 stand unless the founder says otherwise; decision 3 edits the founder-gated `crates/tam-limits` and so is taken only on the founder's explicit words, never by silence.

1. Does the extension enter the plan, and where?
   Recommendation: yes, as a candidate rather than a commitment, sequenced after Phase 2's desktop has run a live create and publish read back through the authoritative API, and gated on G1 to G3 passing, with G4 started earlier because its latency is external.
   Nothing about it is written before the gates answer, on the same discipline that puts the login probe in front of Phase 2.
   One qualification, and it is the substance of the recommendation rather than a caveat: G1 tries the cheap shape and the tab-navigation shape, and the cheapest that passes is the design.
   A pass only on the tab-navigation shape is not a green light but a second decision, because the surface it buys is a DOM-driving client with a visible marketplace tab rather than the adapter transposition that makes the extension cheap.
   Two of the gates are founder actions and start only on the founder's word: G1 is a live request against TPT on the founder's own account and machine, and G4 is a store submission.

2. Chrome and Edge only at first, or Firefox from the start?
   Recommendation: Chrome and Edge only, with the manifest carrying both `background.service_worker` and `background.scripts` so Firefox stays reachable at no cost, since current versions of both browsers tolerate the dual key.
   Firefox is unserved by every competitor in the category, and taking it on adds a second build target, a second reviewer and a per-release source-submission obligation for a market no incumbent has found worth serving.

3. The `tam-limits` assertion for wasm.
   Recommendation: take the `cfg(not(target_family = "wasm"))` on the `usize::BITS` assertion at `crates/tam-limits/src/lib.rs:206`, rather than deleting or widening it.
   The assertion's own stated reason is a conversion at the axum boundary, and no client has an axum boundary, so the `cfg` narrows the claim to where it is true instead of relaxing a gate to make a build pass — which is the distinction the enforcement rule in `CLAUDE.md` draws.

4. The `Send` route.
   Recommendation: the `oneshot` bridge first, and the `MaybeSend` shim only if a second seam needs it.
   The bridge is about forty lines per seam and touches no shared crate; the shim is forty-two edit sites across four files that every server binary compiles, and it is the cleaner end state rather than the cheaper first move.
   Taking the bridge first also keeps the extension spike from putting a change into founder-gated shared surface before the gates have said the extension is real.
