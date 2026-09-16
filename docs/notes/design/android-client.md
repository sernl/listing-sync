# The Android client

Phase D2's second surface: the same Tauri v2 application, the same SvelteKit console, built for a phone, where D3 limits it to work the seller starts.

- date: 2026-09-03
- status: the debug and release APKs both build here and are measured; nothing is signed, nothing has run on a device, and nothing has been uploaded anywhere
- decisions it implements: D2 (Windows desktop first, then Android, all Tauri v2), D3 (a phone runs user-initiated work only for the no-API branch), D12 (the webview login probe), D14 (per-surface login and the device registry), D29 (Android builds from NixOS through `androidenv`, and the first Play upload is by hand)
- companions: `docs/notes/design/desktop-client.md` is the client this one is a build of, and `docs/notes/design/desktop-distribution.md` is the Windows pipeline this one parallels

Every claim below names the source it was read from and the date it was read.
Where a source in this repository disagrees with what the code now says, both are quoted and the disagreement is stated rather than resolved silently.

## What Tauri v2 Android needs

Four rust targets, not one: `aarch64-linux-android`, `armv7-linux-androideabi`, `i686-linux-android` and `x86_64-linux-android`.
Source: <https://v2.tauri.app/start/prerequisites/>, fetched 2026-09-03.
`rust-toolchain.toml` carries only the first of the four today, because `just check-portable` proves one triple per surface rather than one per ABI; a build needs all four, or an explicit `--target` list narrowing the bundle.

A JDK, an Android SDK platform, the SDK build-tools, the platform-tools, the command-line tools and an NDK, reached through `JAVA_HOME`, `ANDROID_HOME` and `NDK_HOME`.
The prerequisites page routes all of that through an Android Studio install and its bundled JetBrains Runtime, and offers no command-line-only alternative for Android the way it does for macOS desktop work.
D29 rules that out here and routes it through `androidenv` instead, which is the same set of packages without the IDE.

A `cdylib` from the library crate, and a mobile entry point.
`apps/desktop/src-tauri/Cargo.toml` declares `[lib] name = "tam_desktop"` with no `crate-type`, so it builds an rlib and nothing an Android `System.loadLibrary` could open.
The entry point is `tauri::mobile_entry_point`, which exists in `tauri-macros` 2.6.3 and is re-exported from `tauri` 2.11.5 (`src/lib.rs:79`), and is applied as `#[cfg_attr(mobile, tauri::mobile_entry_point)]` on `run`; that half is done.
The `crate-type` half is one additive line in the crate's manifest, `crate-type = ["staticlib", "cdylib", "rlib"]`, and without it Gradle has no `libtam_desktop.so` to package and there is no APK at all.

`cargo tauri android init`, which writes `apps/desktop/src-tauri/gen/android`: a Gradle project, an `AndroidManifest.xml`, the `WryActivity` subclass and the `build.gradle.kts` the signing config is edited into.
The minimum supported Android version is 7.0, SDK 24, raisable through `bundle.android.minSdkVersion` in `tauri.conf.json`, and the version code is derived as `major*1000000 + minor*1000 + patch` from the configuration's `version` unless `bundle.android.versionCode` overrides it.
Source: <https://v2.tauri.app/distribute/google-play/>, fetched 2026-09-03.

## What the desktop code assumes that a phone lacks

Four assumptions, and they are not equally hard.
One is already false, one is a founder-gated dependency, one is a product rule rather than a technical gap, and one is very likely fine.

### The login webview, which is the one that turns out to work

`docs/notes/design/desktop-client.md` records under "What is stubbed" that the mobile targets "additionally need a hand-written Tauri plugin for Android cookie access".
That is no longer true of the versions this tree resolves, and the correction matters because it is the difference between a fortnight of Kotlin and nothing at all.
`wry` 0.55.1 implements `cookies_for_url` on Android by sending `WebViewMessage::GetCookies` to the main pipe (`src/android/mod.rs:423`), which calls the Kotlin method `RustWebView.getCookies(url)` (`src/android/main_pipe.rs:457`), whose whole body is `CookieManager.getInstance().getCookie(url)` (`src/android/kotlin/RustWebView.kt:90`).
That is the platform's own cookie store, so the string it returns is the `Cookie` header the WebView would send, HttpOnly cookies included — which is exactly the property `connect.rs` depends on and `document.cookie` cannot give.
Read from the vendored crate sources at `~/.cargo/registry`, 2026-09-03.

