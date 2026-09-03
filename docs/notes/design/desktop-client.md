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

## Startup diagnostics

The release binary sets `windows_subsystem = "windows"` (`src/main.rs`), so on Windows it has no console and everything it writes to stderr is discarded.
A failure before the window appears would therefore show as an application that opens and closes with nothing to read, on a platform no one here can attach a debugger to.
`src/startup.rs` gives that failure a file: `startup.log`, in the application data directory, truncated to one `starting <version> <os> <arch>` line on every launch and appended to by the two failure paths.
Nothing has yet failed this way; the section exists so that the first time something does, the evidence is already on disk rather than a Windows build away.

Where the log is, by platform:

| Platform | Path |
|---|---|
| Windows | `%APPDATA%\io.teachouse.desktop\startup.log` |
| Linux | `$XDG_DATA_HOME/io.teachouse.desktop/startup.log`, or `~/.local/share/io.teachouse.desktop/startup.log` |
| macOS | `~/Library/Application Support/io.teachouse.desktop/startup.log` |

That mapping is `PathResolver::app_data_dir`, which is `dirs::data_dir()` joined with the `identifier` field of `tauri.conf.json` — `io.teachouse.desktop` (tauri 2.11.5, `src/path/desktop.rs:247`).
`dirs` 6.0.0 resolves `data_dir` to `FOLDERID_RoamingAppData` on Windows (`src/win.rs:10`), to `$XDG_DATA_HOME` or `$HOME/.local/share` on Linux (`src/lin.rs:11`), and to `$HOME/Library/Application Support` on macOS (`src/mac.rs:12`).
Windows reads the known folder rather than the `%APPDATA%` environment variable; the two agree unless someone has overridden the variable, and the log records the directory it actually used.

Three outcomes, and each says something different.
A log holding the opening line and a failure block names the cause outright.
A log holding only the opening line means the process died after the application was built without reaching either failure path, which is a crash rather than an error — a faulting native library rather than a Rust panic.
A missing or empty log means it died before the data directory resolved: either the executable never started, or `app_data_dir` itself failed.

To capture stderr and the exit code from PowerShell, redirect through `cmd`, because PowerShell's own operators give a GUI-subsystem process nothing to inherit:

```powershell
cmd /c "Teachouse.exe 2> err.txt"
echo $LASTEXITCODE
```

