# The desktop client

The first landable slice of Phase 2: a Tauri v2 application that hosts the existing SvelteKit console and owns the seller's marketplace sessions on the seller's own device.

- date: 2026-09-03
- status: built and green under `just check`; no marketplace request is made anywhere in it
- decisions it implements: D1 (two-branch automation), D2 (Windows desktop first, Tauri v2), D10 and D11 (the entitlement-token exception and its latency), D12 (the webview probe that gated this surface), D14 (per-surface login and the device registry), D27, D29 (build infrastructure), D30 (the user-facing wording)

## What exists

The crate is `apps/desktop/src-tauri`, package `tam-desktop`, binary `teachouse`, bundle identifier `io.teachouse.desktop`.
It is a workspace member, so it is compiled, linted and tested by `just check` alongside everything else.
The console is the frontend: `frontendDist` is the SvelteKit static build at `web/build` and `devUrl` is the SvelteKit dev server, so there is one console and the desktop client hosts it rather than reimplementing it.

Six modules carry the slice.
`connect` holds the login page and the logged-in condition for each marketplace, and refuses outright for any marketplace whose transport class is `OfficialApi`.
`session` holds the cookie jar, the session record, and the `SessionStore` trait with a keychain implementation and an in-memory one.
`device` holds the device identity.
`entitlement` verifies the signed token and answers the gate.
`scheduler` holds the local timer and the `WorkSource` seam.
`commands` and `state` are the Tauri wiring: three commands and what the running application holds.

The two-branch rule is structural here rather than remembered.
`connect::login_target` returns a `NotSellerDevice` refusal for Etsy, and a test walks every value of `Marketplace`, asserting that a login target exists for exactly those whose `transport_class()` is `SellerDevice`.
Adding a marketplace therefore forces the decision at compile time, and getting it wrong fails a test rather than shipping a login window for a marketplace whose automation the server should be running under a sanctioned token.

## The session record

A session is captured by opening the marketplace's own login page in a second webview window on the seller's machine, watching that window's cookie store from Rust until the marketplace's logged-in condition holds, and filing what it finds in the operating system's keychain.

The record is:

```
SessionRecord {
    marketplace:   Marketplace,
    account_label: Option<String>,
    captured_at:   Timestamp,        // milliseconds, as tam_types spells it
    device_id:     DeviceId,
    jar:           CookieJar,
}
```

`account_label` is always `None` today, and that is a consequence rather than an omission.
The only value that names the account is the marketplace's own identity read — `read_seller_store_id` on TPT and `read_seller_user_id` on Tes — and that is a marketplace request, which this slice makes none of.

The jar never reaches a formatter.
`CookieJar`'s `Debug` prints `CookieJar(2 cookies, redacted)` and nothing else, not even the cookie names, and `Cookie`'s own `Debug` redacts as well, so the derive on `SessionRecord` stays safe as fields are added.
A test asserts that neither a cookie name nor a cookie value appears in the record's `Debug` output.
The status type the interface reads, `SessionStatus`, is a separate struct with no field a jar could travel in, so the command surface cannot leak one by a filtering mistake.

The logged-in condition is read off the jar rather than off the page, because the jar is what the adapter will authenticate with.
For TeachersPayTeachers it is the `csrfToken` cookie plus one of `sessionKey` or `TPT`: the CSRF cookie is not optional, because `TptSession` refuses a jar without it and the adapter mirrors its value into `x-csrf-token`, and it alone proves nothing because it is handed out before login too.
For Tes it is `TESSession`.
Both conditions are first-contact heuristics drawn from the committed cassette fixtures, the broker's vault tests, and the M7 plan's wire reading; neither has been verified against a live login, and the first live run may move them.

Reading the cookies from Rust rather than from the page is the point of hosting the login at all.
`document.cookie` cannot see an HttpOnly cookie, and the session cookie is HttpOnly on both marketplaces.

## The device identity

A version-4 UUID written once into the application data directory as `device.json`, labelled with the hostname read through `tauri-plugin-os`.
Not a hardware or machine UUID: D14 records that `machine-uid` covers neither Android nor iOS, and D2 puts both on the roadmap.
The label is refreshed from the current hostname on every read while the identifier is not, because a renamed machine is the same device and re-registering it would strand every entitlement token bound to the old identifier.

## The entitlement token

A compact JWS with `alg: EdDSA` over Ed25519, verified against a key compiled into the binary.
The format and the crate are the ones `crates/tam-api/src/auth.rs` already uses for better-auth's assertions, because a second signature stack would be a second thing to get wrong.

```
{
  "sub":          "<account id>",
  "aud":          "tam-desktop",
  "iss":          "tam-server",
  "device":       "<device id, unhyphenated UUID>",
  "marketplaces": ["Tpt", "Tes"],
  "exp":          <seconds since the epoch: revalidation is due>,
  "grace":        <seconds since the epoch: work stops here>
}
```

