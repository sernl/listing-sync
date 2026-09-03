# The extension client shell

The browser-extension surface as it exists today: a shell that builds, loads and answers, with no marketplace logic in it, plus the two store channels it would ship through and what the founder must set up personally before either is possible.

- date: 2026-09-03
- method: built and measured on this machine; the shell was loaded in headless Chromium 151.0.7922.71 and its popup read back; every size and hash below is measured rather than estimated
- status: the shell is built and green under `just check` and `just check-portable`; nothing has been submitted to any store, and the extension remains a candidate rather than a commitment
- decided by: `docs/research/rethink/oxichrome-extension-client.md` section 8, whose four decisions the founder took on 2026-09-03
- store facts second-hand: the Chrome and Mozilla policy statements in sections 6 and 7 are quoted from the dated citation table in `docs/research/rethink/oxichrome-extension/r2-platform.md`, which recorded them from the primary pages on 2026-09-03; the primary URLs are named so they can be re-read, and they should be re-read at submission time because store documentation changes without notice

## 1. What the shell is

`apps/extension` is the crate `tam-extension`, version 0.1.0, built as a `cdylib` plus `rlib` for `wasm32-unknown-unknown` through wasm-bindgen pinned at exactly `=0.2.121`.
The pin is not a preference: the wasm-bindgen CLI refuses a crate whose version differs from its own, and the CLI is the one nixpkgs carries at the revision `flake.lock` pins, which is the same reasoning `crates/tam-core-wasm` records.
This is the second `cdylib` in the workspace and it adds nothing structural to the pipeline the core already established.

The Rust half is four modules.
`lib.rs` exposes `version()` and `health()`, both returning strings, on the JSON-string boundary discipline `tam-core-wasm` set: one wire format rather than two, and an error shape rather than a trap, because a trap poisons the module instance and takes every later call with it.
`bridge.rs` is the `!Send` boundary crossed once, and is the reason the crate exists at this stage.
`alarms.rs` hand-declares `chrome.alarms`, which no Rust bindings crate covers.
`transport.rs` is a browser request shaped like a transport send, demonstrating that the bridge satisfies the bound; it names no host, sets no marketplace header and carries no session.

The bridge is decision 4 of the memo, taken in preference to the `MaybeSend` cfg shim.
Every marketplace transport in this workspace returns a future bound by `+ Send`, and a browser request cannot satisfy that bound directly because `JsFuture` holds an `Rc<RefCell<_>>`.
The `!Send` future is spawned on the thread that created it and the caller receives a `oneshot::Receiver`, which is `Send` whenever its payload is, so the `!Send` half never crosses the bound and only its answer does.
The spawner is a parameter rather than a direct call to `spawn_local`, which is what makes the pattern testable on the host: wasm passes `spawn_local` and the tests pass a hand-driven executor on `Waker::noop()`.

One type-level detail is worth recording because it was not obvious and the compiler had to be argued with.
The bridge returns a named `Answer<T>` rather than `impl Future`, because a return-position `impl Trait` captures every type parameter in scope, which would capture both the spawner and the `!Send` future.
Precise capturing does not rescue it: `use<...>` must mention all type parameters, so it cannot be narrowed to `T` alone.
A named struct holding one receiver is the only construct that can state the invariant, which is that the answer is all that survives the call.

The JavaScript half is four hand-written files in `apps/extension/static`.
`manifest.json` is Manifest V3 and carries both `background.service_worker` and `background.scripts` with `type: module`, which is decision 2's cheap way of keeping Firefox reachable without serving it.
Its `content_security_policy.extension_pages` is `script-src 'self' 'wasm-unsafe-eval'; object-src 'self'`, it requests `alarms` and `storage` and nothing else, and its three host permissions are enumerated rather than wildcarded: `www.teacherspayteachers.com`, `www.tes.com` and `api.teachouse.io`, matching the `ORIGIN` constants in the two adapter crates and `DEFAULT_BASE_URL` in the desktop control plane.
Enumeration rather than `<all_urls>` is what every incumbent cross-lister does, and it is also what keeps the single-purpose reading in section 6 defensible.

`background.js` registers every listener synchronously at top level before any await, and instantiates the wasm afterwards.
That ordering is the whole point: Chrome delivers an event to a terminated service worker only when the listener was registered during the worker's first evaluation, and registering after an await is the defect that makes oxichrome's generated worker unusable.
The `onMessage` handler returns `true` to keep the channel open for an asynchronous reply, which is the second thing oxichrome cannot express, because its generated closures return unit.

