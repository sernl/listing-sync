# R2 — browser extension platform assessment (Chrome and Firefox, MV3)

All fetch dates are 2026-09-03.
Sources are restricted to developer.chrome.com, developer.mozilla.org, chromestatus.com, extensionworkshop.com (Mozilla's official developer site), bugzilla.mozilla.org and chromium.org.
No marketplace host and no product host was contacted.

Confidence scale: high = a primary source states the claim in words; medium = a primary source states it indirectly, or the source is a tracker comment rather than documentation; low = inferred from two primary facts with no direct statement.

## 1. Background lifetime

Chrome terminates an MV3 extension service worker on three independent conditions, and all three bite this workload.
The idle rule is thirty seconds, reset by an incoming event or by any call to an extension API since Chrome 110.
The per-request rule is five minutes: a single event or API call that takes longer than five minutes to process kills the worker, and Chrome 116 relaxed this only for a named set of APIs that display a user prompt, not for network work.
The fetch rule is the one that dominates: Chrome terminates the worker "when a `fetch()` response takes more than 30 seconds to arrive."
A 200 MB upload cannot return a response inside thirty seconds on any consumer uplink, so a single-request 200 MB PUT to marketplace S3 is not a viable unit of work in a Chrome service worker.
The realistic maximum for one continuous unit of work is therefore bounded by whichever is smaller: thirty seconds of waiting on any one fetch, or five minutes of total processing for one event.
Chrome's documented ways to hold a worker open are an active WebSocket (Chrome 116), long-lived messaging (Chrome 114), an attached debugger session (Chrome 118), a native messaging port (Chrome 105), and messages from an offscreen document (Chrome 109) — note that none of these lifts the per-fetch thirty-second rule, they only reset the idle timer.
The five-minute cap has not been removed; it is still stated as a live termination condition on the current lifecycle page.

Firefox does not support `background.service_worker` at all.
MDN states plainly that it "is not supported (see Firefox bug 1573659)", and that bug is still open with status NEW and no target milestone as of this fetch.
Firefox uses an event page instead, available from Firefox 106, with the same "unload when idle" model but no service-worker semantics.
MDN gives no number for the Firefox idle timeout; the Mozilla bug discussion for the feature describes a timer with a 30 second default, a 0.1 s minimum and a 300 s maximum, reset on each extension event, with suspension skipped while devtools are attached or a StreamFilter is active.
Because a Firefox event page is a real DOM document rather than a service worker, it does not carry Chrome's per-fetch thirty-second rule, but MDN's own text is contradictory on whether message ports hold it open, so its lifetime should be treated as no more reliable than Chrome's.
One manifest can carry both keys, and MDN documents this as the recommended cross-browser pattern: Chrome uses `service_worker`, Firefox uses `scripts`, Safari uses `scripts` unless `preferred_environment` says otherwise.
Chrome ignores `background.scripts` in an MV3 manifest from Chrome 121 rather than refusing to load, and Firefox from 121 starts the background page regardless of `service_worker` being present, so the dual-key manifest is safe on current versions of both.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| Terminates after 30 s of inactivity; events and extension API calls reset the timer | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| Terminates when a single request (event or API call) takes longer than 5 minutes | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| Terminates when a `fetch()` response takes more than 30 seconds to arrive | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| The 5-minute cap was relaxed only for APIs that display a user prompt (Chrome 116) | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| WebSocket, long-lived messaging, debugger, native messaging and offscreen messages extend lifetime | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| "`background.service_worker` is not supported" | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/background | 2026-09-03 | high |
| Bug 1573659 (meta: background service worker for MV3) is status NEW, no target milestone | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1573659 | 2026-09-03 | high |
| Non-persistent background pages (event pages) supported from Firefox 106 | Firefox | https://extensionworkshop.com/documentation/develop/manifest-v3-migration-guide/ | 2026-09-03 | high |
| Event page idle timer defaults to 30 s (min 0.1 s, max 300 s), reset by extension events | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1771203 | 2026-09-03 | medium |
| One manifest may carry both `background.scripts` and `background.service_worker` | both | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/background | 2026-09-03 | high |
| Chrome ≥121 ignores `background.scripts` in MV3 instead of refusing to load | Chrome | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/background | 2026-09-03 | high |

## 2. Scheduling

Chrome's alarm floor is thirty seconds from Chrome 120, having been one minute before that.
The API reference states "Chrome limits alarms to at most once every 30 seconds but may delay them an arbitrary amount more", and a `periodInMinutes` or `delayInMinutes` below 0.5 is not honoured and produces a warning.
There is no precision guarantee at all: "alarm firings can be arbitrarily delayed", and the unpacked-extension exemption removes the floor only during development.
Chrome caps active alarms at 500 from Chrome 117.
Alarms do fire while the browser is running and the worker is dead: the lifecycle page states that "if the service worker has gone dormant, an incoming event will revive them", and `chrome.alarms.onAlarm` is such an event — this is the intended replacement for `setTimeout` in a worker that does not survive.
Alarms survive device sleep but do not cause a wake: "Alarms continue to run while a device is sleeping. However, an alarm will not wake up a device", and missed alarms fire on wake.
Nothing fires while the browser is closed, and neither vendor states this as a sentence because an extension has no process then; the closest primary statements are MDN's "Alarms do not persist across browser sessions" and Chrome's `persistAcrossSessions` flag, which describes persistence of the alarm record across a restart, not firing while the browser is down.
MDN documents no Firefox-specific minimum period; every numeric floor it states is explicitly attributed to Chrome, so the Firefox floor is an open question.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| Minimum alarm period 30 s from Chrome 120 (previously 1 minute); sub-0.5-minute values warn | Chrome | https://developer.chrome.com/docs/extensions/reference/api/alarms | 2026-09-03 | high |
| "Alarm firings can be arbitrarily delayed" — no precision guarantee | Chrome | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/alarms/create | 2026-09-03 | high |
| Active alarms limited to 500 from Chrome 117 | Chrome | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/alarms/create | 2026-09-03 | high |
| An incoming event revives a dormant service worker | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| Alarms run during device sleep but do not wake the device; missed alarms fire on wake | Chrome | https://developer.chrome.com/docs/extensions/reference/api/alarms | 2026-09-03 | high |
| "Alarms do not persist across browser sessions" | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/alarms | 2026-09-03 | high |
| Nothing fires while the browser is closed | both | (no direct primary statement; inferred from the two rows above) | 2026-09-03 | medium |
| No documented Firefox-specific minimum alarm period | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/alarms/create | 2026-09-03 | high (as an absence) |

## 3. Cookies and requests

Chrome's extension documentation answers the SameSite question directly and favourably.
"Requests from an extension to a third-party are treated as same-site if the extension has host permissions for the third-party. This means `SameSite=Strict` cookies can be sent."
It qualifies this twice: the treatment applies to network requests only, not to `document.cookie`, and it does not apply if third-party cookies are blocked.
`HttpOnly` cookies are attached by the network stack rather than by script, so the same rule carries them; no source states this in words and it follows from the definition of `HttpOnly`.
CORS is bypassed for hosts in `host_permissions`, but only from the background context: MDN states that from Chrome 73 and Firefox 101 in MV3 "content scripts are subject to the same CORS policy as the page they are running within. Only backend scripts have elevated cross-domain privileges", and that host permissions "don't work in content scripts, but they still do in regular extension pages".
Credentials are not automatic and must be set explicitly, because `fetch`'s `credentials` option "defaults to `same-origin`" and a request from a `chrome-extension://` origin to a marketplace host is cross-origin — so every marketplace fetch needs `credentials: 'include'`.
What `Origin` and `Referer` the marketplace actually sees from a background fetch is the one thing in this section I could not settle from a primary source; MDN describes the privileged content-script instances as "not setting the `Origin` and `Referer` headers", but says nothing about the background context in MV3.
Header control in MV3 Chrome is via `declarativeNetRequest`'s `modifyHeaders` action, which requires `declarativeNetRequestWithHostAccess` plus host permissions; the docs list an allowlist for the `append` operation that excludes `Origin` and `Referer`, and say nothing about restrictions on `set` or `remove` for those two headers.
Blocking `webRequest` is gone in Chrome MV3 except for policy-installed extensions, where "the `webRequestBlocking` permission is still available in Manifest V3"; the Chrome docs make no statement about Firefox.
Firefox treats `host_permissions` as optional: MDN says "most browsers treat `host_permissions` as optional" and that users "can grant or revoke host permissions on an ad hoc basis".
From Firefox 127 host permissions listed in `host_permissions` and `content_scripts` are shown in the install prompt and granted on installation, but new host permissions added in an update are still not shown to the user (Firefox bug 1893232), so an extension must call `permissions.contains` and then `permissions.request` at runtime rather than assume the grant.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| Extension→third-party requests are same-site with host permissions; `SameSite=Strict` cookies can be sent | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/storage-and-cookies | 2026-09-03 | high |
| Applies to network requests only, not `document.cookie`; void if third-party cookies are blocked | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/storage-and-cookies | 2026-09-03 | high |
| Same-site treatment for extension requests exists from Chrome 79/80 onward, conditional on host permissions | Chrome | https://www.chromium.org/updates/same-site/faq/ | 2026-09-03 | medium |
| Only background/extension pages get elevated cross-domain privileges; content scripts follow page CORS (Chrome 73, Firefox 101 MV3) | both | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Content_scripts | 2026-09-03 | high |
| `fetch` `credentials` defaults to `same-origin`, so `include` is required for marketplace hosts | both | https://developer.mozilla.org/en-US/docs/Web/API/RequestInit | 2026-09-03 | high |
| `Origin`/`Referer` sent by an MV3 background fetch — undocumented | both | (not found in primary sources) | 2026-09-03 | — |
| `modifyHeaders` exists; `append` allowlist excludes `Origin`/`Referer`; `set`/`remove` unrestricted in the docs | Chrome | https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest | 2026-09-03 | medium |
| `webRequestBlocking` survives in MV3 only for policy-installed extensions | Chrome | https://developer.chrome.com/docs/extensions/develop/migrate/blocking-web-requests | 2026-09-03 | high |
| Host permissions are treated as optional and are user-revocable ad hoc | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/host_permissions | 2026-09-03 | high |
| From Firefox 127 host permissions appear in the install prompt and are granted on install | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/host_permissions | 2026-09-03 | high |
| New host permissions added by an update are not shown to the user (bug 1893232) | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/host_permissions | 2026-09-03 | high |
| Runtime grant path is `permissions.contains` then `permissions.request` | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/host_permissions | 2026-09-03 | high |

## 4. WebAssembly

Both browsers require the same CSP token and neither grants it by default.
Chrome's default `extension_pages` policy is `script-src 'self'; object-src 'self';`, under which "WebAssembly will be disabled"; the minimum policy that enables wasm is `script-src 'self' 'wasm-unsafe-eval'; object-src 'self';` and the policy "cannot be relaxed beyond this minimum value".
Firefox permits only `'self'`, `'none'` and `'wasm-unsafe-eval'` in `script-src` and `worker-src`, and states outright that "'wasm-unsafe-eval' must be specified in the CSP if an extension is to use WebAssembly".
Chrome requires `object-src`; Firefox has made it optional from Firefox 106; a single manifest supplying both directives satisfies both.
Firefox's inclusion of `worker-src` in the same allowlist is the best available evidence that wasm runs inside a worker/service-worker context, and MDN's worker-availability page does not enumerate WebAssembly either way — I found no sentence in either vendor's docs that says "WebAssembly runs in an extension service worker" in those words.
Wasm in an offscreen document and in a popup follows from both being extension pages governed by the same `extension_pages` policy.
Package size caps are 2 GB for the Chrome Web Store and 200 MB for AMO, so a Rust-to-wasm binary is nowhere near either limit.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| Default MV3 CSP disables WebAssembly | Chrome | https://developer.chrome.com/docs/extensions/reference/manifest/content-security-policy | 2026-09-03 | high |
| Minimum wasm-enabling policy is `script-src 'self' 'wasm-unsafe-eval'; object-src 'self';` and cannot be relaxed further | Chrome | https://developer.chrome.com/docs/extensions/reference/manifest/content-security-policy | 2026-09-03 | high |
| "'wasm-unsafe-eval' must be specified in the CSP if an extension is to use WebAssembly" | Firefox | https://extensionworkshop.com/documentation/develop/manifest-v3-migration-guide/ | 2026-09-03 | high |
| `script-src` and `worker-src` may only be `'self'`, `'none'`, `'wasm-unsafe-eval'` | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/content_security_policy | 2026-09-03 | high |
| `object-src` required in Chrome, optional in Firefox from 106 | both | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/content_security_policy | 2026-09-03 | high |
| Wasm runs in the service worker / worker context | both | inferred from `worker-src` accepting `'wasm-unsafe-eval'` (same URL as above) | 2026-09-03 | medium |
| Maximum package size 2 GB | Chrome | https://developer.chrome.com/docs/webstore/publish | 2026-09-03 | high |
| Add-ons larger than 200 MB fail validation | Firefox | https://extensionworkshop.com/documentation/publish/submitting-an-add-on/ | 2026-09-03 | medium |

## 5. DOM parsing

The strongest evidence that `DOMParser` is unavailable in a Chrome extension service worker is that Chrome shipped an entire API to work around it.
The offscreen API's reason enum contains `DOM_PARSER`, described as "specifies that the offscreen document needs to use the DOMParser API", which would be pointless if the worker had one.
MDN's list of Web APIs available to workers does not include `DOMParser`, though it also does not mark it unavailable, so this is an argument from two absences plus Chrome's workaround rather than a single stated sentence.
The offscreen document landed in Chrome 109, an installed extension may have only one open at a time, and no reason except `AUDIO_PLAYBACK` imposes a lifetime limit — so a `DOM_PARSER` offscreen document persists until closed.
Messages sent from an offscreen document reset the service worker's idle timers (Chrome 109), which makes it a lifetime tool as well as a DOM tool.
Firefox has no offscreen API and needs none: its event page is a real DOM document, so `DOMParser` is directly available there.
Parsing HTML in Rust removes the need for the offscreen document entirely on the Chrome side, and with it a cross-browser divergence, one extra process, the single-offscreen-document constraint and a message hop per parse.
It does not remove the offscreen document as a lifetime-extension trick, so if that trick is wanted it must be justified on its own terms.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| `DOM_PARSER` is a listed offscreen reason — "needs to use the DOMParser API" | Chrome | https://developer.chrome.com/docs/extensions/reference/api/offscreen | 2026-09-03 | high |
| `DOMParser` is absent from MDN's list of APIs available to workers | both | https://developer.mozilla.org/en-US/docs/Web/API/Web_Workers_API/Functions_and_classes_available_to_workers | 2026-09-03 | medium |
| Offscreen API is Chrome 109+, one document per installed extension at a time | Chrome | https://developer.chrome.com/docs/extensions/reference/api/offscreen | 2026-09-03 | high |
| Only `AUDIO_PLAYBACK` sets a lifetime limit (30 s); other reasons set none | Chrome | https://developer.chrome.com/docs/extensions/reference/api/offscreen | 2026-09-03 | high |
| Messages from an offscreen document reset the service worker idle timers | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| No offscreen API in Firefox; the event page is a DOM document | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/manifest.json/background | 2026-09-03 | medium |

## 6. Storage and memory

`chrome.storage.local` is capped at `QUOTA_BYTES` = 10485760 (10 MB; 5 MB in Chrome 113 and earlier), and the docs state the value "will be ignored if the extension has the `unlimitedStorage` permission".
Firefox instead subjects `storage.local` to the same limits as IndexedDB databases, and warns that even with `unlimitedStorage` an extension "may get a quota exceeded error when the disk space used by storage exceeds the global limit".
IndexedDB is available in workers per MDN and is explicitly stated to be accessible in extension service workers by Chrome's own storage documentation, alongside Cache Storage.
Neither vendor documents a memory limit for an extension background context, so the 200 MB payload question cannot be answered from primary sources at all — this is the largest documentation gap in this report.
Streaming a request body is where the two browsers diverge hardest and where the design gets pinned.
Chrome supports `ReadableStream` request bodies from Chromium 105, but "the fetch will be rejected if the connection is HTTP/1.x", `duplex: 'half'` must be set, and a non-303 redirect on a streaming-body request rejects.
Chrome's own article documents Safari's position and says nothing about Firefox, and I found no primary Firefox source confirming or denying `ReadableStream` upload support.
The HTTP/1.x rejection matters more than the browser gap: marketplace S3-style signed-upload endpoints commonly speak HTTP/1.1, in which case streaming upload is unavailable in Chrome too and the whole 200 MB body must be materialised as a `Blob`/`ArrayBuffer` before the request starts.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| `storage.local` `QUOTA_BYTES` = 10485760 (5 MB in Chrome ≤113) | Chrome | https://developer.chrome.com/docs/extensions/reference/api/storage | 2026-09-03 | high |
| The local quota "will be ignored if the extension has the `unlimitedStorage` permission" | Chrome | https://developer.chrome.com/docs/extensions/reference/api/storage | 2026-09-03 | high |
| `storage.sync` `QUOTA_BYTES` = 102400; `storage.session` = 10485760 | Chrome | https://developer.chrome.com/docs/extensions/reference/api/storage | 2026-09-03 | high |
| `storage.local` is bounded by IndexedDB limits; `unlimitedStorage` still subject to a global limit | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/storage/local | 2026-09-03 | high |
| IndexedDB and Cache Storage are accessible in service workers | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/storage-and-cookies | 2026-09-03 | high |
| IndexedDB and Web Crypto are available to workers | both | https://developer.mozilla.org/en-US/docs/Web/API/Web_Workers_API/Functions_and_classes_available_to_workers | 2026-09-03 | high |
| No documented memory limit for an extension background context | both | (not found in primary sources) | 2026-09-03 | — |
| Streaming request bodies from Chromium 105; requires `duplex: 'half'` | Chrome | https://developer.chrome.com/docs/capabilities/web-apis/fetch-streaming-requests | 2026-09-03 | high |
| "The fetch will be rejected if the connection is HTTP/1.x" | Chrome | https://developer.chrome.com/docs/capabilities/web-apis/fetch-streaming-requests | 2026-09-03 | high |
| A non-303 redirect on a streaming-body request rejects | Chrome | https://developer.chrome.com/docs/capabilities/web-apis/fetch-streaming-requests | 2026-09-03 | high |
| Firefox `ReadableStream` upload support — undocumented in the sources consulted | Firefox | (not found in primary sources) | 2026-09-03 | — |

## 7. Identity and crypto

The Web Crypto API is available to workers per MDN's worker-availability list, and `SubtleCrypto.sign` is marked available in Web Workers.
`Ed25519` is a first-class named algorithm for `SubtleCrypto` sign and verify, with SHA-512 as its fixed digest.
Chrome shipped it in Chrome 137; Firefox shipped it in Firefox 129, where bug 1804788 is RESOLVED FIXED against the "129 Branch" milestone.
Both are comfortably behind us as of this date, so verifying an EdDSA entitlement token with `crypto.subtle` in the background context is available on both browsers without wasm.
Verifying in wasm remains the portability floor if a materially older browser must be supported, and is the only option if the token format ever moves off Ed25519 to something Web Crypto does not name.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| Web Crypto API is available to workers | both | https://developer.mozilla.org/en-US/docs/Web/API/Web_Workers_API/Functions_and_classes_available_to_workers | 2026-09-03 | high |
| `SubtleCrypto.sign` is "available in Web Workers"; secure-context only | both | https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto/sign | 2026-09-03 | high |
| Ed25519 is a supported sign/verify algorithm; digest is always SHA-512 | both | https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto/sign | 2026-09-03 | high |
| Ed25519 in Web Crypto shipped in Chrome 137 | Chrome | https://chromestatus.com/feature/4913922408710144 and https://developer.chrome.com/release-notes/137 | 2026-09-03 | high |
| Ed25519 in Web Crypto shipped in Firefox 129 (bug 1804788 RESOLVED FIXED, "129 Branch") | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1804788 | 2026-09-03 | high |

## 8. Distribution and review

Chrome Web Store review is "completed within a few days, but it can take up to a few weeks", and the store tells developers to contact support only after three weeks pending.
Review explicitly takes longer for extensions "that request broad host permissions or sensitive execution permissions", which this extension will have by construction.
The single-purpose policy requires a narrow, easy-to-understand purpose and warns that "excessive permissions unrelated to your extension's single purpose will be viewed as enabling unrelated functionalities" — a cross-listing extension has a coherent single purpose, but its permission surface will be read against that purpose.
Obfuscation is banned outright ("developers must not obfuscate code or conceal functionality of their extension", extending to "any external code or resource fetched by the extension package"), while minification is expressly allowed in three named forms.
The Chrome code-readability policy does not mention WebAssembly or compiled binaries at all, and neither does the review-process page, so whether a Rust-compiled wasm blob reads as "concealing functionality" is undetermined by the published policy and rests on reviewer discretion.
I found no Chrome Web Store policy that names automating actions on third-party sites on a user's behalf, and none that names circumventing or violating a third-party site's terms of service; the searched pages (program policies index, quality guidelines FAQ, misleading or unexpected behavior, code readability) contain no such rule.
That absence is a finding in itself: the exposure is enforcement discretion under the deceptive-behavior and unexpected-behavior headings, not a bright-line published prohibition.
Chrome self-hosting is effectively unavailable for consumers — "Linux is the only platform where Chrome users can install extensions that are hosted outside of the Chrome Web Store", using `update_url` plus an `updates.xml` gupdate manifest.
Firefox requires Mozilla signing for release and beta: unsigned extensions install only in Developer Edition, Nightly and ESR after toggling `xpinstall.signatures.required`.
Self-distribution is real on Firefox via unlisted signing, through the Developer Hub, `web-ext sign`, or the AMO signing API, and "it can take up to 24 hours for your submission to be signed and published, or longer if your submission is selected for manual review".
Unlisted is not unreviewed: "all add-ons, including self-distributed ones, are subject to be manually reviewed at any time after submission" and all undergo automated validation before signing.
Mozilla's source-code rule is the concrete obligation for a wasm build: add-ons "may contain transpiled, minified or otherwise machine-generated code, but Mozilla needs to review a copy of the source code", with build instructions, environment and tool versions, the full command list, and lockfiles, sufficient for a reviewer "to rebuild your extension from the source code".
WebAssembly is not named in that trigger list, but "any other custom tool that takes files, applies pre-processing, and generates file(s)" plainly covers a Rust-to-wasm toolchain, so expect to submit the Rust source and a reproducible build.
Failure to comply risks rejection, delay, "or, in the worst-case, result in your extension being taken down".

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| Review "within a few days, but it can take up to a few weeks"; escalate after 3 weeks | Chrome | https://developer.chrome.com/docs/webstore/review-process | 2026-09-03 | high |
| Broad host permissions and sensitive permissions lengthen review | Chrome | https://developer.chrome.com/docs/webstore/review-process | 2026-09-03 | high |
| Single-purpose policy; excessive unrelated permissions read as unrelated functionality | Chrome | https://developer.chrome.com/docs/webstore/program-policies/quality-guidelines-faq | 2026-09-03 | high |
| Obfuscation banned; minification allowed in three named forms | Chrome | https://developer.chrome.com/docs/webstore/program-policies/code-readability | 2026-09-03 | high |
| Code-readability policy is silent on WebAssembly and compiled binaries | Chrome | https://developer.chrome.com/docs/webstore/program-policies/code-readability | 2026-09-03 | high (as an absence) |
| No published policy names third-party ToS circumvention or automating third-party sites | Chrome | https://developer.chrome.com/docs/webstore/program-policies/ , .../quality-guidelines-faq , .../unexpected-behavior | 2026-09-03 | medium (absence across three pages, not exhaustive) |
| Self-hosting outside the store works on Linux only, via `update_url` + `updates.xml` | Chrome | https://developer.chrome.com/docs/extensions/how-to/distribute/host-on-linux | 2026-09-03 | high |
| Signing by Mozilla required for release and beta Firefox | Firefox | https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/ | 2026-09-03 | high |
| Unsigned installs only in Developer Edition, Nightly, ESR with `xpinstall.signatures.required` toggled | Firefox | https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/ | 2026-09-03 | high |
| Unlisted (self-distributed) signing is supported; up to 24 h to sign and publish | Firefox | https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/ | 2026-09-03 | high |
| All add-ons, including unlisted, may be manually reviewed at any time after submission | Firefox | https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/ | 2026-09-03 | high |
| Machine-generated code requires a reviewable source-code package with rebuild instructions and lockfiles | Firefox | https://extensionworkshop.com/documentation/publish/source-code-submission/ and .../add-on-policies/ | 2026-09-03 | high |
| WebAssembly is not named in the source-submission trigger list, but generated-file tooling is | Firefox | https://extensionworkshop.com/documentation/publish/source-code-submission/ | 2026-09-03 | high |
| Non-compliance risks rejection, delay, or takedown | Firefox | https://extensionworkshop.com/documentation/publish/source-code-submission/ | 2026-09-03 | high |
| Add-ons must be self-contained and must not load remote code for execution | Firefox | https://extensionworkshop.com/documentation/publish/add-on-policies/ | 2026-09-03 | high |

## 9. Auto-update and kill switch

Chrome "checks for extension updates on startup and every few hours", with no numeric interval published.
The install is gated on idleness, not on the check: "an update is only installed when the extension is considered idle", which "primarily means that the extension's service worker is not running", and any open side panel, popup or options page also blocks it.
An active content script does not block the install, and Chrome warns that "this idle requirement can cause delays in updates for frequently active extensions" — an extension that runs a scheduled work loop is exactly such an extension, and may not go idle until the browser restarts.
`chrome.runtime.requestUpdateCheck` exists but is throttled and is documented as inappropriate for repeating-timer use; even a successful check still waits for idleness before installing.
Firefox checks once a day by default, `extensions.update.interval` = 86400 seconds, with 120 seconds the minimum supported value for testing.
Publishing an update on AMO adds up to 24 hours to sign, and longer under manual review, before the 24-hour client check can even see it.
The consequence for a server-side kill switch is direct: an extension update is a multi-day propagation channel on Chrome and a one-to-two-day channel on Firefox, so a kill switch must never depend on shipping a new version.
It must be enforced by the server refusing to issue work orders and by the client entitlement gate failing closed on expiry, both of which are already the architecture in the project's non-negotiables.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| "Chrome checks for extension updates on startup and every few hours" | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/extensions-update-lifecycle | 2026-09-03 | high |
| An update installs only when the extension is idle; a running service worker blocks it | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/extensions-update-lifecycle | 2026-09-03 | high |
| Open side panel, popup or options page blocks idleness; an active content script does not | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/extensions-update-lifecycle | 2026-09-03 | high |
| The idle requirement can delay updates for frequently active extensions indefinitely until restart | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/extensions-update-lifecycle | 2026-09-03 | high |
| `requestUpdateCheck` is throttled and unsuitable for repeating-timer use | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/extensions-update-lifecycle | 2026-09-03 | high |
| Default add-on update interval is 86400 s (24 h); 120 s is the testing minimum | Firefox | https://extensionworkshop.com/documentation/manage/updating-your-extension/ | 2026-09-03 | medium |
| Users update to a rolled-back version "within 24 hours" by default | Firefox | https://extensionworkshop.com/documentation/publish/version-rollback/ | 2026-09-03 | medium |
| Up to 24 h to sign and publish on AMO, longer under manual review | Firefox | https://extensionworkshop.com/documentation/publish/signing-and-distribution-overview/ | 2026-09-03 | high |

## Open questions not answerable from primary sources

What `Origin` and `Referer` a marketplace sees from an MV3 background-context fetch, and whether `declarativeNetRequest` `modifyHeaders` can `set` or `remove` them.
Whether Firefox supports `ReadableStream` request bodies with `fetch`, and under what transport conditions.
Any documented memory ceiling for a Chrome extension service worker or a Firefox event page.
Firefox's minimum honoured `alarms` period, which MDN attributes only to Chrome.
Whether a Rust-compiled wasm binary satisfies Chrome's code-readability policy, which does not mention compiled code.
Whether either store has an unpublished or case-law position on automating a user's actions against a third-party site's terms.

# Follow-up

All fetch dates 2026-09-03.
One source outside the original allowlist is used, and only for the single claim the lead specified: docs.aws.amazon.com for S3 multipart part sizes.

## F1. Escape hatches for the long upload on Chrome

### F1(a) The offscreen document

Chrome states the independence directly: "The lifetime of an offscreen document is independent of the service worker that created it."
The same post adds that "the lifetime of this page, and the permissions it will be granted are separate from that of the extension service worker", which settles both the survival question and the permissions question in one sentence.
An offscreen document therefore outlives the service worker's own termination, and the service worker does not need to be running for it to exist.
Only `AUDIO_PLAYBACK` sets a lifetime limit (30 seconds without audio); every other reason, `DOM_PARSER` and `WORKERS` and `BLOBS` included, sets no limit, so the document persists until `chrome.offscreen.closeDocument()` is called.
One installed extension may have exactly one open at a time, with a second permitted only for an active incognito profile under split mode.
The one API exposed inside it is `chrome.runtime` — "the `runtime` API is the only extensions API supported by offscreen documents" — so it can fetch and it can message, but it cannot call `chrome.alarms`, `chrome.storage` or anything else directly.
On the decisive question, no source states in words that the service-worker fetch rules do not apply to an offscreen document, so the answer is assembled from scoping.
The three rules live under the heading "Idle and shutdown" on the page titled "The extension service worker lifecycle", introduced by "Normally, Chrome terminates a service worker when one of the following conditions is met", and every keep-alive note on that page is phrased as extending "the service worker".
That page mentions offscreen documents only in the opposite direction — "messages sent from an offscreen document reset the timers" — treating the offscreen document as an external actor on the worker's timer rather than a subject of it.
Combined with the explicit independence statement, the rules are scoped to the service worker and do not govern a fetch issued from an offscreen document.
This is inference from scoping plus one explicit independence sentence, not a stated exemption, so it must be confirmed empirically before any design depends on it.
Two forward risks are stated in primary sources and should not be discounted: the launch post says the page "will have a lifetime mechanism similar to event pages in Manifest V2" and "will be torn down when it stops performing actions", and that "the user agent may place further restrictions on the lifetime specific to the purpose specified".
Chrome has not shipped such a limit for non-audio reasons, but it has reserved the right to, in writing.
Nothing in any consulted source addresses Chrome closing an offscreen document under memory pressure, so that remains unanswered.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| "The lifetime of an offscreen document is independent of the service worker that created it" | Chrome | https://developer.chrome.com/blog/Offscreen-Documents-in-Manifest-v3 | 2026-09-03 | high |
| Lifetime and permissions are "separate from that of the extension service worker" | Chrome | https://developer.chrome.com/blog/Offscreen-Documents-in-Manifest-v3 | 2026-09-03 | high |
| Only `AUDIO_PLAYBACK` sets a lifetime limit; all other reasons set none | Chrome | https://developer.chrome.com/docs/extensions/reference/api/offscreen | 2026-09-03 | high |
| One open offscreen document per installed extension (plus one for incognito under split mode) | Chrome | https://developer.chrome.com/docs/extensions/reference/api/offscreen | 2026-09-03 | high |
| "The `runtime` API is the only extensions API supported by offscreen documents" | Chrome | https://developer.chrome.com/docs/extensions/reference/api/offscreen | 2026-09-03 | high |
| The 30 s fetch and 5-minute request rules are scoped to "a service worker" and do not govern an offscreen document | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle (scoping) + blog above (independence) | 2026-09-03 | medium |
| Chrome reserves the right to add further lifetime restrictions per reason | Chrome | https://developer.chrome.com/blog/Offscreen-Documents-in-Manifest-v3 | 2026-09-03 | high |
| Memory-pressure closure of an offscreen document — undocumented | Chrome | (not found in primary sources) | 2026-09-03 | — |

### F1(b) An extension page in a tab

An extension page opens in a tab without user action: the `chrome.tabs` reference states that "most features don't require any permissions to use. For example: creating a new tab", and its own example is `chrome.tabs.create({ url: 'onboarding.html' })`, an extension-relative URL that resolves under `chrome-extension://`.
No source documents any idle termination for an extension page in a tab, and the service-worker lifecycle rules do not reach it by the same scoping argument as F1(a).
The real hazard is tab discarding, and it is documented: `discard()` "discards a tab from memory", a discarded tab's "content is reloaded the next time it is activated", and `autoDiscardable` controls "whether the tab can be discarded automatically by the browser when resources are low".
A background runner tab is inactive by definition, so it is a prime discard candidate, and discarding destroys in-flight work exactly as service-worker termination does.
The documented mitigation is to set `autoDiscardable: false` on the runner tab, which is a per-tab property the extension controls.
No source states that Chrome honours `autoDiscardable: false` unconditionally under severe memory pressure, so treat it as a strong hint rather than a guarantee.
The unavoidable cost is that this tab is visible in the user's tab strip, which is a product decision rather than a platform limit.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| Creating a tab requires no permission and no stated user gesture | Chrome | https://developer.chrome.com/docs/extensions/reference/api/tabs | 2026-09-03 | high |
| Extension-relative URLs open in tabs via `tabs.create` | Chrome | https://developer.chrome.com/docs/extensions/reference/api/tabs | 2026-09-03 | high |
| No documented idle-termination rule for an extension page in a tab | Chrome | (absence across the lifecycle and tabs references) | 2026-09-03 | medium |
| `autoDiscardable`: "whether the tab can be discarded automatically by the browser when resources are low" | Chrome | https://developer.chrome.com/docs/extensions/reference/api/tabs | 2026-09-03 | high |
| A discarded tab's content "is reloaded the next time it is activated" | Chrome | https://developer.chrome.com/docs/extensions/reference/api/tabs | 2026-09-03 | high |

### F1(c) Cookie treatment from these contexts

Both contexts are extension pages on the `chrome-extension://` origin, and the cookie rule is written against the extension, not against the service worker: "Requests from an extension to a third-party are treated as same-site if the extension has host permissions for the third-party."
The Chromium SameSite FAQ frames the same rule page-first — the request is same-site "if an extension page initiates a request to a web URL" and the extension holds host permission for it — which if anything documents extension pages more directly than the worker.
Chromium's cross-origin document says the CORS bypass belongs to extension pages generally: "Extension pages, such as background pages, popups, or options pages, are unaffected by this change" and "will continue to be allowed to bypass CORS for cross-origin requests as they do today."
So an offscreen document and a runner tab get the same host-permission cookie treatment as the worker, with the same three caveats: network requests only, void if third-party cookies are blocked, and `credentials: 'include'` still required because the fetch default remains `same-origin`.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| The same-site rule is written against "an extension" holding host permissions, not against a context | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/storage-and-cookies | 2026-09-03 | high |
| The rule is framed as "an extension page initiates a request to a web URL" | Chrome | https://www.chromium.org/updates/same-site/faq/ | 2026-09-03 | medium |
| Extension pages "continue to be allowed to bypass CORS for cross-origin requests" | Chrome | https://www.chromium.org/Home/chromium-security/extension-content-script-fetches/ | 2026-09-03 | high |

## F2. Firefox event page and in-flight fetch

MDN publishes no list of what resets the Firefox idle timer, so this is answered from Bugzilla.
Bug 1851373, "Firefox terminates the background script of WebExtensions after 30 seconds", is VERIFIED FIXED against the 119 branch, uplifted to 118 and backported to 117.0.1, and it exists precisely because event pages "responsible for long lasting tasks" were being killed mid-task.
Mozilla's position on the 30-second suspend is that it is deliberate: Luca Greco writes that "the current behavior introduced in Bug 1830767 is actually the intended behavior and should have been the case since the original changes to introduce the event pages."
What that bug fixed was messaging, not network activity — the timer reset had been an accidental side effect of `runtime.onMessage` routing, and the fix restored it deliberately.
A commenter demonstrated, and Greco confirmed, that ports are not enough: "for backgrounds with active ports, Firefox will still force stop after 30 seconds", tracked onward as a separate gap.
The closest thing to fetch coverage is bug 1785294, RESOLVED FIXED on the 106 branch, which made Firefox "implicitly track promise returned by API listeners" and "defer terminating the background context for a bit longer if the promises are still pending", while "eventually forcefully terminating the event page if they are still pending after a certain extended timeout".
That mechanism keys on a promise returned from a WebExtension API event listener, not on a bare `fetch()`, so a fetch only benefits if it is awaited inside such a listener — for example inside an `alarms.onAlarm` handler that returns the promise.
Even then the reprieve is bounded by an "extended timeout" whose value is not published anywhere I could find, and the patch titles show it may reset the idle timeout "only once".
So the answer is: a pending fetch is not itself activity, an awaited fetch inside an API listener buys one bounded extension, and there is no published maximum lifetime for the event page beyond the design bug's 300-second ceiling on the idle timer itself.
Firefox is not the safe harbour for a long upload; it fails on a different schedule than Chrome, with less documentation.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| MDN publishes no list of what resets the event page idle timer | Firefox | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Background_scripts | 2026-09-03 | high (as an absence) |
| Bug 1851373 VERIFIED FIXED (119 branch; uplifted 118, backported 117.0.1) — event pages killed mid-task | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1851373 | 2026-09-03 | high |
| The 30-second suspend "is actually the intended behavior" | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1851373 | 2026-09-03 | high |
| Active ports do not prevent force-stop at 30 seconds | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1851373 | 2026-09-03 | medium |
| Pending promises from API event listeners defer termination "for a bit longer" (bug 1785294, fixed in 106) | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1785294 | 2026-09-03 | high |
| Forced termination follows "after a certain extended timeout" whose value is unpublished | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1785294 | 2026-09-03 | high |
| A bare in-flight `fetch()` is not tracked as activity | Firefox | inferred: neither bug names fetch; the tracked object is a listener-returned promise | 2026-09-03 | medium |
| No published maximum event page lifetime | Firefox | (not found in primary sources) | 2026-09-03 | — |

## F3. Origin and Referer on a cross-origin POST from an extension context

Firefox is documented and the answer is `moz-extension://<uuid>`.
Bug 1607936 records the expected result verbatim as "'Origin: moz-extension://HASH' should have been sent in the header", filed because a regression had made Firefox send "'Origin: null'" instead.
That regression came from bug 1405971, "Webextension UUID leak to servers via Fetch request headers", an intentional privacy change to stop the per-install UUID reaching servers; it broke CORS for legitimate extensions and was backed out.
The consequence for this product is concrete: a marketplace sees a stable per-installation identifier on every extension-origin request, which is a ready-made signal for detection and blocking, and Mozilla has already tried and abandoned suppressing it.
Chrome publishes no equivalent statement, so its behaviour is assembled indirectly.
The `webRequest` reference lists "the `Origin` request header" among headers that require `extraHeaders` to modify since Chrome 79, which establishes that extension requests carry one.
The same reference states the limit on spoofing it: extensions "can't change the `request origin` or initiator, which is a concept defined in the Fetch spec", so modifying the header alone "might not work as intended and may result in unexpected errors" in the response's CORS checks.
Chromium's cross-origin document supplies the contrast that identifies the value: content-script fetches "will have an `Origin` request header with the page's origin", whereas extension pages are a separate class that bypasses CORS — which is only coherent if extension-page requests carry the extension's own origin.
Reading those together with the Fetch specification's rule that `Origin` accompanies any non-GET/HEAD request, a POST from a Chrome extension context carries `Origin: chrome-extension://<id>`, and the same value applies from the worker, an offscreen document and an extension tab, since all three share that origin.
This should be confirmed against a request bin before anything depends on it.
No primary source I found states what `Referer` a background fetch sends in either browser.
On `declarativeNetRequest`, neither Chrome's reference nor MDN's `ModifyHeaderInfo` page names `Origin` or `Referer` anywhere at all; the only published restriction is Chrome's allowlist for the `append` operation, which excludes both, leaving `set` and `remove` undocumented rather than prohibited.
The blocker is upstream of the header list.
MDN states that "for all requests, except for navigation requests (i.e., resource type `main_frame` and `sub_frame`), host permissions are also required for the request's initiator", and the initiator of the extension's own fetch is its own `chrome-extension://` or `moz-extension://` origin, for which an extension cannot hold a host permission.
MDN also lists "requests from other extensions" among requests that "cannot be matched by extensions", and Firefox bug 1825824 tracks exactly this class under the title "[DNR] initiator-based access checks is stricter than webRequest for extension:-initiated requests".
So `declarativeNetRequest` is not a dependable instrument for rewriting headers on the extension's own requests, and even where a rewrite landed, Chrome has already stated that the underlying request origin is immutable, so a server-side check comparing `Origin` against its own host would still fail.
The consequence is the sharpest architectural finding here: if a marketplace's CSRF defence requires `Origin` to equal its own host on a POST, no documented extension API makes a background-context request satisfy it.
The one documented path that does satisfy it is to issue the request from a content script running in a tab on the marketplace's origin, because "cross-origin fetches initiated from content scripts will have an `Origin` request header with the page's origin".
That inverts the transport: the write must originate inside a marketplace tab, with the background context reduced to orchestration, which is also a closer fit to the project's own "originates in the seller's own browser under the seller's own session" non-negotiable.
The cost is that content scripts lost host-permission CORS privileges in Chrome 73 and Firefox 101 MV3, so such a request is an ordinary same-origin page request and succeeds only where the marketplace's own front end would succeed — which for its own create form and its own signed-upload endpoint is exactly the case.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| Background-script cross-origin requests send `Origin: moz-extension://<uuid>` | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1607936 | 2026-09-03 | high |
| Suppressing it to `null` (bug 1405971) broke CORS and was backed out | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1607936 | 2026-09-03 | high |
| Extension requests carry an `Origin` header; it needs `extraHeaders` to modify since Chrome 79 | Chrome | https://developer.chrome.com/docs/extensions/reference/api/webRequest | 2026-09-03 | high |
| Extensions "can't change the `request origin` or initiator"; header spoofing yields CORS failure | Chrome | https://developer.chrome.com/docs/extensions/reference/api/webRequest | 2026-09-03 | high |
| Content-script fetches carry "the page's origin"; extension pages are a separate class that bypasses CORS | Chrome | https://www.chromium.org/Home/chromium-security/extension-content-script-fetches/ | 2026-09-03 | high |
| A POST from a Chrome extension context therefore sends `Origin: chrome-extension://<id>` | Chrome | inferred from the three rows above plus the Fetch spec's non-GET/HEAD rule | 2026-09-03 | medium |
| `Referer` on a background fetch — undocumented in both browsers | both | (not found in primary sources) | 2026-09-03 | — |
| Neither dNR reference names `Origin` or `Referer`; only `append` has a published allowlist, excluding both | both | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/declarativeNetRequest/ModifyHeaderInfo and https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest | 2026-09-03 | high |
| Host permissions are required for the request's initiator on all non-navigation requests | both | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/declarativeNetRequest | 2026-09-03 | high |
| "Requests from other extensions" cannot be matched by dNR | both | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/declarativeNetRequest | 2026-09-03 | high |
| Bug 1825824: dNR initiator checks are stricter than webRequest for extension-initiated requests | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1825824 | 2026-09-03 | medium |
| A content script in a marketplace tab sends the marketplace's own origin | Chrome | https://www.chromium.org/Home/chromium-security/extension-content-script-fetches/ | 2026-09-03 | high |

## F4. Chunked uploads

Both fetch-related rules are written per request, and the wording carries the scope.
The five-minute rule reads "when a single request, such as an event or API call, takes longer than 5 minutes to process", and the thirty-second rule reads "when a `fetch()` response takes more than 30 seconds to arrive" — one fetch, one response.
So an S3 multipart upload whose parts each answer inside thirty seconds does not trip the fetch rule, and the worker survives, provided each part is a separate `fetch()`.
There is a trap in the other rule that the per-request reading does not remove.
The five-minute limit is scoped to "a single request, such as an event or API call", which is the event dispatch, not the individual fetch — so a single `alarms.onAlarm` handler that loops over forty parts is one request for the purposes of that rule and dies at five minutes regardless of how fast each part completes.
A multi-part upload must therefore be driven across separate event dispatches, one part per alarm or a self-rescheduling chain, rather than inside one long handler.
That in turn collides with the thirty-second alarm floor: forty parts at one alarm each is at least twenty minutes of wall clock, which argues for the offscreen document or runner tab from F1 as the driver instead, where neither rule applies.
S3's part-size floor sets the arithmetic: parts are "5 MiB to 5 GiB" with "no minimum size limit on the last part", up to 10,000 parts and a 48.8 TiB object.
A 200 MB payload at the 5 MiB floor is roughly forty parts, and answering each within thirty seconds needs about 1.4 Mbps sustained upstream — comfortable on most connections, marginal on poor ones, and larger parts trade headroom against that margin.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| The 30-second rule is per `fetch()` response | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| The 5-minute rule is scoped to "a single request, such as an event or API call" | Chrome | https://developer.chrome.com/docs/extensions/develop/concepts/service-workers/lifecycle | 2026-09-03 | high |
| A loop over many parts inside one event handler is still one request under the 5-minute rule | Chrome | inferred from the rule's wording above | 2026-09-03 | medium |
| S3 part size is 5 MiB to 5 GiB; no minimum on the last part | — | https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html | 2026-09-03 | high |
| Maximum 10,000 parts per upload; maximum object 48.8 TiB | — | https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html | 2026-09-03 | high |

## Follow-up open questions

Whether Chrome closes an offscreen document under memory pressure, and whether `autoDiscardable: false` is honoured under severe pressure.
The value of Firefox's "extended timeout" before a background context with pending listener promises is forcibly terminated.
What `Referer` a background-context fetch sends in either browser.
Empirical confirmation of the `Origin` value Chrome sends on a POST from worker, offscreen document and extension tab, which no primary source states.
Whether `declarativeNetRequest` `set`/`remove` on `Origin` or `Referer` is silently rejected as well as being blocked by the initiator check.

## F5. Addendum — the fetch-metadata envelope

All fetch dates 2026-09-03.
The W3C Fetch Metadata specification is used here as the lead permitted, alongside the Chromium and Mozilla trackers and Chromium source.

### F5(a) What the four Sec-Fetch-* headers carry from an extension context

Two of the four are structurally fixed by the fact that the request is a `fetch()`, and no extension context changes them.
MDN defines `Sec-Fetch-Dest: empty` as the value "used for destinations that do not have their own value. For example: `fetch()`, `navigator.sendBeacon()`, `EventSource`, `XMLHttpRequest`, `WebSocket`", and reserves `document` for when "the request is the result of a user-initiated top-level navigation".
MDN defines `Sec-Fetch-Mode: navigate` as "the request is initiated by navigation between HTML documents", against `cors` for "a CORS protocol request", and notes these values "correspond to the values in `Request.mode`" — a value a `fetch()` cannot select, since `navigate` is not a settable request mode.
`Sec-Fetch-User` is not merely fixed but absent: it "is sent for requests initiated by user activation, and its value is always `?1`", and "when a request is triggered by something other than a user activation, the spec requires browsers to omit the header completely."
Its absence is itself a signal, so a gate looking for `?1` sees nothing, not a wrong value.
The one primary artefact that shows all of this together is the reporter's capture in Firefox bug 1722703, a background-page XHR arriving as `Sec-Fetch-Dest: empty`, `Sec-Fetch-Mode: cors`, `Sec-Fetch-Site: cross-site` — dest and mode exactly as predicted, with no `Sec-Fetch-User` line at all.
Only `Sec-Fetch-Site` varies, and the spec ties it to access rather than to which extension context issued the request: §4.4 says that without access "its requests to that URL could contain a `Sec-Fetch-Site` header whose value is `cross-site`", and "if the extension does have access to a given URL, the `Sec-Fetch-Site` value could be `same-origin`."
Firefox implemented exactly that: bug 1722703 is RESOLVED FIXED on the 92 branch, its patch titled "Consider requests from extension with access to the requested site as Sec-Fetch-Site: 'same-origin'", after shipping Sec-Fetch-* in Firefox 90 with `cross-site` and breaking real sites.
Chrome took the other reading and sends `none`, per a comment in bug 1722044 — "Firefox Nightly 93.0a1 sends 'Sec-Fetch-Site': 'same-origin' header when using fetch from a WebExtension. Chrome sends 'Sec-Fetch-Site': 'none'" — which is a tracker comment rather than Chrome documentation, so medium confidence, and Chrome publishes nothing on this.
Because the value follows access rather than context, the worker, an offscreen document and an extension tab all send the same triple, and the browsers disagree on the one header that varies.
The spec also gates the whole family on transport: "if r's url is not a potentially trustworthy URL, return", which is satisfied for any HTTPS marketplace and so is not a constraint here.
The conclusion is unambiguous and it is a structural one, not a policy one: a `fetch()` from any extension context cannot produce a navigation-shaped envelope, because dest is `empty` and not `document`, mode is `cors` and not `navigate`, `Sec-Fetch-User` is absent rather than `?1`, and site is `none` on Chrome or `same-origin` on Firefox rather than whatever a real navigation would have carried.
If the marketplace's gate keys on a navigation envelope, only an actual navigation produces one — which means navigating a tab to the form, not fetching it.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| `Sec-Fetch-Dest: empty` is the value for `fetch()` and `XMLHttpRequest`; `document` is for a user-initiated top-level navigation | both | https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Sec-Fetch-Dest | 2026-09-03 | high |
| `Sec-Fetch-Mode: navigate` means "initiated by navigation between HTML documents"; values correspond to `Request.mode` | both | https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Sec-Fetch-Mode | 2026-09-03 | high |
| `Sec-Fetch-User` is sent only on user activation, is always `?1`, and is otherwise omitted entirely | both | https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Sec-Fetch-User | 2026-09-03 | high |
| Observed background-page XHR envelope: `Sec-Fetch-Dest: empty`, `Sec-Fetch-Mode: cors`, `Sec-Fetch-Site: cross-site`, no `Sec-Fetch-User` | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1722703 | 2026-09-03 | high |
| Spec §4.4: extension without access may send `cross-site`; with access may send `same-origin` | both | https://w3c.github.io/webappsec-fetch-metadata/ | 2026-09-03 | high |
| Firefox sends `same-origin` when the extension has access (bug 1722703, RESOLVED FIXED, 92 branch) | Firefox | https://bugzilla.mozilla.org/show_bug.cgi?id=1722703 | 2026-09-03 | high |
| Chrome sends `Sec-Fetch-Site: none` for a WebExtension `fetch()` | Chrome | https://bugzilla.mozilla.org/show_bug.cgi?id=1722044 (tracker comment; no Chrome doc) | 2026-09-03 | medium |
| The value follows host access, not which extension context issues the request, so worker/offscreen/tab are identical | both | https://w3c.github.io/webappsec-fetch-metadata/ §4.4 | 2026-09-03 | medium |
| Fetch-metadata headers are sent only to potentially trustworthy URLs | both | https://w3c.github.io/webappsec-fetch-metadata/ | 2026-09-03 | high |
| Spec §4.4 also anticipates an `Origin` from extension contexts "with an implementation-defined value that allows servers to distinguish extension-initiated requests" | both | https://w3c.github.io/webappsec-fetch-metadata/ | 2026-09-03 | high |

### F5(b) Whether the extension can rewrite the envelope, and whether it can read a manual redirect

Four of the eight headers in the envelope are `Sec-` prefixed and are forbidden request headers by name.
MDN's forbidden-request-header glossary lists "`Sec-` headers" as a whole category, alongside `Origin`, `Referer`, `Cookie` and `Host`, and defines the category as headers that "cannot be set or modified programmatically in a request" because "the user agent retains full control over them".
MDN's fetch-metadata glossary says it of these four specifically: "These headers are prefixed with `Sec-`, and hence are forbidden request headers. As such, they cannot be modified from JavaScript."
That statement scopes to JavaScript, and extension header APIs sit below the JavaScript layer, so it does not settle the extension case on its own.
The Chromium evidence points the same way without being conclusive: `net::HttpUtil::IsSafeHeader`'s unit-test data lists `"sec-"`, `"sEc-"` and `"sec-foo"` among unsafe header names against `"foo"` and `"x-"` as safe, and there is a Chromium issue titled "Unchecked runtime.lastError: Unsafe request header name" arising from extensions attempting to set blocked headers.
Chrome's own `webRequest` reference cuts the other way on the other four: its worked example "illustrates how to delete the `User-Agent` header from all requests", and it names `Origin` (Chrome 79+), `Accept-Language`, `Accept-Encoding`, `Referer` and `Cookie` (Chrome 72+) as headers that can be modified or removed once `'extraHeaders'` is specified — no `Sec-` header appears in any of its restricted lists at all.
So the published position is that user-agent, origin, referer and accept-language are modifiable and the four sec-fetch-* are not, which is exactly the wrong half.
None of this is the binding constraint, because the binding constraint is upstream of the header list and is high confidence.
MDN states that "for all requests, except for navigation requests (i.e., resource type `main_frame` and `sub_frame`), host permissions are also required for the request's initiator", and the initiator of the extension's own fetch is its own `chrome-extension://` or `moz-extension://` origin, for which an extension cannot hold a host permission.
MDN separately lists "requests from other extensions" among requests that "cannot be matched by extensions", and Firefox bug 1825824 tracks this class explicitly as "[DNR] initiator-based access checks is stricter than webRequest for extension:-initiated requests".
`declarativeNetRequest` therefore cannot be relied on to match the extension's own requests at all, which makes the per-header question moot for this design.
And even where a rewrite did land, Chrome has already stated the ceiling: extensions "can't change the `request origin` or initiator, which is a concept defined in the Fetch spec", so a server comparing `Origin` to its own host still sees the extension.
On the redirect question the answer is yes, by a different hook than the one implied.
`webRequest` remains available in MV3 for observation — "aside from `\"webRequestBlocking\"`, the webRequest API is unchanged and available for normal use" — and `onHeadersReceived` is "fired when HTTP response headers of a request have been received" with `responseHeaders` being "the HTTP response headers that have been received with this response".
`Location` appears on no restricted response-header list; only `Set-Cookie` (Chrome 72+) and `X-Frame-Options` (Chrome 89+) require `'extraHeaders'`, so a 302's `Location` is readable there even though the `fetch()` itself resolves to an opaqueredirect response with no headers.
`onBeforeRedirect` is the wrong hook: it "fires when a redirect is about to be executed", and with `redirect: "manual"` no redirect is executed, so it should not fire — that is inference from the two definitions, not a stated rule.
The extension's own requests are observable: the only self-request exclusion documented is that "synchronous XMLHttpRequests from your extension are hidden from blocking event handlers in order to prevent deadlocks", and the hidden-request list covers other extensions' origins and a few sensitive browser URLs, not the extension's own async fetches.
The cost is that this requires the `webRequest` permission plus host permissions, and `webRequest` is among the permissions Chrome names as lengthening review.

| Claim | Browser | Source URL | Fetched | Confidence |
|---|---|---|---|---|
| "`Sec-` headers" are forbidden request headers that "cannot be set or modified programmatically in a request" | both | https://developer.mozilla.org/en-US/docs/Glossary/Forbidden_request_header | 2026-09-03 | high |
| The four fetch-metadata headers "cannot be modified from JavaScript" | both | https://developer.mozilla.org/en-US/docs/Glossary/Fetch_metadata_request_header | 2026-09-03 | high (scoped to JS) |
| `Accept-Language` is not on the forbidden list; `Origin`, `Referer`, `Cookie`, `Host` are; `User-Agent` no longer is | both | https://developer.mozilla.org/en-US/docs/Glossary/Forbidden_request_header | 2026-09-03 | high |
| Chromium's `IsSafeHeader` test data treats `"sec-"`-prefixed names as unsafe | Chrome | https://chromium.googlesource.com/chromium/src/+/9d3bb0c75ddbeafd53f5ad32813b0fe70b7e0779%5E%21/ | 2026-09-03 | medium |
| Extensions attempting blocked header names produce "Unsafe request header name" errors | Chrome | https://issues.chromium.org/issues/40676639 | 2026-09-03 | medium |
| `webRequest` can delete `User-Agent`; `Origin`, `Referer`, `Accept-Language`, `Cookie` modifiable with `'extraHeaders'` | Chrome | https://developer.chrome.com/docs/extensions/reference/api/webRequest | 2026-09-03 | high |
| No `Sec-` header appears on any webRequest restricted or not-provided list | Chrome | https://developer.chrome.com/docs/extensions/reference/api/webRequest | 2026-09-03 | high (as an absence) |
| No dNR reference in either browser names `Sec-Fetch-*`, `Origin` or `Referer` for `set`/`remove` | both | https://developer.chrome.com/docs/extensions/reference/api/declarativeNetRequest and https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/declarativeNetRequest/ModifyHeaderInfo | 2026-09-03 | high (as an absence) |
| dNR requires host permissions for the request's initiator on all non-navigation requests | both | https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/declarativeNetRequest | 2026-09-03 | high |
| dNR cannot match the extension's own requests (initiator is its own extension origin) | both | same as above, plus https://bugzilla.mozilla.org/show_bug.cgi?id=1825824 | 2026-09-03 | medium |
| Extensions "can't change the `request origin` or initiator" regardless of header rewriting | Chrome | https://developer.chrome.com/docs/extensions/reference/api/webRequest | 2026-09-03 | high |
| Non-blocking `webRequest` is available in MV3: "aside from `webRequestBlocking`, the webRequest API is unchanged" | Chrome | https://developer.chrome.com/docs/extensions/reference/api/webRequest | 2026-09-03 | high |
| `onHeadersReceived` exposes `responseHeaders`, so a 302 `Location` is readable despite an opaqueredirect `fetch()` response | Chrome | https://developer.chrome.com/docs/extensions/reference/api/webRequest | 2026-09-03 | medium |
| `onBeforeRedirect` should not fire under `redirect: "manual"` because no redirect is executed | Chrome | inferred from the definitions of `onBeforeRedirect` and `redirect: "manual"` | 2026-09-03 | medium |
| Only synchronous XHRs from the extension are hidden, and only from blocking handlers | Chrome | https://developer.chrome.com/docs/extensions/reference/api/webRequest | 2026-09-03 | high |

### F5(c) What this decides

The marketplace write path is not reachable from a background extension context if the marketplace gates on a navigation-shaped fetch-metadata envelope, and the reason is structural rather than a missing permission.
Four of the eight headers in the TPT envelope are browser-controlled and derived from the request's destination, mode and activation, none of which a `fetch()` can select; the four that extensions can rewrite are precisely the four that are not sec-fetch.
Nothing in either browser's extension API surface closes that gap, and `declarativeNetRequest` cannot even match the extension's own requests to try.
The path that does work is the one F3 already identified for `Origin`, and it works here for the same reason: a real navigation in a real tab produces `Sec-Fetch-Dest: document`, `Sec-Fetch-Mode: navigate`, `Sec-Fetch-Site` per the actual relationship and `Sec-Fetch-User: ?1` natively, because the browser is genuinely doing what those headers describe.
The refinement this addendum adds to F3 is that a content-script `fetch()` is still a `fetch()` and reproduces the page's envelope, not a navigation's, so if the gate wants a navigation envelope the form must be reached by navigating a tab to it rather than by fetching its URL.
That is a heavier architecture — drive a tab, wait for load, read the DOM through a content script, submit from page context — but it is the only shape for which the evidence says the headers come out right, and it is a closer reading of the project's "every marketplace request originates on the seller's own device under the seller's own session" than a background fetch ever was.
The remaining unknown worth resolving before committing is whether TPT's gate actually keys on the sec-fetch family or on the user-agent and accept-language pair, since the latter two are rewritable on requests dNR can match and would not force the tab-driving architecture.
