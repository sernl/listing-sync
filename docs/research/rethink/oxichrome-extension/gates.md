# Kill gates G1 to G3 — measurements

The four gates are stated in `oxichrome-extension-client.md` section 6.
This file records what each one measured, with the commands that produced it.
G4, the store's view of a wasm blob, is not covered here because it requires a store submission rather than a local run.

Everything here ran against local servers bound to `127.0.0.1`; nothing in this file contacted TeachersPayTeachers, Tes or any other marketplace host.
G1 is built and verified against a local mimic, and is the founder's to run.

The browser throughout is Chromium 151.0.7922.71 from nixpkgs, invoked as `nix shell nixpkgs#chromium --command chromium`.
`--headless=new` loads an unpacked MV3 extension and runs its service worker on this version, so no virtual display was needed.
The probes and their servers live in `apps/extension-probe/`.

## G2 — the 302 `Location` through non-blocking `webRequest`

Verdict: pass.

The gate asks whether an MV3 extension can read the `Location` header of a 302 that its own `fetch()` never follows, and join that header to the request that produced it.
`crates/tam-marketplace-tpt/src/classify.rs:488-500` makes this the whole classification contract for a create or edit submit: a 302 whose `Location` yields a product id is the only landing shape, and everything else is `NoDurableIdentifier`.

```
G2_TIMEOUT=60 bash apps/extension-probe/run-g2.sh /tmp/g2-result.json
```

The extension is `apps/extension-probe/g2-redirect/`, declaring `webRequest` and host permissions for `http://127.0.0.1/*` and nothing else; `webRequestBlocking` is absent from the manifest, which the report re-checks at runtime and records as `webRequestBlockingDeclared: false`.
Its service worker registers six `webRequest` listeners at top level, then issues `fetch(url, {redirect: "manual", credentials: "include"})` against a Python server that answers `302 Found` with `Location: /redirect-target?product=88117001`.
A second run with `redirect: "follow"` is issued for contrast.
The captured report is `apps/extension-probe/g2-result.json`.

The fetch resolved as the memo predicted and the listener saw what the fetch could not.
`response.type` is `opaqueredirect`, `response.status` is `0`, and `[...response.headers.keys()]` is the empty list, so nothing about the redirect is recoverable through the fetch API.
`onHeadersReceived`, registered non-blocking with `["responseHeaders"]` and without `extraHeaders`, delivered the complete header set: `HTTP/1.1 302 Found` with `Location: /redirect-target?product=88117001` among the eight headers, confirming F5(b)'s claim that `Location` is on no restricted list.
The join succeeds on both keys the gate asked about.
Every stage of the manual run carries `requestId` `2`, and every stage carries the request URL `http://127.0.0.1:8731/redirect-source?mode=manual` unchanged, so `classify_submit` can be handed the header by either.

`onBeforeRedirect` does fire under `redirect: "manual"`, which the memo left open, and it carries strictly more than the header does.
Its `redirectUrl` is `http://127.0.0.1:8731/redirect-target?product=88117001`, already resolved against the request URL, where the raw `Location` header is the relative `/redirect-target?product=88117001`.
A product-id extractor that expects an absolute URL should therefore read `onBeforeRedirect.redirectUrl` and treat the `onHeadersReceived` `Location` as the fallback, rather than the reverse.

One asymmetry matters for the implementation and is not in any source consulted.
Under `redirect: "manual"` the event sequence is `onBeforeRequest`, `onSendHeaders`, `onHeadersReceived`, `onBeforeRedirect`, `onErrorOccurred` with `error: net::ERR_ABORTED` — the redirect is observed and then the request is aborted rather than completed.
Under `redirect: "follow"` the same `requestId` continues through a second `onBeforeRequest`/`onSendHeaders`/`onHeadersReceived` for the target and ends in `onCompleted` with status 200.
So a driver that keys "this request is finished" on `onCompleted` will never see the manual case finish, and must treat `onErrorOccurred` with `ERR_ABORTED` following an `onBeforeRedirect` as the success path.

The run is stable.
A second execution against a fresh profile produced byte-identical values on every decisive field: response type, status, the captured `Location`, whether `onBeforeRedirect` fired, the stage sequence, and the empty fetch-visible header list.