Three limits come with it, and each has a consequence here.
`cookies()`, the all-URLs read, returns an empty `Vec` on Android and `tauri` 2.11.5 documents it as unsupported there (`src/webview/mod.rs:2170`); nothing in this crate calls it, so nothing is lost.
`set_cookie` and `delete_cookie` are no-ops on Android (`src/android/mod.rs:432` and `:437`), so a revocation wipe can clear our own keychain entry but cannot clear the WebView's copy; `clear_all_browsing_data` is the only lever, and it is all-or-nothing across every origin the app has visited.
`CookieManager` is process-global rather than per-webview, so on Android there is one cookie store shared by the console and by any marketplace page the app loads, and `cookies_for_url` returns the same answer whichever webview is asked.

That last property is what makes the second window unnecessary.
`commands::connect_marketplace` builds a second `WebviewWindowBuilder` today, and Tauri's mobile surface is a single Activity; rather than gamble on a second window existing, the Android path navigates the one webview to the login page, polls `cookies_for_url` against the marketplace origin, and navigates back to the console when the logged-in condition holds.
Whether a second window would in fact build on Android is left open rather than asserted, because settling it needs a device and settling it changes nothing about the design above.

### The OS keychain, which a sealed file replaces

`keyring` 3.6.3 has no Android backend.
Its platform blocks cover Linux, FreeBSD, OpenBSD, macOS, iOS and Windows and stop there (`src/lib.rs:207` to `:293`), and `Cargo.toml` here declares `keyring` only under those same three target predicates, so on `*-linux-android` the crate is not a dependency at all and `session/keychain.rs` does not compile.
`SessionStore` is already a trait with two implementations, so the seam exists; what was missing is a third.

Three candidates were weighed.
A Kotlin plugin over `androidx.security.crypto`'s `EncryptedSharedPreferences`, the platform-blessed route, costing a hand-written Tauri plugin, a Gradle dependency and a JNI surface to keep true.
`tauri-plugin-stronghold`, an official Tauri plugin, pure Rust and identical on every platform this product targets, costing one more crate and a decision about where its password comes from.
The application's own private files directory with no encryption at all, which on a non-rooted device is already unreadable by other applications and is what `payload.rs` and `device.json` use regardless.

Decided 2026-09-03, in two steps, and the second step reversed the first.
Stronghold was recommended and taken, then measured and dropped.
Measuring it is what settled it: `tauri-plugin-stronghold` 2.3.2 adds sixty-seven packages to `Cargo.lock`, and four of them need exceptions in `deny.toml`, a shared gate.
Two are licence exceptions — `constant_time_eq` 0.1.5 and `tiny-keccak` 2.0.2 on CC0-1.0 — and two are advisory ignores for unmaintained crates, RUSTSEC-2025-0141 against `bincode` 1.3.3 and RUSTSEC-2024-0436 against `paste` 1.0.15, all four arriving under `iota_stronghold` 2.1.0.
Sixty-seven packages and four gate exceptions, two of them unmaintained, underneath the module that holds the seller's marketplace session is the wrong trade when the device already supplies the secrecy: the Android Keystore holds the key either way, and Stronghold's snapshot encryption would be a second cipher over the same secret.

What replaced it costs nothing new.
`crates/tam-secrets` already seals bytes for the server's credential vault, and it is exactly the primitive this needs: XChaCha20-Poly1305 with a random per-record data key wrapped under a caller-supplied thirty-two-byte key-encryption key, a random 192-bit nonce at both layers, and additional authenticated data bound through both.
`Kek::from_bytes` refuses any length but thirty-two with a named error, `Sealed`'s `Debug` redacts every field, and the type is `Zeroize`.
Reaching it is a workspace path edge rather than a dependency: `chacha20poly1305` 0.10.1 and `aead` 0.5.2 are already in the lock through that crate, so the package count does not move, and `cargo check -p tam-secrets --target aarch64-linux-android` finishes clean.

The store is `session/encrypted.rs`, and nothing in it is Android-specific.
One file under the application data directory holds one sealed envelope per marketplace, filed under the same `entry_key` the keychain store uses, with the record as JSON inside the envelope and that same key as the additional authenticated data, so an envelope moved to another marketplace's entry fails to authenticate rather than opening as the wrong session.
A file that exists but does not parse is an error rather than an empty store, because answering a damaged file with "no sessions" would overwrite what might still be recoverable on the next write.