The `wasm-unsafe-eval` question the prior assessment left open is closed on our own build rather than reasoned about.
The generated glue contains zero occurrences of `new Function` and zero of `eval(`, instantiating through `WebAssembly.instantiateStreaming` with a plain `WebAssembly.instantiate` fallback, so the grantable CSP token suffices.

Measured on the built artefacts: `extension_bg.wasm` is 104,278 bytes raw and 38,238 gzipped, the generated glue is 5,938 and 1,827, the four hand-written files total 3,226 and 1,749.
The shipped package is 113,442 bytes of content, 41,814 gzipped, and the zip itself is 42,920 bytes.
Both stores' package ceilings are orders of magnitude above that and are not a consideration.

## 2. The build

`just extension-dev` produces the unpacked extension at `apps/extension/dist`, which a browser loads from disk.
`just extension-build` packages it, and `just extension-source-package` produces the reviewable source archive section 7 requires.

The package is deterministic by construction, because a reviewer who cannot reproduce our bytes cannot check the artefact against the source we hand them.
Three things in a zip are otherwise free to vary and all three are fixed: entries are written in sorted order, every timestamp is 1980-01-01, and every mode is 0644.
Members are named explicitly rather than walked, so a file ships because someone listed it, which is also how the two `.d.ts` files wasm-bindgen writes for TypeScript consumers stay out of the package.

Determinism of the zip is not sufficient on its own, because the wasm inside it must be reproducible too.
Absolute paths reach the binary through panic messages and debug information, so the build remaps both the checkout root and the cargo registry root to fixed names.
Without that remapping the bytes depend on where the tree and the registry cache happen to sit, and a reviewer on their own machine cannot reproduce them however deterministic the packer is.

The current artefact is `73c036b0043919c9d1af5ffc2d2fa917917d2e09d71bff91496cbcda1bb4ba9a`, and three independent builds agree on it: the repository's normal build, a rebuild in a separate cargo target directory, and a build from the extracted source package in a different filesystem location entirely.

The manifest and the crate state one version, and `apps/extension/tests/version_sync.rs` fails when they diverge.
The manifest is embedded at compile time with `include_str!` rather than read at run time, so the test cannot pass against a copy the build did not use.
Bumping a release therefore means editing both, and the build stops if only one is edited.

`apps/extension/verify-shell.sh` loads the built directory in headless Chromium and reports what the browser made of it.
On Chromium 151.0.7922.71 the service worker registers from the dual-key manifest and the popup renders `shell 0.1.0 — ok`, which is the version coming back out of the wasm module under the extension-pages CSP.
The script derives the unpacked extension id from the SHA-256 of the absolute `dist` path, because that is how Chrome derives it, and scraping the target list instead would also catch Chrome's own component extensions.

## 3. Redirect handling

The 302 `Location` read is the classification contract for a TPT create, and it is a JavaScript problem rather than a Rust one.
Gate G2 settled how it behaves, measured on Chromium 151 with `webRequest` declared and `webRequestBlocking` absent; the artefact is `apps/extension-probe/g2-result.json`.

Under `redirect: "manual"` the fetch resolves to an `opaqueredirect` response with status 0 and zero headers visible to the caller, so the `Location` is reachable only through `webRequest.onHeadersReceived`, which sees it in plain `responseHeaders` with no `extraHeaders` opt-in.
The header as served is relative while `onBeforeRedirect.redirectUrl` is already absolutised, so a product-id extractor should read `redirectUrl` first and fall back to the `Location` header.

The completion rule is the part that will bite if it is not written down.
Under `manual` the request sequence ends `onBeforeRedirect` then `onErrorOccurred` with `net::ERR_ABORTED`, and `onCompleted` never fires, so a transport that waits for `onCompleted` hangs on every redirecting submit.
Completion must be keyed on that pair instead.
The contrast is worth keeping beside it: under `redirect: "follow"` one request id spans both hops and ends on `onCompleted` normally, so the rule is specific to `manual` rather than general.
Both runs joined cleanly by request id and by URL, which is what attributing a `Location` to the request that produced it requires.

## 4. The payload path

Gate G3 drove a 256 MiB upload in 52 parts of 5 MiB from two hosts, and both passed; the per-run artefacts retained are `apps/extension-probe/g3-result-offscreen.json` and `apps/extension-probe/g3-result-tab.json`, and the seven-run record is `docs/research/rethink/oxichrome-extension/gates.md`.
The offscreen document completed in 42,258 ms at 6.06 MiB/s and the runner tab in 42,504 ms at 6.02 MiB/s, each moving all 268,435,456 bytes with the server confirming 52 parts received and no failure.
The offscreen document survived while the service worker was terminated at the documented 30-second idle mark and revived cleanly on the next message, which is the property the offscreen host was chosen for.