Run it from the directory the installer wrote (the Start-menu shortcut's target names it), and set `$env:RUST_BACKTRACE = "1"` first so the report carries a backtrace.
Exit code 101 is a Rust panic; the failure block in the log is then the same text the discarded stderr would have carried.

The likely causes, in the order to check them:

The WebView2 runtime is absent or broken.
It is the one Windows dependency with no Linux analogue, and Tauri builds the window declared in `tauri.conf.json` before it calls this crate's `setup` closure, in the same function whose failure it raises as a panic (`src/app.rs:2524` and `src/app.rs:1424`).
This is why the opening line is written between `build` and `run` rather than inside `setup`: written from `setup` it would come too late to record this cause at all.
The NSIS bundle installs the runtime through the default `downloadBootstrapper`, which needs network access at install time and silently leaves an installation without it.

A missing Visual C++ runtime DLL.
The Windows binary is cross-compiled from NixOS through `cargo-xwin`, and `ring` reaches it as C compiled by `clang-cl` against the MSVC runtime; a machine without the redistributable fails in the loader, before `main`.
The signature is the empty case above — no log at all — plus a Windows loader dialog.

`device.json` unreadable or unparseable, in the same directory as the log.
A truncated write from an earlier crash, or a roaming profile mid-sync, makes `device::load_or_create` fail, and the failure block names the file.
Deleting it costs the device its identity and forces a re-registration, which is why the client never does so itself.

`app_data_dir` unresolvable, which is the missing-log case with the executable confirmed to have started.

Two things that look like candidates and are not.
The updater's placeholder public key is never parsed at startup: the plugin deserialises `endpoints` and `pubkey` and checks only that the endpoints are `https` (tauri-plugin-updater 2.11.0, `src/config.rs`), and the key is read when a check runs, which nothing does yet.
A bundle built without `npm run build` opens a blank window rather than closing, because `build.rs` creates an empty `web/build` and `generate_context!` embeds it.

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
macOS, Android and iOS are not built; D2 defers them, and the mobile targets additionally need a `crate-type` change for the mobile entry point.
Corrected 2026-09-03: this sentence also claimed a hand-written Tauri plugin for Android cookie access, and that is not true of the versions this tree resolves.
`wry` 0.55.1 implements `cookies_for_url` on Android (`src/android/mod.rs:423`) by calling the Kotlin method `RustWebView.getCookies(url)` (`src/android/main_pipe.rs:457`), whose body is `CookieManager.getInstance().getCookie(url)` (`src/android/kotlin/RustWebView.kt:90`) — the platform's own store, so HttpOnly cookies are included.
What is genuinely absent there is `cookies()`, the all-URLs read, which returns an empty vector, and `set_cookie` and `delete_cookie`, which are no-ops; none of the three is called by this crate.
`docs/notes/design/android-client.md` carries the reading and its consequences.

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

## Four findings on the dependency policy, and the decisions taken

The dependency policy is founder-gated, and adding Tauri moved it in four ways that no build here could resolve on its own.
All four were decided on 2026-09-03 by founder-delegated decision, and what follows records each finding as measured and then the decision taken on it.

The workspace dependency graph doubles, from 342 packages to 684.
Much of the addition is target-gated and never compiles on Linux — the `windows_*`, `objc2-*`, `ndk` and `jni` families — but every one of them is in `Cargo.lock` and therefore inside `cargo deny` and `cargo audit`'s scope.

`cargo deny check licenses` now fails on seven crates.
Five are MPL-2.0: `cssparser`, `cssparser-macros`, `dtoa-short` and `selectors` arrive through `dom_query` under `tauri-utils`, and `option-ext` through `dirs`.
`target-lexicon` is Apache-2.0 WITH LLVM-exception, through `cfg-expr` and `system-deps` under the GTK bindings.
`webpki-root-certs` is CDLA-Permissive-2.0, through `rustls-platform-verifier` under `reqwest`, and is the same licence and the same reason as the `webpki-roots` exception `deny.toml` already carries.
That is now decided, on 2026-09-03, by founder-delegated decision.
`deny.toml` carries six of the seven as scoped per-crate `[[licenses.exceptions]]` entries, in the greppable form that file already uses.
`target-lexicon`'s expression joins the allow list instead, because `Apache-2.0 WITH LLVM-exception` only relaxes an entry already there.
What the exceptions record is the distinction between build-time and shipped.
`cssparser`, `cssparser-macros`, `dtoa-short` and `selectors` reach the graph only under `tauri-build`, a build dependency, so they compile during the build and ship in no artefact.
Measured with `cargo tree -e normal,no-proc-macro`, no workspace binary reaches any of them.
`option-ext` does ship, unmodified, in the desktop binary through `dirs` under `tauri`, which makes it the only copyleft-licensed code in anything we distribute.
MPL-2.0 is file-level copyleft, and the unmodified upstream source already published on crates.io discharges it; modifying the crate would oblige us to publish the modified files.

`cargo deny check bans` now fails on one entry, and this one is worth reading closely rather than waiving.
`serde_json`'s `unbounded_depth` feature is denied because, with it off, the method that disables the 128-deep parse limit does not exist at all.
It is now enabled, through exactly one path: `cargo_metadata`, an optional dependency of `tauri-utils` turned on by its `build` feature.
That path reaches `serde_json` only through `tauri-macros`, a proc-macro crate, and under `resolver = "2"` proc-macro features are not unified into the target build, so the `serde_json` linked into a shipped binary should not carry it.
`cargo deny` reads one unified feature set out of `cargo metadata` and does not model that separation, so the check fails regardless.
That is now decided, on 2026-09-03, by founder-delegated decision.
The `[[bans.features]]` entry left `deny.toml` for the `deny` recipe in the justfile, which can ask the question `deny.toml` cannot.
For every workspace member carrying a binary target, taken from `cargo metadata` so that a new binary needs no edit, the recipe runs `cargo tree -e normal,no-proc-macro` and fails, naming the crate, if `serde_json` appears in that graph with `unbounded_depth`.
Measured on the day of the decision, all fourteen carry `default,raw_value,std`, and the desktop binary `alloc` besides, so the separation above holds and nothing we ship can disable the depth limit.
The bound is now enforced against the runtime graph rather than against the unified feature set.

`cargo deny check advisories` fails on sixteen crates, a fourth finding this note did not anticipate and the doubling above explains.
All sixteen are unmaintained notices rather than vulnerabilities, and all sixteen are Tauri v2's Linux stack.
Ten are the gtk-rs 0.18 bindings, RUSTSEC-2024-0411 to 0420, which bind GTK3, the only GTK Tauri v2 supports; `proc-macro-error` and the five `unic` crates are transitive under that same chain.
That is now decided, on 2026-09-03, by founder-delegated decision, and `deny.toml` ignores those sixteen advisory ids under a single comment carrying this reason.
Nothing is softened in general: an unmaintained crate outside the list still fails the lane, and a new advisory against any of these crates would carry an id that is not on it.
The vulnerability policy is untouched and could not be softened even deliberately, because cargo-deny 0.20 removed the key that once allowed it.
Revisit when Tauri adopts GTK4.

## Sources

`docs/notes/design/client-side-architecture.md`, sections 5 to 8.
`docs/notes/design/vendoo-for-teachers-rethink.md`, decisions D1, D2, D10, D11, D12, D14, D27, D29 and D30.
`docs/research/rethink/client-surfaces-and-cross-compile.md`, sections 4.1 and 5.1.
`tools/login-probe/README.md`, for the webview stack, the Linux nix-shell recipe and the Windows cross-compile route.
`crates/tam-marketplace-tpt/src/session.rs` and `crates/tam-marketplace-tes/src/session.rs`, for the cookies that constitute a session.
`crates/tam-api/src/auth.rs`, for the EdDSA verification this crate reuses.