The password is not typed by the seller.
It is a thirty-two-byte secret generated once, wrapped with an AES-GCM key held in the Android Keystore, and persisted as a wrapped blob in the application's private files directory.
It reaches the store through a two-method port, `DeviceKeySource`, whose `obtain` returns a `Kek` — the `ZeroizeOnDrop` type, so the secret does not outlive the call that used it — and whose `forget` destroys the key itself.
That second method is a whole-device wipe and `SessionStore::forget` never calls it, because forgetting one marketplace must not make the others unreadable; what makes a wipe a fact rather than a deletion the filesystem might not have honoured is that the key is gone, so anything that survived the file cannot be read.
The port's Android implementation is one Kotlin class registered from this crate through `PluginApi::register_android_plugin` and reached with `PluginHandle::run_mobile_plugin` (tauri 2.11.5, `src/plugin/mobile.rs:208` and `:324`), which needs no separate plugin crate, no Gradle module and no direct `jni` dependency, because Tauri's own does the work.
Its test implementation holds a fixed key, which is what lets the whole store be exercised on the host rather than only cross-compiled.

Three parameters on that Keystore key, and the reasoning for each is worth keeping.
`setUnlockedDeviceRequired(true)` where the API level allows it, which is 28 and up and therefore behind a version check, because the seller's device being unlocked is already the condition under which any of this runs.
StrongBox requested with a fallback, because a device with a hardware security module should use it and a device without one must still work.
And `setUserAuthenticationRequired` deliberately left off: D3 already makes every run on a phone something the seller starts on an unlocked device, so a fingerprint or PIN prompt in front of every sync buys little and costs the seller every time.
That last one is a single flag and a founder-flippable option: turning it on tightens the key to a per-use authentication without changing anything else in this design.

What happens at the edges, stated rather than left to be discovered.
A fresh install has neither a wrapped blob nor a snapshot, so a new secret is generated and the seller signs in to each marketplace again.
An uninstall or an application wipe destroys the Keystore key, which leaves any surviving copy of the snapshot undecryptable rather than merely deleted.
Signing out is the existing `SessionStore::forget` path and the heartbeat's revocation wipe, unchanged, with the one Android limit recorded above: `delete_cookie` is a no-op there, so the WebView's own copy of the cookie needs `clear_all_browsing_data`.

### The background scheduler, which D3 settles as a product rule

The rethink memo's §5.2 reading stands: Android's Doze stops `JobScheduler` and therefore `WorkManager`, and the battery-optimisation exemption that would evade it is barred by Play policy.
D3 does not try to engineer around that; it narrows the product instead, so a phone runs work the seller starts and nothing else, and full parity remains for an API-branch marketplace because the server schedules that one and the phone never had to originate the request.

The code consequence is specific.
`run_schedule` in `lib.rs` opens a `tokio::time::interval` that ticks for the life of the process, and `Scheduler::hourly_over_seller_device_marketplaces` names the cadence; on Android that loop is compiled out and replaced by a command the console's own button calls.
The check-in is the half that cannot simply be dropped, because it is how a revoked device learns it was revoked, so on Android it runs on application resume and again before every user-initiated run rather than on a timer.
The rule the interface must carry is that the app never claims a schedule it cannot keep: no "syncing hourly" copy on a phone, and the device page says what this device actually does.

### The payload cache, which is very likely fine

`payload.rs` writes the bytes an upload needs into a directory under `app_data_dir` and removes them when the item settles.
On Android `PathResolver::app_data_dir` resolves through the platform plugin's `getDataDir` (`tauri` 2.11.5, `src/path/android.rs:137`), which is the application's own private storage, so the directory exists and is private without a permission.
What differs is that Android may reclaim that storage under pressure and may kill the process at any point, which the start-up sweep already covers.

## What is built here

The Android shell is `nix develop .#android`, added to `flake.nix` beside the default one rather than folded into it.
A second `nixpkgs` import backs it, because the Android SDK is unfree and its licence has to be accepted, and neither `allowUnfree` nor the licence flag belongs on the shell every other lane uses.
Acceptance is `config.android_sdk.accept_license = true` rather than the `NIXPKGS_ACCEPT_ANDROID_SDK_LICENSE` environment variable: `androidenv`'s `license.nix` reads the attribute first and only falls back to an impure `getEnv` when it is absent, so the attribute is the route that survives a pure evaluation.
Every version in the composition is named rather than left at `latest`, because `latest` resolves through nixpkgs' own `repo.json` and an input bump would otherwise move the SDK out from under a build that had been proven against it.

Measured in that shell on 2026-09-03: OpenJDK 17.0.20, `cmdline-tools` 19.0, `platform-tools` 37.0.1, `build-tools` 36.0.0, platform 36, NDK 29.0.14206865, and all four Android rust targets present under the channel `rust-toolchain.toml` pins.
The emulator, the system images and CMake are all off, which is several gigabytes not downloaded to compile an APK.