Audience and issuer are fixed in code rather than configured, because a build able to widen either would accept a token minted for something else.
A token naming a different device is refused, so a token copied to a second machine does not work there.
A `grace` earlier than `exp` is refused as malformed rather than treated as a very short grace.

Expiry is decided by the gate rather than by the JWT library, and `validate_exp` is deliberately off: a token past `exp` but inside `grace` is still workable under D11, and the library would refuse it outright and collapse the grace window to nothing.
`exp` and `grace` are seconds, because RFC 7519 fixes `exp` to be, while `tam_types::Timestamp` is milliseconds; every comparison converts, and an overflow in that conversion answers "refuse".

`EntitlementGate::may_work(marketplace, now)` fails closed by construction.
No token is no, a marketplace the token does not name is no, and past the grace deadline is no.
That is the whole of the client-side enforcement, and it is advisory: D10 keeps Postgres the decision-maker, the token only transports a decision Postgres already made, and the server re-checks on every control-plane call regardless.
The per-marketplace grant set is the kill switch, and dropping one marketplace from it answers a cease-and-desist across the installed fleet within one revalidation window with no shipped update.

The compiled-in key, `entitlement::EMBEDDED_PUBLIC_KEY`, is thirty-two zero bytes.
That is not a valid Ed25519 point, so a build carrying it verifies nothing and every gate answers no, which is the correct behaviour for a placeholder.
A test asserts it.

## The capability list

The capability set is deliberately short, and the login window is deliberately absent from it.

`capabilities/default.json` names the `main` window only and grants `core:event:default`, `core:window:default`, `os:allow-hostname` and `updater:default`.
Commands the application defines itself are not permission-gated in Tauri v2, so the three below need no entry.

The marketplace page gets no capability at all.
That is the constraint section 7 of `client-side-architecture.md` records: dynamic per-webview capability scoping is unreliable, and the site's own content-security policy blocks Tauri's `ipc.localhost` protocol anyway, so the marketplace origin is never given an IPC surface to begin with.
Everything the application learns about a login it learns by reading the webview's cookie store from Rust.

The three commands are `connect_marketplace(marketplace)`, `session_status(marketplace)` and `forget_session(marketplace)`.
All three are `async`, which is required rather than stylistic: `cookies_for_url` deadlocks on Windows when called from a synchronous command or an event handler.

## The build recipes

```
just desktop-dev            # the client against a running `just web-dev`
just desktop-build          # the Linux bundle
just desktop-build-windows  # the Windows NSIS installer, cross-compiled
```

Measured artefacts, 2026-09-03, from this tree:

| Artefact | Size |
|---|---|
| `target/release/teachouse` (Linux binary) | 16.1 MB |
| `Teachouse_0.1.0_amd64.deb` | 5.2 MB |
| `target/x86_64-pc-windows-msvc/release/teachouse.exe` | 11.7 MB |
| `Teachouse_0.1.0_x64-setup.exe` (NSIS) | 3.1 MB |

The Windows cross-compile works from NixOS through `cargo-xwin`, which is D29's local route, but not with the toolchain the login-probe recipe used.
That probe has no C dependencies; this crate reaches `ring` through both `jsonwebtoken` and `rustls`, and `ring` builds C.
nixpkgs' `cc-wrapper` is not multi-target aware: it reads `cargo-xwin`'s MSVC-style `/imsvc` include flags as filenames and adds `-fPIC`, which `clang-cl` rejects for a windows-msvc target, and `cc-rs` then also needs `llvm-lib`, which the wrapper does not supply.
The fix is an unwrapped clang, `lld` and `llvm` prepended for exactly one command.
They are kept off the dev shell's `PATH`, because an unwrapped clang ahead of the wrapper breaks every native C build in the tree; the flake exports `TAURI_WINDOWS_TOOLCHAIN_BIN` instead and the recipe prepends it.
The installer is unsigned, and Tauri warns about both that and cross-compilation being experimental; D29 already reserves the MSI and the signing step for a GitHub Windows runner, and this recipe is the local one.

The dev shell gained `cargo-tauri`, `cargo-xwin`, `nsis`, `pkg-config`, and, as `buildInputs`, `gtk3`, `webkitgtk_4_1`, `libsoup_3` and `glib-networking`.
`GIO_EXTRA_MODULES` is exported for the same reason the login probe needs it: WebKitGTK takes TLS from a GIO module no setup hook adds, and without it every `https` navigation lands on "TLS support is not available" rather than on the login page.

`build.rs` creates `web/build` if it is absent.
This is not incidental.
`tauri::generate_context!` panics outright when `frontendDist` does not exist, and it looks for it whenever the `custom-protocol` feature is on, which is what `just check`'s `--all-features` does; the console's build output is a gitignored npm artefact, so without this a clone that had not yet run `just web-check` could not run the gated lane at all.
An empty directory embeds nothing and satisfies the check, and the bundle recipes build the console before they bundle anything.