## G3 — the payload path at the cap

Verdict: pass, on both hosts.

The gate drives a 256 MiB payload as 52 multipart parts at TPT's 5 MiB part size (`crates/tam-marketplace-tpt/src/s3.rs:25`) from a driver other than the service worker, and asks three questions rather than one: whether the driver survives the whole upload, whether the worker is terminated and revived cleanly mid-upload, and what it costs in memory.

```
G3_LABEL=offscreen-3 G3_VARIANT=offscreen G3_TOTAL_BYTES=268435456 \
  G3_PART_BYTES=5242880 G3_PART_DELAY=0.8 G3_PROFILE=/tmp/g3-profile-off3 \
  bash apps/extension-probe/run-g3.sh
```

`G3_VARIANT=tab` selects the runner-tab variant instead.
Reusing `G3_PROFILE` across two invocations is what makes the second one a genuine browser restart against an already-installed extension.
The extension is `apps/extension-probe/g3-payload/`; the sink is `apps/extension-probe/servers/g3-sink.py`, which accepts `PUT /part/<n>` and records each arrival.
The sink delays 0.8 s per part deliberately, to stretch the run past the worker's 30-second idle threshold; that delay, not the network, sets the throughput figure below.

Seven runs were made: four offscreen (two fresh profiles, two restarts) and three runner-tab (two fresh, one restart).
Every one sent 52 of 52 parts, 268435456 bytes, arriving in index order, with no failure.
The last part is 1 MiB and the other 51 are 5 MiB, which is the shape S3 requires.

The offscreen document survives.
`chrome.offscreen.createDocument` accepted the reason `BLOBS` on the first attempt in every run, so the `WORKERS` fallback the probe carries was never exercised.
The document then ran for the whole 42-second upload and posted its own report at the end, with the worker dead for roughly thirty of those seconds.
Allocating the 256 MiB `Uint8Array` took 3 to 7 ms and never failed, so holding the payload by value — the shape `FileSource` returns — is not the constraint r2 feared it might be at this size.

The worker is terminated mid-upload and revived cleanly, and the runner-tab variant let us watch it happen.
`chrome.runtime.getContexts({contextTypes: ["BACKGROUND"]})` polled every three seconds returned one context until 29.8 s and zero from 32.8 s onward in run `tab-2`, and 29.1 s to 32.1 s in `tab-3-restart` — the documented 30-second idle rule, observed directly at three-second resolution, reproducing across a browser restart to within 700 ms.
The worker had gone idle immediately after opening the driver and never touched it again.
Revival is clean: the driver's `chrome.runtime.sendMessage` at the end of the upload respawned the worker, whose top-level code re-ran, re-announced itself to the sink, was told the run was already claimed, and started nothing.
In every one of the seven runs the sink recorded exactly two worker spawns, the second within 12 ms of the driver's wake message, and `getContexts` after the wake returned one context again.

One asymmetry between the two hosts is worth recording because it shapes any diagnostics we build.
`chrome.runtime.getContexts` is not a function inside an offscreen document on this version, despite `chrome.runtime` being the API an offscreen document has, so an offscreen driver cannot observe the worker's liveness for itself; a runner tab can.
The offscreen variant's lifecycle evidence is therefore the spawn beacon the worker sends the sink on every spawn, which is what the probe uses.
The valid context types on 151 are `BACKGROUND`, `DEVELOPER_TOOLS`, `OFFSCREEN_DOCUMENT`, `POPUP`, `SIDE_PANEL` and `TAB`; a worker is `BACKGROUND`, and `SERVICE_WORKER` is rejected.

The runner tab behaved as documented and cost nothing extra to set up.
`chrome.tabs.create` opened it inactive with no user gesture, and `chrome.tabs.update(tabId, {autoDiscardable: false})` returned `autoDiscardable: false`, so the hint was accepted.
Its numbers are indistinguishable from the offscreen document's: 42.29, 42.50 and 42.31 seconds against 42.66, 42.26 and 42.28 for the offscreen runs.
Nothing here induced memory pressure, so r2's open question — whether Chrome closes an offscreen document, or ignores `autoDiscardable: false`, under real pressure — is untouched by this gate and stays open.