Holding the payload by value is not the constraint at this size.
The JavaScript heap reported 257 MiB used of 257 MiB total in every run, and the renderer holding the payload peaked between 311.3 and 317.4 MiB across the seven runs, measured from `/proc/<pid>/status` `VmRSS` sampled every half second (`docs/research/rethink/oxichrome-extension/gates.md`).
That is roughly 55 to 60 MiB of process overhead above the payload, with no evidence of a second copy being held anywhere, which is the specific worry `FileSource` returning bytes by value raises.
So `FileSource::fetch` returning `FileContent { bytes: Vec<u8> }` by value does not need a streaming variant to clear this gate, though that says nothing about behaviour under memory pressure.

Two findings shape the design rather than merely passing the gate.

An offscreen document cannot observe worker liveness itself.
`chrome.runtime.getContexts` is not a function inside an offscreen document on Chromium 151, and `SERVICE_WORKER` is not a valid `contextType` in any case, since the worker is `BACKGROUND`.
The offscreen run recorded the `TypeError` where the tab run recorded a context count, so an offscreen driver needs a server-side beacon for diagnostics while a runner tab can observe the worker directly.
That is a real argument for the tab variant that has nothing to do with throughput.

The case for hosting the driver outside the worker rests on the tail rather than on typical latency.
In the two committed runs the slowest part was 816 ms and 851 ms against a 30-second per-fetch limit, which is not close, and across the seven runs the client-observed per-part latency was 804 ms minimum, 812 ms median and 815 ms at the 95th percentile.
The exception is run `offscreen-2-restart`, which completed all 52 parts but took 70.8 s because a single part took 28,325 ms while its neighbours took 817 ms (`gates.md`).
Two things about that outlier have to be said together or it will be misread.
Its cause is not attributable from what was captured, it did not recur in the other six runs, and the sink is a single Python process sharing the machine with a sampler, so a host-side stall is the likelier explanation than a browser one.
And it landed 1.7 s under the 30-second per-`fetch()` limit that terminates a service worker, so had that part been driven from the worker rather than from the offscreen document the run would have been about two seconds from dying.
The evidence for hosting the driver outside the worker is therefore the existence of a tail that reaches the limit at all, not a claim that the browser produced it.

Memory-pressure closure of the offscreen document remains unmeasured, and it is the open item that would move the choice between the two hosts.

## 5. What is not verified

The Firefox leg is unverified and ships as such under decision 2.
Chromium accepts the dual background key and registers the worker; `background.scripts` with `type: module` has not been tested on Firefox, because Firefox is not served first and no Firefox was installed to test it.
Serving Firefox later means a second build, a second reviewer and a per-release source-submission obligation, for a market no incumbent in the category has found worth serving.

Nothing in this shell has touched a marketplace, and gate G1 — which decides whether the transport is a plain fetch or a tab-driving architecture — has not been answered here.
Until it is, the shell is a shell, and `transport.rs` demonstrates a bound rather than a design.

## 6. Chrome Web Store, unlisted

Review is completed "within a few days, but it can take up to a few weeks", and takes explicitly longer for extensions that "request broad host permissions or sensitive execution permissions", which this one has by construction (developer.chrome.com/docs/webstore/review-process).
The single-purpose policy warns that "excessive permissions unrelated to your extension's single purpose will be viewed as enabling unrelated functionalities", which is why the three enumerated host permissions matter as a submission property and not only as an engineering one (developer.chrome.com/docs/webstore/program-policies/quality-guidelines-faq).
Obfuscation is banned outright while minification is expressly allowed, and the code-readability policy does not mention WebAssembly or compiled binaries at all, so whether our wasm reads as concealing functionality is undetermined by published policy and rests on reviewer discretion (developer.chrome.com/docs/webstore/program-policies/code-readability).
That undetermined question is the whole reason gate G4 exists and the reason it should start before the other gates, since its latency is someone else's queue.

The submission sequence, following developer.chrome.com/docs/webstore/publish:

1. Run `just extension-build` and take `apps/extension/build/tam-extension.zip`.
2. Sign in to the Chrome Web Store developer dashboard with the account from the checklist in section 8, and add a new item.
3. Upload the zip. The dashboard reads `manifest.json` for the name, version and permissions, so nothing about the package is retyped.
4. Complete the store listing, the privacy disclosures and the permission justifications. Each requested permission needs a justification naming why the single purpose requires it, and the host permissions need one that matches the three enumerated origins.
5. Set visibility to unlisted, so the item is installable by link and does not appear in search.
6. Submit for review, and expect days to weeks.