`cargo tauri android init` generated `apps/desktop/src-tauri/gen/android`, 512K, with `compileSdk` and `targetSdk` at 36, `minSdk` at 24 and Gradle 8.14.3 through the wrapper.
That tree is committed rather than regenerated, because the release signing configuration has to live in its `app/build.gradle.kts` and a build that patched that file on the fly would emit an unsigned release on a failed patch rather than an error.
The `signingConfigs` block added there is guarded, which Tauri's own snippet is not: theirs reads `keystore.properties` unconditionally and throws on a machine that has none, which is every machine here, so ours applies the configuration only when the file exists and lets Gradle name the output `app-<abi>-release-unsigned.apk` when it does not.

Three code changes carry the platform difference, and each is narrow.
`session/keychain.rs` is compiled only on the three platforms `keyring` supports, and `session/encrypted.rs` is selected in its place on Android.
A third module briefly stood between the two, refusing a capture on Android while the store was undecided; it was removed once this one landed, because a build with no key source is not a configuration that ships and a module with no consumer pays no rent.
The in-memory store was never a candidate for that role, for the reason its own documentation gives — a session that silently stopped being persisted looks identical to one that was.
`tauri-plugin-updater` is registered under `#[cfg(desktop)]` only, because registering a plugin that declares no Android support would give the console an update surface that answers nothing.
And the hourly timer is `#[cfg(desktop)]`: on a phone the cycle runs once at start-up and again on every `RunEvent::Resumed`, which is what D3 leaves in place of a schedule and is the only moment a device signed out elsewhere can learn it.

Both APKs were built here on 2026-09-03, arm64 only.

| Artefact | Size | `libtam_desktop.so` inside it |
|---|---|---|
| `app-universal-debug.apk` | 202.5 MB | 205.1 MB, unstripped |
| `app-universal-release-unsigned.apk` | 20.6 MB | 19.0 MB, stripped |

The debug figure is the whole of the difference and it is not a problem to solve.
Tauri's generated debug build type writes `jniLibs.keepDebugSymbols` for all four ABIs, which keeps the dev profile's full DWARF in the library on purpose, and the release build type carries no such line, so the Android Gradle Plugin strips there.
Nothing else in either APK is large: `classes.dex` is 2.0 MB in the release build and `resources.arsc` 1.0 MB, with no other entry above 40 KB.
Twenty megabytes is the number to plan against, and it is one ABI; a universal release carrying arm64 and armv7 will be roughly twice that, which is why Play takes the bundle and splits it per device.

The release APK is named `app-universal-release-unsigned.apk` rather than `app-universal-release.apk`, and that is the guard working rather than an oversight: no `keystore.properties` exists in this tree, so Gradle leaves the signing configuration unset and says so in the filename.

The recipes are `just android-init`, `android-build-debug`, `android-build` and `android-build-aab`, all of them inside that shell.
The debug recipe builds arm64 alone, because it is a device to install on rather than a release; the release recipe builds arm64 and armv7, which are the two ABIs a phone runs, and adding either x86 ABI for a Chromebook or an emulator is one word.

`.github/workflows/android-build.yml` is run by hand, with one `profile` input choosing the debug APK or the release APK, and it keeps the result as a workflow artefact beside a `SHA256SUMS.txt` and publishes nothing.
Neither a push to `main` nor a version tag starts it, for two separate reasons: an NDK-and-Rust build is twenty minutes of a finite Actions budget and is the wrong default for every commit, and the desktop's own `v*.*.*` tag must not start an Android job that cannot yet produce anything usable, because the session store is undecided and no signing secret exists.
A red Android run sitting beside the founder's first desktop release would cost more than the trigger is worth; it returns when the client reaches beta.
It runs on `ubuntu-latest`, pins the same NDK the flake does and fails by name if `setup-android` did not put it there, and pins every third-party action to a full commit SHA.
When the keystore secret is absent it says so in the log as a warning and lets the unsigned filename say it again.

## The interface

Founder requirement, 2026-09-03: the Android client's user interface follows Vendoo's Android app in structure, navigation, screen names and feature set, in Teachouse's own colours and typography, with no Vendoo text, asset or layout copied.
Because Phase 7 shares the console build with the desktop rather than reimplementing it, this is the console's mobile layout rather than a second interface, and the map it follows is `docs/research/rethink/vendoo-console-cross-reference.md`.
The interface work is therefore sequenced after that document lands, and is not part of the Stronghold session store.

## Signing and distribution

The release keystore is a JKS made with `keytool`, and Tauri's page gives the command as `keytool -genkey -v -keystore ~/upload-keystore.jks -keyalg RSA -keysize 2048 -validity 10000 -alias upload`.
Its location and password reach Gradle through `gen/android/keystore.properties`, holding `password`, `keyAlias` and `storeFile`, and the page's own warning is that neither the keystore nor that properties file goes into source control.
The build reads it through a `signingConfigs` block added to `gen/android/app/build.gradle.kts`, with `import java.io.FileInputStream` at the top and `signingConfig = signingConfigs.getByName("release")` inside `buildTypes`.
Source: <https://v2.tauri.app/distribute/sign/android/>, fetched 2026-09-03.