Memory, by `/proc/<pid>/status` `VmRSS` sampled every 0.5 s over every process whose command line names the run's `--user-data-dir`.
That method was chosen over `chrome://memory-internals` because it needs no automation surface and attributes cleanly per process.
The extension renderer holding the payload peaked between 311.3 and 317.4 MiB across the seven runs, against a JS heap that reported 257 MiB used of 257 MiB total in every single run — so roughly 55 to 60 MiB of process overhead above the payload itself, and no evidence of a second copy being held anywhere.
The whole browser instance peaked between 1124.5 and 1175.4 MiB, of which the renderer above is the only part that scales with the payload.

Throughput is an artefact of the sink and should not be read as a network measurement: 6.02 to 6.06 MiB/s, which is 52 parts against a deliberate 0.8 s per-part delay.
The client-observed per-part latency confirms it — minimum 804 ms, median 812 ms, 95th percentile 815 ms — so the browser contributed about ten milliseconds per 5 MiB part.

One outlier deserves recording rather than smoothing away.
Run `offscreen-2-restart` completed all 52 parts but took 70.8 s, with a single part taking 28325 ms while its neighbours took 817 ms.
The cause is not attributable from what was captured, and it did not recur in the six other runs; the sink is a single Python process sharing the machine with a sampler, so a host-side stall is the likelier explanation than a browser one.
It is worth keeping because of where it landed: 28.3 s is 1.7 s under the 30-second per-`fetch()` limit that terminates a service worker, and had that part been driven from the worker rather than from the offscreen document, the run would have been roughly two seconds from dying.
That is the concrete form of the argument for hosting the driver outside the worker, measured rather than reasoned.

The pass condition is met on both hosts: the upload completes, and it completes again after a browser restart, four times over.

## G1 — the header envelope against a live TPT

Verdict: built, verified end to end against a local mimic, awaiting the founder.

The gate is a founder-gated live action on the founder's own machine and
account, so nothing here contacted TeachersPayTeachers.
The probe is `apps/extension-probe/g1-tpt-envelope/`, and its founder-facing
instructions are the README in that directory, modelled on
`tools/login-probe/README.md`.
It exercises the two URLs the memo names, `https://www.teacherspayteachers.com/Login`
and `https://www.teacherspayteachers.com/My-Products/New/Digital-Next`, the
second taken from `CREATE_DIGITAL_PATH` in
`crates/tam-marketplace-tpt/src/endpoints.rs:506`.

Four routes: the three section 6 specifies, plus one the self-check showed was
needed.
Route A issues the request from the service worker with `credentials: "include"`.
Route B issues it from a content script injected into an already-open
marketplace tab.
Route C navigates a tab the extension opens to the create form and reads the
arriving document from a content script.
Route D navigates the seller's already-open marketplace tab to the same form by
setting `location.href` from a content script, so the marketplace document is
the navigation's initiator.
Each route records, through `onSendHeaders` with `extraHeaders` and
`onHeadersReceived`, the request headers actually sent, the response status,
every `cf-*` response header, and the page markers `tools/login-probe`
looks for, with `formServed` computed from the five hidden inputs
`crates/tam-marketplace-tpt/src/form.rs:182-186` scrapes.
The pass condition per route is the form served rather than an interstitial.

Output needs no permission beyond those the routes already require: the panel
shows the JSON in a textarea with a copy button and a `Blob`-backed save link,
so the `downloads` permission is not requested.
The manifest asks for `webRequest`, `scripting`, `tabs` and `storage`, and host
permissions for the marketplace and for `127.0.0.1` (the latter only so the
local verification can run).

Nothing sensitive leaves the browser.
Cookie values are stripped and only cookie names are recorded, on both the
request and the `set-cookie` response; every header outside a short
non-sensitive allowlist is recorded as present without its value; and page text
is captured only when a marker fired and then only 300 characters.
The self-check confirms this rather than asserting it: the mimic sets
`csrfToken=mimic-csrf-value` and `session_id=mimic-session`, and neither string
appears anywhere in the 18093-byte report.