Self-hosting outside the store is not a fallback worth planning around: it works on Linux only, through `update_url` and an `updates.xml` gupdate manifest (developer.chrome.com/docs/extensions/how-to/distribute/host-on-linux).

## 7. Mozilla add-ons, unlisted signing

Firefox requires Mozilla signing for release and beta; unsigned extensions install only in Developer Edition, Nightly and ESR after toggling `xpinstall.signatures.required`.
Unlisted self-distribution is supported through the Developer Hub, `web-ext sign` or the signing API, and "it can take up to 24 hours for your submission to be signed and published, or longer if your submission is selected for manual review".
Unlisted is not unreviewed: "all add-ons, including self-distributed ones, are subject to be manually reviewed at any time after submission" (extensionworkshop.com/documentation/publish/signing-and-distribution-overview/).

The source-code rule is the concrete obligation a wasm build creates.
Add-ons "may contain transpiled, minified or otherwise machine-generated code, but Mozilla needs to review a copy of the source code", with build instructions, environment and tool versions, the full command list and lockfiles, sufficient for a reviewer "to rebuild your extension from the source code" (extensionworkshop.com/documentation/publish/source-code-submission/ and the add-on policies).
WebAssembly is not named in the trigger list, but "any other custom tool that takes files, applies pre-processing, and generates file(s)" plainly covers a Rust-to-wasm toolchain.
Non-compliance risks rejection, delay, "or, in the worst-case, result in your extension being taken down".

`just extension-source-package` produces exactly that archive, and it is self-sufficient rather than a pointer at this repository.
It carries the extension crate, a generated minimal workspace root, this repository's `Cargo.lock` unchanged, `rust-toolchain.toml`, a flake pinning the same nixpkgs and rust-overlay revisions, the deterministic packer, and a `BUILD.md` recording the exact rustc, cargo, wasm-bindgen and python versions the submitted build used.
The justfile it contains is extracted from the real one rather than retyped, so the commands a reviewer runs cannot drift from the commands we run.
The generated workspace root copies the `[profile.*]` tables as well as the lints, which matters more than it sounds: `[profile.release]` turns on `overflow-checks` and `debug-assertions`, and an earlier version of the generator omitted them and had a reviewer building a materially different program.

The submission sequence:

1. Run `just extension-build` and `just extension-source-package`.
2. Sign in to addons.mozilla.org, go to the Developer Hub and submit a new add-on.
3. Choose "On your own" for distribution, which is the unlisted, self-distributed channel.
4. Upload `apps/extension/build/tam-extension.zip`. Automated validation runs before signing.
5. When asked whether the add-on requires source code, answer yes and upload `apps/extension/build/tam-extension-source.zip`. This is not optional for a wasm build.
6. Wait for signing, up to 24 hours or longer under manual review, then download the signed `.xpi`, which is the file that installs.

Any reviewer question about reproducing the artefact is answered by `BUILD.md`: `nix develop --command just extension-build`, then compare the SHA-256 the packer prints against the submitted file.

## 8. The founder checklist

These are accounts, fees and secrets that cannot be created by an agent and must be set up personally.
None of them is needed before the gates answer, and none should be bought until the extension is a commitment rather than a candidate.

A Chrome Web Store developer account, registered with a Google account the business controls rather than a personal one.
Registration carries a one-time developer fee; the amount is stated at signup and is not recorded here, because no local source verifies it.
Decide before registering whether the publisher is an individual or a verified organisation, since changing it later is not a settings toggle.

A Mozilla add-ons account on addons.mozilla.org, likewise on a business-controlled address.
There is no fee.
This is needed only if Firefox is served, which decision 2 defers.

The extension id and its key.
An unpacked extension's id is derived from the absolute path of its directory, which is why the id in section 2 changes if the directory moves, and a packed extension's id is derived from its signing key instead.
The Chrome Web Store assigns the id and holds the key on first upload, so the id is stable from then on and no key needs generating in advance.
Where a stable id is needed before the first upload — for testing an allowlist, or for anything that names the extension by id — that requires a `key` in the manifest generated from a keypair the founder creates and keeps, and it must never be committed.
Deciding whether the pre-upload stable id is needed is a founder call, and doing nothing is the right default until something actually requires it.

A decision about which account owns each store listing, because both stores tie ownership to the account that submits and transferring later is friction.

Two of the gates are founder actions and start only on the founder's word: G1 is a live request against TPT on the founder's own account and machine, and G4 is a store submission.
Nothing in this note has performed either.