Because those edits live in a generated directory, `gen/android` is committed rather than regenerated, the same way the Windows bundle's icons and configuration are.
The alternative — regenerate on every build and patch the Gradle file from the workflow — puts a `sed` between us and a signed artefact, and a failed patch would produce an unsigned release rather than an error.

The build commands are `cargo tauri android build --apk` and `cargo tauri android build --aab`, with `--split-per-abi` for per-architecture APKs and `--target` to narrow the ABI set.
The AAB lands at `gen/android/app/build/outputs/bundle/universalRelease/app-universal-release.aab`.
Source: <https://v2.tauri.app/distribute/google-play/>, fetched 2026-09-03.

Play or sideload, and the recommendation is both, in that order of eventual importance and the reverse order of urgency.
A Play Console developer account costs "US$25 one-time registration fee" (<https://support.google.com/googleplay/android-developer/answer/6112435>, fetched 2026-09-03), and Play's internal testing track is the cheapest way to put a build on the founder's own phone and on a handful of teachers' phones without a public listing.
Tauri does not automate any of it: "The first upload must be made manually in the website so it can verify your app signature and bundle identifier", and "Tauri currently does not offer a way to automate the process of creating Android releases", which is what D29 already recorded.
A directly-downloaded APK needs no account and no review and is the right beta channel for the first weeks, at the cost of the seller having to allow installation from an unknown source.

CrabNebula Cloud does not distribute Android in the sense the Windows client uses it.
Its CI guidance mentions Android only as a build target — "If your application targets Android and iOS, we recommend defining separate jobs per platform so your workflow is easier to read" (<https://docs.crabnebula.dev/cloud/ci/tauri-v2-workflow/>, fetched 2026-09-03) — and its supported application types are Tauri v1, Tauri v2, `cargo-packager` and "Other: Any generic app/asset" (<https://docs.crabnebula.dev/cloud/>, fetched 2026-09-03).
The decisive fact is on our side of the wire rather than theirs: `tauri-plugin-updater` declares `platforms.support.android.level = "none"` in its own manifest (tauri-plugin-updater 2.11.0, `Cargo.toml`), so there is no in-app updater on Android to point at an endpoint at all.
The Cloud can therefore host an APK as a generic asset for the sideload channel, which is worth doing because it is free and already in the pipeline, but Android updates ship through Play, exactly as the rethink memo said at line 507.

That is now built rather than planned.
`.github/workflows/desktop-release.yml` carries an `android` job that builds the arm64 release APK on a version tag, in parallel with the Windows bundles, and uploads it to the same CrabNebula release with no `--public-platform` and no `--update-platform` — the documented form for a platform-independent asset, and the only one available given that neither platform list names Android (<https://docs.crabnebula.dev/cloud/cli/upload-assets/>, fetched 2026-09-03).
A generic asset is still fetched from the CDN by file name, at `https://cdn.crabnebula.app/download/<org-slug>/<app-slug>/latest/<asset-file-name>` (<https://docs.crabnebula.dev/cloud/cli/fetch-latest-release/>, fetched 2026-09-03), which is exactly what a sideload link needs and all it needs.
The APK is also attached to the GitHub release for the tag.

The whole job is gated on the signing secrets rather than only its upload.
An unsigned APK cannot be installed, so building one would spend twenty minutes of every tag to produce nothing a seller could use; the `verify` job computes the boolean from the three `ANDROID_KEY_*` secrets and the Android job's `if:` reads it, so an unconfigured repository skips the job and says so in the run summary.
Both Android profiles build arm64 alone, here and in `android-build.yml`, so a hand-run build and a tagged one ship the same ABI set; armv7 returns as its own decision if a seller's phone needs it.
The remaining guard is the filename Gradle chooses: the collect step matches `*-release.apk` alone, so a keystore that failed to apply leaves only `*-release-unsigned.apk`, matches nothing and fails the job instead of publishing something no one can install.
The key material never reaches the working tree unignored: `gen/android/.gitignore` already lists `keystore.properties` and `key.properties`, which is the generated project's own doing rather than ours.
The NDK version reaches this workflow and `android-build.yml` alike through `.github/scripts/android-pins.sh`, which reads it from `flake.nix`, so the pin has one home.

The workflow runs on `ubuntu-latest`, which is the cheap half of the tree.
GitHub prices a Linux 2-core runner at $0.006 a minute against a Windows 2-core runner's $0.010, and a private repository on the Free plan includes 2,000 minutes a month (<https://docs.github.com/en/billing/concepts/product-billing/github-actions>, fetched 2026-09-03).
Against the quota rather than the overage price, Linux counts at one minute per minute and Windows at two, so an Android job is the cheapest thing this repository can run on a runner and a debug-APK-per-push cadence is affordable in a way the Windows release job would not be.

## What the founder must do

The session-store question that used to head this list was answered on 2026-09-03 and is recorded above.
Of what remains, the upload keystore has been generated and nothing else has: no account was created and nothing was uploaded anywhere.

Create a Google Play Console developer account and pay the US$25 one-time fee, if the Play internal-testing track is wanted for the beta; the direct-APK channel needs neither.

The upload keystore is generated and waiting at `~/.tauri/android/teachouse-upload.p12`, mode 600, with its password in `teachouse-upload.password` beside it at mode 600 and the directory itself at 700.
It is PKCS12 rather than JKS — the JDK 17 default, which avoids `keytool`'s legacy-format warning — with alias `upload`, RSA 2048, ten thousand days of validity and a subject of `CN=Teachouse` alone, because the subject is permanent and no country or organisation was known to put in it.
Back both files up somewhere that survives losing the machine, and copy the password into the password manager.
This key is as irreplaceable as the updater key and for a different reason: Play binds an application's identity to its signing key, and losing it means the listing can never be updated again.

Then set the GitHub secrets the signing step reads, whose names Tauri's own CI example fixes: `ANDROID_KEY_ALIAS` (`upload`), `ANDROID_KEY_PASSWORD` (the contents of the password file), and `ANDROID_KEY_BASE64` (`base64 -i ~/.tauri/android/teachouse-upload.p12`).

## Amended 2026-09-06: the resume cadence, and a check-in the seller can press

The scheduler section above describes what a phone was meant to do — the check-in on resume, and the work pull replaced by a command a console button calls.
The code did something else.
`run_schedule` on mobile ran a full `cycle` on every `Resumed`, and a cycle is a check-in followed by a scheduler tick over every seller-device marketplace, which posts a work claim to the control plane and then makes marketplace requests from the phone.
A phone is brought forward twenty times an hour by an ordinary seller, so that was twenty claims and twenty rounds of requests, which is a deviation from D3 rather than an application of it.
No console button existed either: `device_check_in` was granted and registered, and the console called it once per load and never again.

Both halves are now built.

A resume checks in every time and ticks the scheduler at most once per `Scheduler::DEFAULT_CADENCE`.
The check-in is not gated and must not be, because it is the only channel by which a phone learns the seller signed it out; the work pull is the half with no such warrant.
The cadence is the hour the desktop timer already keeps, read from the scheduler the mobile loop already constructs rather than invented as a second number, so no founder-gated limit moves.
The mechanism is a pure predicate, `scheduler::work_is_due`, and one held instant that is stamped only on the branch that actually ticked; `heartbeat::resume` is the composition of the two and is what the mobile loop calls.
A clock that has moved backwards reads as not due and resolves itself, rather than handing an oscillating clock a pull on every resume.

The console carries "Check in now" in the "Your machines" panel header, rendered only inside the application.
It is the same `device_check_in` the console calls on load, so it costs one button rather than a command, and the server's own upsert makes pressing it twice a refresh rather than a second machine.
Not offered disabled in a browser: a control a seller can never enable is a promise the page cannot keep, and the panel already says that machines report for themselves.

Beside it, `DeviceState` now carries `detail` — `ControlPlaneError`'s own sentence when a check-in did not reach us, and null when it did.
The panel renders it as one line when a check-in it was asked for came back false.
The four sentences name no credential, no jar and no host but our own control plane, and they are the difference between "nothing appeared in the list" and a cause somebody can act on.
On a phone that difference is the whole diagnostic surface: `startup.log` is in private storage no one reaches without `run-as`, and stdout goes to logcat, which needs a cable.
`docs/notes/runbooks/android-phone-check.md` is the ten-minute check that reads this line, and is the first exercise of any of this on a real handset rather than on an emulator.

## Amended 2026-09-06: the login capture landed on Android, by navigation

The section above plans the phone's login as a navigation of the one webview and leaves open whether a second window would in fact build there.
Both halves are now settled, and the open question is closed against the second window rather than left unproven.

A second window does not build on Android.
tao's `Window::new` takes the next Android context with no window created, and there is exactly one Activity, so a second window answers `OsError::NoAvailableActivity` (tao 0.35.3, `src/platform_impl/android/mod.rs` and `src/platform_impl/android/ndk_glue.rs`).
That is why the console withheld the Connect button on a phone rather than offering one that would fail: the seller would have read "the sign-in could not be opened on this machine" and had nothing to do about it.

What is built is the navigation this note recommended.
`commands::connect_marketplace` now chooses between two surfaces rather than assuming one.
`ConnectSurface::SecondWindow` is every desktop platform and is today's body unchanged — a window labelled `login-<Marketplace>`, the same poll, the same ten-minute deadline, the same destroy.
`ConnectSurface::OneWindow` is Android: it navigates window `main` to `target.login_url`, spawns the capture on the runtime, and answers `ConnectOutcome::Opening` before the navigation lands.
The surface is a value rather than a `cfg`, so both bodies compile on every target and the phone's arm is exercised by host tests on a developer's machine instead of only on a handset.

Answering before navigating is the shape the surface forces rather than a preference.
Tauri delivers a command's answer by evaluating a callback in whatever page the webview is showing, and on this surface the page that asked is about to be the marketplace's own — the one page `capabilities/default.json` describes the fence as keeping our code out of.
So the answer carries no result, the capture runs with no caller waiting on it, and the verdict comes back in the address: `connect::return_url` builds `{base}/marketplaces?connect=<code>&marketplace=<name>` over a closed `ConnectVerdict` of `Captured`, `Deadline`, `Abandoned`, `Refused` and `NotKept`, and `connectReturn` in `web/src/lib/pages/marketplaces/view.ts` is its total reader.
A query parameter rather than a command or an event because it needs no capability and no grant, and because it can only choose a sentence: whether a marketplace is connected is still read from the server's connection list, so a hand-typed parameter changes copy and never state.

Two things a computer never has to deal with are dealt with here.
A phone's abandon is the back gesture, which walks the webview's history and so lands it back at our own origin rather than closing anything, so the poll watches for that as well as for the deadline.
And because `navigate` is a message to the platform's main thread and the address only changes when the load commits, the capture first waits for the sign-in to actually replace the console before "back at our own origin" is allowed to mean abandoned — without that the first poll would read the console's own address and report the sign-in abandoned half a second after the seller asked for it.
A sign-in that never appears at all within thirty seconds is `Refused`, which is a different sentence from one the seller did not finish.
A sign-in that finished and could not be filed is `NotKept`, which is a third sentence again: the cause worth naming there is a device signed out or revoked from the console, because `file_session`'s check-in learns of it and wipes the store, and the seller's remedy is to sign in to Teachouse again on the phone.
The two were one code until a review found what that cost — a seller whose device had been signed out mid-sign-in was told the sign-in could not be opened, which is the opposite of what happened.

The fence changes character on this surface and the difference is worth stating.
On a computer the marketplace page sits in a window whose label is in no capability, and that absence is the whole fence.
On a phone it sits in window `main`, which every capability names, and what refuses it is the per-invoke remote-origin check against the one origin `console.json` grants (tauri 2.11.5, `src/webview/mod.rs`).
Keep that remote-origin list restricted to exactly `DEFAULT_BASE_URL`; Android's shared window label does not provide isolation from marketplace content.

## Amended 2026-09-06: back walks the webview's history

The section above assumes the back gesture returns the seller to the console, and until this change nothing in our own code arranged that.
wry registers a back callback that calls `goBack()` only when `handleBackNavigation` is set (`vendor/wry-0.55.1/src/android/kotlin/WryActivity.kt:52-77`), and Tauri's generated `TauriActivity` overrides it to `false` (`tauri-2.11.5/mobile/android-codegen/TauriActivity.kt:35`), so no callback of ours was registered and back was left to whatever the platform does with it.
That matters here more than elsewhere, because on this surface the marketplace's sign-in is the whole window: if back leaves the app, the seller is stranded and `ConnectVerdict::Abandoned` is unreachable, since the poll's test for the seller having gone is the webview arriving back at our origin.

`MainActivity.kt` now overrides it to `true`.
That file is tracked, unlike the generated activity beside it, and the property Tauri declares is not final, so the override is legal and wins; `apkanalyzer dex packages` confirms `MainActivity.getHandleBackNavigation` is in the shipped dex and was not there before.
Back therefore walks the webview's history through our own registered callback and finishes the Activity only when the history is exhausted.

What the change is not is a repair of an observed break, and that is worth recording rather than implying otherwise.
On an API 36 x86_64 emulator the build without this override already returned from a second page to the first on one back press, with the Activity still foregrounded and the process unchanged — so on that image the platform's own back routing was already doing what we want, and the review's reading that back finished the Activity did not reproduce.
What the override buys is therefore that the behaviour is ours and explicit rather than a platform default we neither set nor test, on an Android version range we do not control.
Whether a real arm64 handset behaved the way the emulator did is unmeasured in both builds, and `docs/notes/runbooks/android-phone-check.md` is where that evidence belongs.

Three consequences follow and are recorded rather than discovered later.
Back inside the console now walks its own pushState history, so a seller moving between screens goes back a screen instead of leaving; the return leg clears its verdict parameter with a replacing navigation, so back cannot replay a sentence about a sign-in that finished minutes ago.
A seller who moved through several of the marketplace's own pages presses back once per page, so the abandon is reached when they arrive back at ours rather than on the first press.
And the console is reached by navigating the window away from the bundled start page, so that page is one history entry behind the console's first screen and a seller pressing back from there lands on it before the app exits.

Closing an open sheet or picker with back is a later refinement rather than part of this change: nothing here intercepts back above the webview's history, so a back press with a sheet open walks history as it would with the sheet closed.

What has not been proved on a handset is the same thing this note has never been able to prove: that a real TPT or Tes sign-in completes in an Android WebView and that the captured session works from a mobile network.
`docs/notes/runbooks/android-phone-check.md` is where that evidence belongs, and it is the founder's own account and their own phone.

## Amended 2026-09-07: one notification per cycle

The client takes `tauri-plugin-notification` 2.4.0 on every platform, registered in `lib.rs` beside the os and opener plugins, and raises one notification per cycle that settled anything, never one per item.
The summary is taken over the tick's whole report after it is recorded, in `heartbeat::cycle`, and `notify.rs` holds the seam: a `Notifier` trait with one method over one summary value, implemented by the plugin and by a recorder in tests, as `SessionStore` and `ControlPlane` are.
The notice uses the words of the server's completion mail, so a seller reading both reads one thing, with the counts under the console's own outcome words.
No capability file changes and no npm package: the notification is raised from Rust, and Tauri's capabilities gate IPC commands rather than the Rust API.

On Android the permission is requested at the first cycle that has something to say rather than at launch, so the prompt arrives with a reason attached, and it is requested once per process: a seller who dismissed it without answering is not asked again hourly, and a seller who refused is not asked at all.
Either leaves the cycle exactly as it was, because the work is done and recorded before the notice is raised.
The plugin's own Android manifest merges `POST_NOTIFICATIONS`, `RECEIVE_BOOT_COMPLETED` and `WAKE_LOCK` into the build at Gradle's manifest merge, so `gen/android/app/src/main/AndroidManifest.xml` is unchanged and the three arrive with the dependency.

Windows needs an installed build rather than a development run, and the plugin's own manifest is the source: `plugins/notification/Cargo.toml:23` in the local checkout of `tauri-apps/plugins-workspace` at `845d8989` records `windows = { level = "full", notes = "Only works for installed apps. Shows powershell name & icon in development." }`.
The code behind it sets the AppUserModelID only when the executable's directory is not `target/debug` or `target/release` (`src/desktop.rs`, the `cfg(windows)` block in `show`), so a development run shows PowerShell's name and icon or nothing at all.
Neither the phone nor an installed Windows build has raised one yet; `docs/notes/runbooks/android-phone-check.md` says what to look for on each.

## Sources

`docs/notes/design/vendoo-for-teachers-rethink.md`, decisions D2, D3, D12, D14 and D29, and its §5.1 and §5.2 readings of mobile session capture and mobile scheduling.
`docs/notes/design/desktop-client.md`, whose "What is stubbed" section this note corrects on Android cookie access.
`docs/notes/design/desktop-distribution.md`, for the pipeline shape this one parallels.
<https://v2.tauri.app/start/prerequisites/>, <https://v2.tauri.app/develop/>, <https://v2.tauri.app/distribute/sign/android/> and <https://v2.tauri.app/distribute/google-play/>, all fetched 2026-09-03.
The nixpkgs Android manual section and `pkgs/development/mobile/androidenv`, read at nixpkgs `56c02bc0`, the revision `flake.lock` pins, 2026-09-03.
`wry` 0.55.1, `tauri` 2.11.5, `tauri-plugin-updater` 2.11.0 and `keyring` 3.6.3, read from the vendored crate sources, 2026-09-03.
`tauri-plugin-stronghold` 2.3.2 and `BiometricPlugin.kt`, read from `~/ghq/github.com/tauri-apps/plugins-workspace`, 2026-09-03.
`tauri-plugin-notification` 2.4.0, its `Cargo.toml`, `src/desktop.rs`, `src/mobile.rs` and `android/src/main/AndroidManifest.xml`, read from the same checkout at `845d8989`, 2026-09-07.