```
G1_TIMEOUT=120 bash apps/extension-probe/verify-g1.sh /tmp/g1-selfcheck.json
```

That runs all six route invocations against `apps/extension-probe/servers/g1-mimic.py`,
which serves a login page with a password input, a Cloudflare-like interstitial,
a redirect, and a create form carrying the five hidden inputs and the
credentials block.
All six ran and all six classified correctly: `CLEAR` with one password input
and `formServed: false` on the login shape, `CLEAR` with `formServed: true` on
the create shape, and the cookie names `csrftoken` and `session_id` riding along
on every route.

One mechanical note for anyone re-running it.
A `chrome-extension://` URL passed as a startup argument is resolved before the
unpacked extension has finished loading and is silently dropped, and headless
Chromium refuses more than one startup target at all, so the harness starts the
browser on `about:blank` and then opens the panel through the DevTools HTTP
endpoint (`PUT /json/new?<encoded url>`).
The unpacked extension's id is the first sixteen bytes of the SHA-256 of its
absolute directory path mapped onto `a`-`p`, which the harness computes and
which was confirmed against the browser's own `/json/list`.

The self-check settles three things about the probe's own reach that were open
in r2, and they are properties of the browser rather than of the mimic.
The `sec-fetch-*` family is visible to `onSendHeaders` with `extraHeaders`;
F5(b)'s finding that no `sec-fetch` header appears on a modifiable list is about
modification, and observation is unaffected.
Route A's envelope is exactly what F5(a) predicted for Chrome — `Sec-Fetch-Site: none`,
`Sec-Fetch-Mode: cors`, `Sec-Fetch-Dest: empty`, no `Sec-Fetch-User`.
It also carries no `sec-ch-ua` client hints at all: routes B, C and D each send
`sec-ch-ua`, `sec-ch-ua-mobile` and `sec-ch-ua-platform`, and route A sends none
of the three.
That is a second detectability signal r2 does not name, and it is worse than the
`sec-fetch` one, because a request claiming to be Chrome in its user agent while
sending no client hints is a state no Chrome produces — which is the same
mismatch `live.rs:18-24` records Cloudflare challenging.
Like the `sec-fetch` family, it is not fixable by rewriting: `sec-ch-ua` carries
the `Sec-` forbidden-header prefix.
Route B carries `Sec-Fetch-Site: same-origin` and the page's `Referer`, but
`Sec-Fetch-Dest: empty` and `Sec-Fetch-Mode: cors`, so it is a fetch's envelope
wearing the page's origin, as section 4 predicts.
Routes C and D are the only shapes producing `Sec-Fetch-Dest: document` and
`Sec-Fetch-Mode: navigate`, and they differ on the header that decides whether
the envelope matches the capture.

That difference is why route D exists, and it was found by the self-check rather
than reasoned about.
Route C navigates a tab the extension opened, which the browser treats as having
no initiator, so it sent `Sec-Fetch-Site: none` and no `Referer` at all — where
`crates/tam-marketplace-tpt/src/live.rs:57-62` hardcodes `same-origin` from the
captured browser.
Route D navigates the seller's own tab from inside the page, and sent
`Sec-Fetch-Site: same-origin`, `Sec-Fetch-Mode: navigate`,
`Sec-Fetch-Dest: document` and `Referer: <the page it left>` — which is the
envelope `live.rs` records, reproduced.
Without route D a single live run could not separate "TPT refuses a navigation
an extension shaped" from "TPT refuses a navigation carrying
`Sec-Fetch-Site: none`", and those are different answers with different build
consequences; with it, one run distinguishes them.

One limit of the probe as built is worth stating before the founder runs it,
because it is not recoverable from the report afterwards.
All four routes issue GET requests, and the Fetch specification attaches
`Origin` only to non-GET, non-HEAD requests, so `origin` is absent on every
route and r2 F3's inferred `Origin: chrome-extension://<id>` for a POST stays
untested by this run.
That is a deliberate consequence of the probe being read-only rather than an
oversight: every POST TPT accepts on the create path is a write, so there is no
harmless POST available to settle it, and the question has to wait for a route
that is willing to create a product.
The `Origin` half of r2 F3 therefore stays inferred, while the `sec-fetch` half
is now measured.