## What is stubbed

No marketplace request is made anywhere in this slice, on any path.

The scheduler ticks on a configurable cron-shaped cadence, hourly by default, consults the entitlement gate per marketplace, and calls a `WorkSource`.
The only implementation of `WorkSource` is `NoWork`, which returns nothing to do.
The gate is consulted before the work source is reached rather than after, and a test asserts the work source is never called for a marketplace the gate refuses, because a gate that refuses after the call has gone out is not a gate.
The engine driver split that gives `WorkSource` a real implementation is a separate stream.

The entitlement gate always answers no in a shipped build today, because nothing sets it: there is no check-in, the server that mints tokens is a later stream, and the compiled-in key is a placeholder.
Clock-tamper detection is not implemented; the gate reads the wall clock and believes it.

The updater is configured but cannot update anything.
Its endpoint is the CrabNebula format with literal `ORG` and `APP` placeholders, its public key is the string `PLACEHOLDER_FOUNDER_SUPPLIES_THIS`, and `createUpdaterArtifacts` is `false`.
The icons are a generated placeholder set, not artwork.
macOS, Android and iOS are not built; D2 defers them, and the mobile targets additionally need a hand-written Tauri plugin for Android cookie access and a `crate-type` change for the mobile entry point.

## What the founder must supply

Four things gate a real release, and none of them can be inferred.

The updater signing keypair, generated with `cargo tauri signer generate`.
Its public half replaces `plugins.updater.pubkey` in `tauri.conf.json` and its private half becomes `TAURI_SIGNING_PRIVATE_KEY` in CI; `createUpdaterArtifacts` then becomes `true`.
This key is irreplaceable: lose it and the installed base can never be updated again.

The CrabNebula organisation and application slugs, which replace `ORG` and `APP` in the updater endpoint.

The Windows code-signing certificate, roughly $120 a year through Azure Trusted Signing, capped at one year since December 2025, with EV certificates no longer bypassing SmartScreen.
Signing runs on the Windows runner, not here.

The Ed25519 entitlement keypair.
Its public half, thirty-two raw bytes, replaces `EMBEDDED_PUBLIC_KEY`; its private half signs tokens on the server.

## Three findings that need a founder decision

The dependency policy is founder-gated, and adding Tauri moves it in three ways that no build here may resolve on its own.

The workspace dependency graph doubles, from 342 packages to 684.
Much of the addition is target-gated and never compiles on Linux — the `windows_*`, `objc2-*`, `ndk` and `jni` families — but every one of them is in `Cargo.lock` and therefore inside `cargo deny` and `cargo audit`'s scope.

`cargo deny check licenses` now fails on seven crates.
Five are MPL-2.0: `cssparser`, `cssparser-macros`, `dtoa-short` and `selectors` arrive through `dom_query` under `tauri-utils`, and `option-ext` through `dirs`.
`target-lexicon` is Apache-2.0 WITH LLVM-exception, through `cfg-expr` and `system-deps` under the GTK bindings.
`webpki-root-certs` is CDLA-Permissive-2.0, through `rustls-platform-verifier` under `reqwest`, and is the same licence and the same reason as the `webpki-roots` exception `deny.toml` already carries.
Each would need its own scoped `[[licenses.exceptions]]` entry, in the greppable per-crate form that file already uses.

`cargo deny check bans` now fails on one entry, and this one is worth reading closely rather than waiving.
`serde_json`'s `unbounded_depth` feature is denied because, with it off, the method that disables the 128-deep parse limit does not exist at all.
It is now enabled, through exactly one path: `cargo_metadata`, an optional dependency of `tauri-utils` turned on by its `build` feature.
That path reaches `serde_json` only through `tauri-macros`, a proc-macro crate, and under `resolver = "2"` proc-macro features are not unified into the target build, so the `serde_json` linked into a shipped binary should not carry it.
`cargo deny` does not model that separation, so the check fails regardless.
The options are a scoped exclusion in `deny.toml`, accepting a red lane, or rejecting the dependency; the first two are founder decisions and the third would end this surface.

## Sources

`docs/notes/design/client-side-architecture.md`, sections 5 to 8.
`docs/notes/design/vendoo-for-teachers-rethink.md`, decisions D1, D2, D10, D11, D12, D14, D27, D29 and D30.
`docs/research/rethink/client-surfaces-and-cross-compile.md`, sections 4.1 and 5.1.
`tools/login-probe/README.md`, for the webview stack, the Linux nix-shell recipe and the Windows cross-compile route.
`crates/tam-marketplace-tpt/src/session.rs` and `crates/tam-marketplace-tes/src/session.rs`, for the cookies that constitute a session.
`crates/tam-api/src/auth.rs`, for the EdDSA verification this crate reuses.
