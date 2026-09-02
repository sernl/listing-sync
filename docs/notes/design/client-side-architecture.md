# A client-side architecture for cross-listing

How the marketplace request comes to originate on the seller's machine, in Rust, cross-platform, with a subscription we still control.

- date: 2026-08-31
- method: four parallel read-only research streams (repository stack-fit, browser extensions, control-plane split, Rust desktop), no marketplace contacted, no code written
- status: design input to a founder decision, nothing built; the companion legal research is `docs/notes/legal/marketplace-terms-assessment.md`

## 1. Purpose, and what it inherits from the legal memo

The legal memo settled what has to change and left open how.
Its architecture finding is that the decisive issue is not permission but architecture: every lawful comparable in this market operates either through an official API or client-side on the user's own device, and none operates server-side holding the user's credentials, which is exactly our model.
The binding constraint it derives from *Amazon.com Services LLC v. Perplexity AI, Inc.*, No. 26-1444 (9th Cir. Aug. 4, 2026) is about **request origin**, not credential location: Perplexity won because "Perplexity's servers never directly access Amazon's servers", so "It is the user who 'accesses' Amazon's computers, with the help of the Assistant" (https://cdn.ca9.uscourts.gov/datastore/opinions/2026/08/04/26-1444.pdf).
Our servers indisputably access TES's and TPT's, which concedes the prong Perplexity won on and leaves only the authorization question that Power Ventures loses the day a cease-and-desist arrives.
Storing the cookie on the seller's disk while our server still opens the socket buys nothing.
This note answers the remaining question: how the request itself moves to the seller's machine, in Rust, on Windows, macOS and Linux, while we keep orchestration, mapping, the dashboard, the subscription and the kill switch.
Claims about this tree cite file and line; claims about the outside world carry the URL the research stream retrieved, and every unverified or empirically-open item is marked as such rather than smoothed over.

## 2. Executive recommendation

Build a native Rust desktop client on Tauri v2, and keep the browser extension reachable as a later addition rather than a fork.
The embedded webview is used for **login only**: the seller authenticates to TES or TPT visually in a webview we own, and we never touch their browser profile.
Rust then reads the cookies out of that webview's own store and the existing adapters run in-process on the seller's machine against `ReqwestTransport` unchanged, so the bytes on the wire are the bytes we captured and live-fired.
The server becomes a control plane: orchestration, mapping decisions, the ledger, the dashboard, entitlement and the kill switch.
It sends **declarative intent** — what outcome is wanted — and never composes or signs a marketplace request.
The scheduler moves to the client, which preserves the deterministic, cron-scheduled non-negotiable because only the location of the timer changes, and it removes the single largest control risk in the legal analysis.
One short-lived Ed25519 entitlement artifact carries both the subscription state and a per-marketplace grant set, so the subscription check and the per-platform cease-and-desist kill switch are the same code path.
Session custody dissolves: roughly 4,575 lines of vault, gateway, lease protocol and key-encryption machinery delete outright, and the four database roles collapse to about two.
Three of the four research streams reached this independently — the repository stack-fit stream's architecture (b), the desktop stream's option O1, and the control-plane stream's O2/O3 are the same design arrived at from portability, form factor and legal posture respectively.
The cost is real and is paid in packaging rather than in code: a five-job CI matrix, an unavoidable macOS runner, roughly $220–600 a year in signing, a permanently fragile Linux WebKitGTK target, install friction, and a product whose freshness depends on the seller's machine being on.
One gap can falsify the whole recommendation: whether the TES and TPT **login pages** carry a Cloudflare or DataDome challenge that an embedded webview cannot clear.
That needs a live read-only probe of the founder's own login pages and it is founder-gated; nothing else in this note should be committed to before it is answered.

## 3. Why client-side at all

The legal memo's ranked mitigation M1 is client-side execution, and it is described there as the structural fix rather than a mitigation, because it changes which question is litigated rather than improving the answer to the existing one.
Everything else on that list — the tested kill switch, never circumventing a technical control, honest self-identification, pursuing official API status, not marketing against the terms — improves our position inside the authorization analysis.
Only moving the request origin gets us out of it, onto the access prong where Perplexity actually won.
The memo is equally clear that this buys nothing against tortious interference, which turns on knowledge of the terms and intent to induce a breach, and which the Ninth Circuit expressly reserved.
So the case for this architecture is narrow and should be stated narrowly: it converts the strongest claim against us from one we lose on revocation into one we have a published appellate answer to, and it leaves the tort exposure exactly where it was.

## 4. The reuse finding: the engine already has the seam

The repository is already structured for this, which is the finding that makes the migration weeks rather than a rewrite.
A sans-io transport seam exists and is the single network door: `pub trait Transport: Send + Sync` at `crates/tam-marketplace/src/transport.rs:293`.
Both sides of that door already serialise — `HttpRequest` at `:83` and `HttpResponse` at `:162` derive `Serialize` and `Deserialize`, because they already cross a process boundary as JSON for the cassette fixtures.
The trait has three implementations today: `ReqwestTransport` (`crates/tam-marketplace-tpt/src/live.rs:515`, `crates/tam-marketplace-tes/src/live.rs:153`), `GatewayTransport`, which proxies through the broker's loopback lease gateway with the cookie injected server-side (`:568` and `:291` respectively), and `CassetteTransport` (`crates/tam-marketplace/src/cassette.rs:52`).
A client-side transport is a fourth implementation of a trait already implemented three times.

The adapters are generic over the seam rather than over reqwest — `TptAdapter<T, F, P>` and `TesAdapter<T, F>`, where every I/O is a trait parameter (transport, file source, pause).
The portability is deliberate and was maintained: there is no clock, time enters as data, `project_fields` is non-async and pure, and ambient I/O (`std::fs`, `std::net`, `std::thread`, `SystemTime`, `Instant::now`) appears only under `#[cfg(test)]`.
`reqwest` appears in the TPT flows module exactly once, in a comment.

The stack-fit stream's ratio is roughly 875 lines of transport-coupled code against roughly 11,268 lines of portable marketplace logic — about 13:1 reuse.
The 875 reconciles against the tree: the TES transport is `crates/tam-marketplace-tes/src/live.rs:1-300` and the TPT transport is `crates/tam-marketplace-tpt/src/live.rs:1-573`, with the test modules beginning at `:301` and `:574`, so the figure counts production transport code only.
The pure core ships client-side unchanged: `tam-limits` (278), `tam-types` (1,544), `tam-marketplace` (1,545, the seam itself), `tam-domain` (5,887), `tam-taxonomy` (5,772).
`tam-pipeline` (1,143) is portable but undeclared as such — blake3, image and zip work are pure, and only `store.rs`'s `LocalObjectStore` touches the filesystem, which is separable.
The adapters that move are `tam-marketplace-tes` (4,189) and `tam-marketplace-tpt` (8,666), neither of which depends on tokio or sqlx.
That is 12,855 lines encoding everything learned from the HAR captures, already live-proven — the TPT write path was live-fired and survived a 35-agent review — and in a native process it compiles and runs unchanged, envelope byte-identical to what was captured.

The server-bound control plane stays where it is: `tam-storage`, `tam-api`, `tam-engine`, `tam-worker`, `tam-admin`, `tam-server` and the SvelteKit dashboard.
Corrected 2026-09-03: `tam-sync-worker`, `tam-analytics`, `tam-canary` and `tam-import` were listed here too, which contradicts D1, because each of the four originates a scheduled request to a no-API marketplace under a seller session from a server timer (`docs/notes/design/engine-driver-split.md` section 5, findings 13 and 14).
All four move to the device: the analytics capture becomes a device-pulled read item, `tam-sync-worker`'s Tes read leg moves and the crate keeps its enqueue half, `tam-canary` becomes a founder-run desktop command, and `tam-import` is restricted to manifest bytes or routed the same way.
`tam-worker` stays for what never leaves the server, the cross-tenant lease scan and the reaper, while the driver it hosts today moves with the interpreter.
The real work is not the transport at all.
It is splitting `crates/tam-engine/src/driver.rs` (1,581 lines) so the effect interpreter runs against a remote ledger behind a repository trait instead of against concrete `tam-storage` repositories.

## 5. The custody dissolution, and the one real loss

If the session lives on the seller's machine, the highest-risk code in the system deletes rather than moves.

| File | Lines | What it does |
|---|---|---|
| `crates/tam-session-broker/src/gateway.rs` | 1,575 | lease proxy, bearer check, cookie injection |
| `crates/tam-session-broker/src/vault.rs` | 1,478 | the only code holding ciphertext and the key-encryption key in one process |
| `crates/tam-session-broker/src/main.rs` | 406 | broker binary and operator commands |
| `crates/tam-session-broker/src/service.rs` | 399 | lease service |
| `crates/tam-session-broker/src/jar.rs` | 276 | Netscape jar handling |
| `crates/tam-session-broker/src/protocol.rs` | 192 | lease protocol types |
| `crates/tam-engine/src/broker_client.rs` | 249 | the engine's side of the lease |
| | **4,575** | |

The key-encryption key goes with them, and so do its escrow and rotation drills and the `LeasePurpose` Pump/Drain/Analytics distinction.
The four deliberate database roles plausibly collapse to two: `tam_broker` and the `connection_secret` table go entirely, and `tam_engine`'s BYPASSRLS justification — the cross-tenant lease scan — narrows and may not survive, leaving `tam_app` under forced RLS and `tam_auth` untouched.

What survives is everything that never needed a credential: connection identity, the global exclusivity digest that stops two organisations claiming one marketplace account, the linking/linked/revoked lifecycle, the audit trail and the authorship attestation.
One wrinkle: the exclusivity digest's HMAC pepper is currently broker-derived, so it needs re-siting somewhere that outlives the broker.

The honest loss is the gateway route allow-lists.
`TES_PREFIXES` and `TPT_PREFIXES` at `crates/tam-session-broker/src/gateway.rs:46` and `:63`, selected at `:111-112`, currently make roster, account-administration and payout routes structurally unreachable, and that is test-enforced in infrastructure we operate.
Client-side, the same rule downgrades from enforced-in-our-infrastructure to enforced-by-code-on-the-user's-machine.
The threat model largely evaporates on inspection — the allow-list mainly bounds *our own* worker's overreach, our signed client still enforces it, and a seller who patched the binary to reach their own payout routes harms only themselves and could already do it from their own browser with curl.
It is still a downgrade and it should go to counsel as one rather than be presented as a pure simplification.

Two further qualifications belong in the same honest column.
The custody is relocated rather than eliminated: a cookie on the seller's disk needs OS-keychain storage, and the claim "our server never sees it" holds against passive compromise of our infrastructure while failing against an actively malicious build of our own client.
And there is a product cost, which Vendoo concedes openly in its own help pages: scheduled sync, analytics refresh and dashboard freshness all become contingent on the seller's machine being on.

## 6. The legal-control spectrum, and where to sit on it

The control-plane stream's decisive repository finding is that the cheapest migration is the wrong one.
`gateway.rs` is already a per-lease loopback proxy injecting a cookie into worker-composed requests, so it is architecturally a dumb credentialed relay, and relocating just the gateway — turning the loopback hop into a network hop — leaves our server composing every request.
That is precisely the configuration the Ninth Circuit reserved.
Stated as the stream put it: the minimal client footprint that keeps the request client-side is not the minimal footprint that buys the legal position.
The seam that buys the position is one level up, at the marketplace adapter.

| Point | Who does what | Assessment |
|---|---|---|
| S0 | Client fully autonomous, server holds nothing | Safest legally, deletes the product |
| S1 | Server sends declarative intent; the client's local adapter composes and issues; the user set the schedule | **Recommended** — inside the express holding |
| S2 | Server sends instructions and parameters; the client composes | The pattern actually held safe in Perplexity |
| S3 | Server composes fully-formed requests; the client attaches the cookie and relays | Today's gateway, merely relocated; where the reservation sits |
| S4 | Server holds the credential and makes the request | Power Ventures; today's architecture; the exposed pattern |

S1 and S2 sit inside the holding because the court addressed server-to-client instruction flow directly: "Perplexity may receive screenshots ... and may communicate instructions to the Assistant. But those activities, by themselves, do not mean that Perplexity has 'accessed'" (https://cdn.ca9.uscourts.gov/datastore/opinions/2026/08/04/26-1444.pdf).
The reservation that S3 walks into is narrower than it is usually quoted as: "We do not address whether ... Perplexity may exercise control over the Assistant in such a way as to gain entry to Amazon's servers".
Two more facts from the same opinion calibrate how much room there is.
Perplexity won *despite* a disputed user-agent alteration, which was the near-fatal fact, so concealment is the aggravator to avoid rather than the thing that decides the case.
And tort claims are expressly left open, with footnote 5 confirming the decision "does not impair Amazon's ability to regulate access via private terms of service", which is why nothing in this note improves the tortious-interference position.

The largest remaining control risk is our own cron, and it is fixable.
The holding rests on contemporaneous user direction — "If the user activates the Assistant" — so a sync firing while the seller sleeps makes our server the initiator of the access.
The fix is to move the scheduler to the client: the seller configures the schedule, a local timer fires, the client pulls pending declarative work, and the server never says "do it now".
This preserves the charter's deterministic, cron-scheduled non-negotiable because only the location of the timer changes; it inverts the causation the analysis turns on; and it defeats the theory that severing our connection would stop the transaction, because it would not.
UCITA's electronic-agent attribution makes the seller the principal even for unattended action, which cuts our way, though it is contract attribution rather than CFAA doctrine.
The genuine gap, marked as such: no case was found addressing server-*scheduled* unattended client action, so this is reasoned from the holding rather than covered by it.

Two supporting findings.
The incumbents draw the same line we would: Vendoo and List Perfectly state "We don't even ask for your credentials!", describe a Chrome extension that "opens a new tab ... hidden copy and pastes", warn that "If you are not logged in ... we are not able to post for you", and yet do hold "For eBay and Etsy ... an access token" — because those have official APIs.
The rule is server-side where an official API authorises it, client-side where none exists, which is exactly our situation since TES and TPT have neither.
No litigation was found against any cross-listing tool, which is weak evidence of tolerance rather than evidence of legality.
Separately, there is an elegant convergence worth exploiting: Google's Manifest V3 remote-code rule — remote resources must not contain logic, configuration data is fine — maps exactly onto the S1/S2-versus-S3 legal line, so shipping selectors and route maps as signed *data* and never as code satisfies both constraints with one discipline.

## 7. Form factor: why native, and what the browser gave up

One finding reshaped the option space before any comparison could be made.
You can no longer attach to the seller's existing logged-in Chrome: Chrome 136 stopped respecting `--remote-debugging-port` against the default profile, citing an "increase in attackers using Chrome Remote Debugging to extract cookies" (https://developer.chrome.com/blog/remote-debugging-port), and the change propagated to Edge and other Chromium builds.
The obvious workaround — copying cookies out of the profile — is the infostealer pattern that Chrome 127's App-Bound Encryption exists to block (Google Security Blog, July 2024), with the bypasses catalogued as malware by security vendors, so it is ruled out on principle rather than on difficulty: it is precisely what opposing counsel would point at.
Firefox alone still permits attach via `geckodriver --connect-existing`, but Firefox-only is not a product.
The consequence is that **every** remaining path requires a fresh in-app login, which removes the extension's and browser-attach's main advantage and is why the comparison lands where it does.

| Option | Reuse of our Rust | Session source | Verdict |
|---|---|---|---|
| O1 Tauri v2 desktop, webview for login only | ~whole engine, `live.rs` unchanged | webview we own, `cookies_for_url()` | **Recommended** |
| O2 thirtyfour 0.37.5 + chromiumoxide | high | dedicated browser, fresh login | Rejected on detectability |
| O3 CEF via `tauri-apps/cef-rs` | ~whole engine | embedded Chromium | Escape hatch, not the plan |
| O4 Electron | zero | embedded Chromium | Baseline only |
| F6 Extension + native messaging | high (host side) | the seller's real browser | Best detectability, disqualified |
| Verso | — | — | Ruled out, archived 2025-10-08 |

O1's APIs are first-class and shipped rather than hacks: `WebviewUrl::External` with an `initialization_script` at document-start, `eval_with_callback` returning JSON to Rust (`tauri-runtime-wry` 2.11), `cookies_for_url()` and `set_cookie`, and `on_navigation(|url| -> bool)` to cancel navigations.
Tauri is TAO plus WRY over the OS webview (WKWebView, WebView2, WebKitGTK), stable at v2 since October 2024, with a sidecar mechanism for bundling a separate binary (https://v2.tauri.app/develop/sidecar/) — though for O1 the engine compiles in as a crate rather than shipping as a sidecar.
One critical constraint: drive Rust-to-page only, and never let the marketplace page call `invoke()`.
The site's own CSP blocks Tauri's `ipc.localhost` protocol and dynamic per-webview capability scoping is buggy, so the marketplace origin gets no capability at all.

O2 is rejected because WebDriver sets `navigator.webdriver` to true by specification and CDP leaves `cdc_` variables, and Cloudflare names Selenium, Puppeteer and Playwright as unsupported browsers (https://developers.cloudflare.com/cloudflare-challenges/reference/supported-browsers); the Chrome 136 finding means they would not inherit the seller's session anyway.
O3 is real and active — Chromium 151 — but its Tauri integration is alpha with live bundler bugs and it adds roughly 100MB per install, so it is the escape hatch if the webview proves unworkable, not the plan.
O4 is a mature version of the O1 pattern with zero Rust reuse, a worse updater and roughly 96MB against Tauri's ~3MB; choosing it would mean sidecar-ing the Rust engine anyway, at which point Tauri wins.

The extension deserves its own paragraph because it is the best legal story and still loses here.
An MV3 service worker with `host_permissions` bypasses CORS and Chrome treats its requests as same-site, so `SameSite=Strict` session cookies ride along (chromium.org, the extension content-script fetches document; developer.chrome.com, the extension storage-and-cookies document), which means our plain-HTTP adapters transpose roughly one-to-one with no DOM puppetry and therefore none of Vendoo's DOM-automation fragility.
From an extension the TLS and HTTP/2 fingerprint and the JS environment are the real browser's, which removes the two vectors that catch server-side clients.
Four things disqualify it as the *first* vehicle.
The redirect model is the hard one: `crates/tam-marketplace-tpt/src/live.rs` sets `Policy::none()` and classifies the TPT create by reading the 302 `Location` header, reqwest on wasm32 has no redirect control, and a browser's `redirect: "manual"` yields an opaque status-0 response — recoverable via the final URL or `chrome.webRequest.onHeadersReceived`, but it changes the classification contract and needs re-verifying against the captures.
Store distribution hands Google and Mozilla a no-warning takedown lever, which re-introduces the exact gatekeeper dependency the client move exists to remove, and that is disqualifying given that the cease-and-desist is the pivot.
Safari is a second product, not a build target: a converter, a container app, App Store review, missing APIs, and an In-App Purchase collision with Guideline 3.1.1 that bars licence keys.
And an extension cannot run cron at all, because `chrome.alarms` has a 30-second floor and fires only while the browser is open, which collides directly with the deterministic-schedule non-negotiable.

The extension nonetheless stays reachable as an **additive second data plane** rather than a fork, and this is the reason to do the work in the order given in section 9: the driver split that the native client forces is the same split the extension needs, so both vehicles sit behind one control-plane API.
Manifest V2 is fully dead in Chrome (removed from the store 31 August 2026), so any future extension is MV3-only.
Rust-in-extension via WASM works, with one trap to verify empirically: MV3's CSP must declare `wasm-unsafe-eval` for extension pages and service workers, and wasm-bindgen's `new Function` glue can require genuine `unsafe-eval`, which is not grantable — the upstream issue is `rustwasm/wasm-bindgen#3098` and this is **unverified** on our own build.
Mozilla additionally requires reproducible source submission for WASM builds, which is real per-release work.

Three detectability caveats belong on the record for the recommended option too.
Cloudflare rates embedded browsers as limited support rather than immune, so O1 is a better position than automation frameworks and not an exemption.
On Windows and macOS the webview is stock WebView2 or WKWebView, which is the strongest position available; on Linux it is WebKitGTK, which is both the weakest engine for detection and the biggest maintenance liability, to the point that Tauri ships an official troubleshooting page for its rendering failures.
And in O1 the actual marketplace calls still use our reqwest and rustls rather than a browser TLS fingerprint — but that is unchanged from today's server engine, the shipped TPT connector already succeeds this way, and what O1 changes is whose IP address and whose machine the request comes from, which is the entire point.
The observation that Vendoo and List Perfectly are both extension-based, making a Rust desktop client differentiated rather than merely acceptable, is medium confidence only: it rests on comparison journalism rather than on their own technical documentation.

## 8. Subscription and kill switch as one mechanism

The shape is standard offline licensing: one-time online activation binding to a device fingerprint, an Ed25519-signed entitlement artifact verified offline at every launch, periodic revalidation against our server, a defined grace period when we are unreachable, and clock-tamper detection.
The artifact carries subscription state *and* a per-marketplace connector grant set with a short validity window measured in hours.
That single design does two jobs: a lapsed subscription and a cease-and-desist from one marketplace are the same code path, and disabling TES or TPT across the entire installed fleet is a control-plane flag flip that propagates within one revalidation window, with no product-wide disable and no shipped update.

The number that matters is the kill-switch latency, which is the TTL plus the grace period, and it is the promise made to whoever sends the cease-and-desist.
It should be chosen deliberately and stated publicly rather than discovered under pressure; the research recommendation is a one-hour TTL with a 24-hour grace, failing closed.
This needs founder words because it is a public commitment.

Two honest limits.
A client-side check is bypassable by a determined user, so it is a good-faith mechanism rather than a guarantee; the complement that actually bites is server-side, because revoking work-item dispatch and mapping fetches degrades the product even on a patched binary.
And the design collides with a stated non-negotiable: the charter says authorisation "is decided in Rust from Postgres, and is never asserted by a token claim", while an entitlement token is by construction a claim.
The proposed scoping is that Postgres remains the decision-maker, the token only transports a decision already made, the server re-checks on every control-plane call regardless of the token, and the client-side gate is fail-closed and advisory only — an availability control on the seller's own machine rather than an authorisation decision.
Whether that scoping is an acceptable exception is a founder ruling, not an inference this note can make.

For the later extension path the same server-side check is the only real kill switch available, because unpublishing from the Chrome Web Store does not disable installed copies, Chrome self-hosting is closed to consumers, and store payments were removed in 2021 so the processor and the entitlement check are ours either way.
MV3's remote-code rule permits an entitlement check that returns a feature flag while banning shipped adapter logic, which is the same signed-data-not-code discipline section 6 already requires.

## 9. Migration shape and what it costs

The sequence is ordered so that each step is independently useful and none is wasted if the next is deferred.

1. Add the `MaybeSend` cfg shim (roughly 20 lines) so the `Transport: Send + Sync` bound at `crates/tam-marketplace/src/transport.rs:293` relaxes on wasm32; harmless today, and it unblocks both non-native paths.
2. Split `crates/tam-engine/src/driver.rs` from `tam-storage` behind a repository trait, so the effect interpreter runs against a remote ledger.
   This is the real work, and it is the same work the extension path would need.
3. Ship the native client running the adapters against `ReqwestTransport` unchanged.
4. Delete the broker and its 4,575 lines, and collapse the database roles.
5. Revisit the extension as a second data plane once the redirect classification is resolved empirically.

The thin-client shape — an extension acting as a remote-driven executor while the server ships one `HttpRequest` per hop — is worth about two weeks as a de-risking spike that proves the control-plane API shape while step 2 proceeds, and it must not become the destination.
It fails on both axes: it is the weakest legal posture, since our servers would compute every byte and direct every hop, which is S3 or worse; and it is mechanically prohibitive, because the send sites are strictly sequential (14 in TPT, 20 in TES) and TPT's S3 multipart upload cuts at 5MiB, so a 200MB resource becomes roughly 40 parts, each needing a signing oracle and a PUT, for 80-plus server round trips with the part bytes transiting our infrastructure.

The desktop tax is the honest cost of the recommendation and it is paid whether or not the code is easy.
There is no meaningful cross-compilation, so CI becomes a five-job matrix including a macOS runner that cannot be avoided.
Signing runs about $99 a year for Apple plus notarisation and about $120 a year for Windows via Azure Trusted Signing, with all certificates capped at one year since December 2025 and EV certificates no longer bypassing SmartScreen; Linux adds deb, rpm and AppImage packaging.
The realistic floor is $220–600 a year plus the CI matrix.
The updater requires a minisign key that is irreplaceable — lose it and the installed base can never be updated again — and on Windows the updater auto-exits the application on install.
WebKitGTK is a permanent maintenance burden confined to Linux.
And two costs are product costs rather than engineering ones: installing a desktop application is materially more friction than installing an extension, and scheduled sync only happens while the seller's machine is on.

## 10. Decisions this note puts to the founder

Recommendations are given for each; the four marked as needing words are not silence-adoptable.

1. **Form factor.** Recommend O1, a Tauri v2 desktop client with the webview used for login only.
2. **Where the seam goes.** Recommend lifting the marketplace adapter to the client (S1) rather than relocating the gateway (S3), accepting that the cheaper lift buys nothing legally.
3. **Client-side scheduler.** Recommend yes: the seller configures, a local timer fires, the client pulls; the charter's deterministic-cron property is preserved because only the location moves.
4. **The entitlement-token exception.**
   Needs founder words.
   The design contradicts the "never asserted by a token claim" non-negotiable unless the section 8 scoping is accepted as an explicit, bounded exception.
5. **Kill-switch latency.**
   Needs founder words.
   The TTL plus grace becomes a public commitment to a cease-and-desist sender; the recommendation is one hour plus 24 hours, failing closed.
6. **Do the resource file bytes stay client-side too?**
   If the upload is itself a marketplace request that must originate on the seller's machine, the bytes must be there at upload time, which argues for moving `tam-pipeline` client-side rather than round-tripping the file.
   The legally cleanest path is machine-to-marketplace with the bytes never touching our servers; the cost is that ingestion moves too.
7. **The route allow-list downgrade.**
   Needs counsel rather than a founder decision, but the founder decides when to ask.
   It moves from infrastructure-enforced to client-enforced, and section 5 gives the argument for why that matters less than it reads.
8. **Linux at launch?** Recommend Windows and macOS first with Linux best-effort, since WebKitGTK is the largest maintenance liability and it is confined to Linux.
9. **The blocking gap.**
   Needs founder words and a live action.
   Do the TES and TPT *login* pages carry a Cloudflare or DataDome challenge?
   The shipped connector already succeeds over plain HTTP against the API endpoints, which is weak evidence that those do not challenge, but the login flow is exactly the part O1 moves into a webview and is the part most likely to challenge.
   This requires a live read-only probe of the founder's own login pages from a stock browser and from an embedded webview, and it decides whether O1 is viable at all.

## 11. Sources

| Area | Reference | Form |
|---|---|---|
| CFAA | *Amazon v. Perplexity*, No. 26-1444 (9th Cir. 2026) — https://cdn.ca9.uscourts.gov/datastore/opinions/2026/08/04/26-1444.pdf | Full URL; retrieved and hash-verified per the legal memo |
| CFAA | *Facebook v. Power Ventures*; *Van Buren v. United States*; UCITA electronic-agent attribution | Via `docs/notes/legal/marketplace-terms-assessment.md`; no separate URL in this research |
| Tauri | Sidecar mechanism — https://v2.tauri.app/develop/sidecar/ | Full URL |
| Tauri | `WebviewUrl::External`, `initialization_script`, `eval_with_callback` (`tauri-runtime-wry` 2.11), `cookies_for_url()`, `set_cookie`, `on_navigation` | API names from the desktop stream; no doc URLs carried into the capture |
| Tauri | Official Linux WebKitGTK rendering-failure troubleshooting page | Cited by the stream; URL not captured |
| CEF | `tauri-apps/cef-rs`, Chromium 151, Tauri integration alpha | Repository reference; no URL captured |
| Extensions | Forbidden request headers — https://developer.mozilla.org/en-US/docs/Glossary/Forbidden_request_header | Full URL |
| Extensions | Extension content-script fetches (chromium.org); extension storage and cookies (developer.chrome.com) | Host and document name only; paths not captured |
| WASM | `wasm-unsafe-eval` CSP requirement and the wasm-bindgen `new Function` glue trap — `rustwasm/wasm-bindgen#3098` | Issue reference; **unverified against our build, verify empirically** |
| WASM | Mozilla AMO reproducible-source requirement for WASM builds | Stated by the stream; no URL captured |
| Automation | Chrome 136 `--remote-debugging-port` change — https://developer.chrome.com/blog/remote-debugging-port | Full URL |
| Automation | Chrome 127 App-Bound Encryption, Google Security Blog, July 2024 | Host and date; exact post URL not captured |
| Automation | Cloudflare supported browsers — https://developers.cloudflare.com/cloudflare-challenges/reference/supported-browsers | Full URL |
| Automation | thirtyfour 0.37.5 (last commit 2026-08-12); chromiumoxide; Verso archived 2025-10-08 | Crate and repository references; no URLs captured |
| Incumbents | Vendoo and List Perfectly help pages, quoted verbatim in section 6 | Quotes captured; source URLs not carried into the capture |
| Incumbents | Vendoo and List Perfectly are extension-based rather than desktop | **Medium confidence — comparison journalism, not vendor documentation** |
| Incumbents | No litigation found against any cross-listing tool | Absence of evidence; weak evidence of tolerance, not of legality |
| Licensing | Keygen and Keyforge offline-entitlement patterns; Ed25519 signed artifact, heartbeat revalidation | Named by the stream; no URLs captured |
| Store policy | MV2 removed from the Chrome Web Store 31 August 2026; store payments removed 2021; self-hosting closed to consumers; MV3 remote-code rule; Apple Guideline 3.1.1 | Stated by the stream; no URLs captured |
| This tree | `transport.rs:293`, `:83`, `:162`; `cassette.rs:52`; tpt `live.rs:515`, `:568`; tes `live.rs:153`, `:291`; `gateway.rs:46`, `:63`, `:111-112`; `driver.rs` | Verified against the working tree, 2026-08-31 |
