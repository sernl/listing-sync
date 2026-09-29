---
title: Checking a real Android device
---

# Checking a real Android device

Use a real arm64 Android phone or tablet to check registration, marketplace sign-in and original-file opening.
Emulator-only checks do not establish that these paths work on a seller's device.
Marketplace sign-in must run from the authenticated control-plane origin; do not widen the native command grant to use a stand-in console.

- date: 2026-09-29
- applies to: an Android phone or tablet on Android 7.0 (API 24) or newer
- prerequisite: a Teachouse account you can sign in to, and — for the second exercise — a TPT or Tes account of your own

## Before you start

Open the release page for the version you are testing and confirm it carries two files: `Teachouse_<version>_universal.apk` and `SHA256SUMS-android.txt`.
Releases up to 0.14.0 carried `Teachouse_<version>_arm64.apk` instead.

If either file is missing, inspect the complete tagged-release logs before installing anything.
Missing signing credentials, a build failure and a skipped release job are distinct failures; do not substitute an unsigned or debug APK for an existing signed installation.

The APK is universal: one file carrying arm64 and x86_64 code, so the same download installs on a phone, a tablet, a Chromebook and an x86_64 emulator.
A 32-bit-only device (old or budget ARM phones) cannot run Teachouse: the byte limits are 64-bit and the build refuses a 32-bit target.
Nothing in it is phone-only: it declares every screen size, requires no telephony, camera or other hardware, and runs in landscape and split screen.

## Install

Download the APK onto the phone, from the release page in the phone's own browser.
Android asks whether to allow installing from that source the first time; allow it, install, and open Teachouse.

Update an existing installation in place, with the same package and signing certificate and a version code no lower than the installed one.
For a cabled device, use `adb -s SERIAL install -r APK`.
Do not uninstall or clear app data to work around a failed update of a **release** installation: that destroys the Android Keystore key used to protect the saved sessions and library.
The one exception is a device that was ever given a debug build, below.

The app opens its own window straight onto the console.
If it shows a bundled page saying it cannot reach us instead, that is the network probe rather than a failure of the app, and the page's own button retries.

## When Android says "App not installed"

Read the whole sentence: Android's installer names the failure class after "App not installed as", and each class has a different fix.

| The installer says | What it means | What to do |
| --- | --- | --- |
| "…as package conflicts with an existing package" | A Teachouse signed with a **different certificate** is already installed. Every release is signed with the one release key, so the installed copy is a debug build. | Uninstall first, below. |
| "…as app isn't compatible with your phone/tablet" | The device is below Android 7.0, the APK is for another processor, or the installed copy has a **higher** version than this APK. | Install the newest release. On a release installation, never uninstall to downgrade. |
| "…as package appears to be invalid" | The download is truncated or not the release file. | Download again; compare its SHA-256 with `SHA256SUMS-android.txt`. |
| A Samsung sheet naming **Auto Blocker** | Samsung's Auto Blocker (One UI 6 and newer) refuses every app not from Play or Galaxy Store. It is on by default on devices that shipped with One UI 6.1.1. | Settings → Security and privacy → Auto Blocker → off, install, then turn it back on. |

With a cable, `adb -s SERIAL install -r APK` prints the underlying code instead of a sentence:

| `adb install` says | Meaning |
| --- | --- |
| `INSTALL_FAILED_UPDATE_INCOMPATIBLE: … signatures do not match newer version` | Different certificate from the installed copy: a debug build is installed. Uninstall first. |
| `INSTALL_FAILED_VERSION_DOWNGRADE` | The installed version code is higher than this APK's. Install a newer release. |
| `INSTALL_FAILED_NO_MATCHING_ABIS` | The APK has no code for this processor: a 32-bit-only device. |
| `INSTALL_FAILED_OLDER_SDK` | The device is below Android 7.0 (API 24). |
| `INSTALL_PARSE_FAILED_NO_CERTIFICATES` or `INSTALL_PARSE_FAILED_NOT_APK` | Unsigned or damaged file. Check the name ends `_universal.apk` and the SHA-256 matches. |
| `INSTALL_FAILED_USER_RESTRICTED` | The device refused the install over USB: Auto Blocker, or "Install via USB" is off in developer options. |
| `INSTALL_FAILED_INSUFFICIENT_STORAGE` | Not enough free space on the device. |

To see what is installed, and whether it is a debug build:

```sh
adb -s SERIAL shell dumpsys package io.teachouse.desktop | grep -E 'versionCode|versionName|pkgFlags'
```

`DEBUGGABLE` in `pkgFlags` is a debug build.
To compare certificates, pull the installed copy and print both:

```sh
adb -s SERIAL pull "$(adb -s SERIAL shell pm path io.teachouse.desktop | sed -n 's/^package://p' | head -1)" installed.apk
apksigner verify --print-certs installed.apk | grep 'SHA-256'
apksigner verify --print-certs Teachouse_<version>_universal.apk | grep 'SHA-256'
```

Two different digests are the signature conflict; the release job prints the release digest in its "collect the APK" step.

### Uninstall first when a debug build was ever installed

A debug build is signed with the key of the machine that built it — a laptop's `~/.android/debug.keystore`, or a key the CI runner makes fresh for each `android-build` debug run.
Android refuses to update an app with one signed by another key, so no release can ever update a debug installation, and no debug build can update another machine's debug build.
Uninstalling is the only way onto the release, and it costs what the Keystore protects:

1. On any other device, open Resources → Files and make sure every original this device keeps is also kept by another device. Uninstalling deletes the ones held only here.
2. On the device: Settings → Apps → Teachouse → Uninstall. With a cable: `adb -s SERIAL uninstall io.teachouse.desktop`.
3. Install the release APK from the release page, open it and sign in again. Marketplace sign-ins on this device must be connected again.
4. The device registers as a new row under "Device sign-ins". Press **Sign out** on the old row for it; that copy of the app no longer exists.

After that, every later release updates in place and this never needs doing again, as long as only release APKs are installed.

## Sign in

An in-place update should retain the existing sign-in. On a fresh installation, sign in in the window the app opened.
Wait for the console to finish loading.
Registration happens on that load, so a page that is still loading has not registered yet.

## Look

Go to Settings → Preferences and find "Device sign-ins", below "Browser sign-ins".

The phone should be a row of its own, named by the phone rather than by a host name: "Google Pixel 8", "Samsung SM-G991B", "OnePlus CPH2451".
Under the name should be a line reading `Android · aarch64 · app <version> · last seen just now`.

Two labels are worth sending back and neither is a failure of registration.
A row reading `localhost` means the phone's own name could not be read and the host name was used instead.
A row reading "Android phone" means the phone reported neither a manufacturer nor a model.

While you are here, open a resource that is listed somewhere and tap its marketplace tile: the phone's own browser should open the listing, rather than the marketplace appearing inside Teachouse.
A listing that opens inside the app instead has lost the seller their way back, and is worth sending back with the marketplace's name.

## Connect a marketplace, and stop at the sign-in page

This is the second exercise and the one nothing but a handset can answer.
Everything above proves the phone is a machine we can see; this proves it is a machine that can hold a marketplace login.
Two passes, and the first deliberately signs in to nothing.

Still on Marketplaces, the TPT and TES cards should each carry a "Connect TPT" or "Connect TES" button.
A card reading "Connect TPT from the Teachouse app on your computer or phone" instead is the browser copy, and on a phone inside the app it is wrong: it means the app did not recognise itself, and that is worth stopping for.

Press Connect TPT.
The console should be replaced, in the same window, by TPT's own sign-in page — there is no second window on a phone, which is why it takes over rather than opening beside.
Do not sign in yet.

Press back.
Within about a second the app should return you to Marketplaces with one line at the top reading "The TPT sign-in did not finish, so nothing was saved. Press Connect TPT to try again."
That sentence is the whole point of this pass: it is the difference between a seller who knows the sign-in did not take and one who is returned to the console in silence and has to guess.
Back walks the pages you have been through rather than closing the app, so if you moved through more than one of TPT's own pages before changing your mind, press it once per page until Marketplaces comes back.
If the app closes instead of returning you, stop and send that back: it is the one behaviour this exercise cannot work without, it has never been observed on a real handset either way, and it is the single most useful thing you can tell us.
Four other sentences can appear in place of the one above, and each says something different — "was not finished in time" is the ten-minute deadline, "page did not open" is the sign-in never appearing at all, "could not be saved on this device" is a sign-in that finished while this phone was signed out of Teachouse, and "TPT is connected on this device" is a success you did not intend.

Then the real pass, which is yours as the account holder and is the only evidence that any of this works.
Press Connect TPT again, sign in to TPT as you would in any browser, and answer any captcha or second factor exactly as you would there — it is your sign-in, on your phone, in TPT's own page, and nothing about it reaches us but the session cookie the app files on the device.
You should be returned to Marketplaces with "TPT is connected on this device", and the TPT card should read connected with the phone named under "Your machines" as the machine holding it.

Repeat for TES if you want both.

What to send back if either pass goes wrong: the sentence you actually read, and a screenshot of the Marketplaces page.
A sign-in that appears to work on the phone but leaves the card disconnected is the case worth the most detail, because it means the capture ran and the cookie the adapter needs was not in the jar.

## Disconnect, and what it can and cannot remove

Press Disconnect on a connected card.
The confirmation on a phone carries one sentence a computer's does not: that your marketplace sign-in stays in the phone's browser, where we cannot remove it, so connecting again may not ask for your password.

That is true rather than a hedge, and it is worth confirming once.
Disconnect, then press Connect again: if the marketplace signs you straight back in without asking for a password, that is the behaviour the sentence describes, and it is the platform's rather than ours — Android gives an app no way to remove one origin's cookies, and the only lever that clears any of them clears every origin the app has visited, ours included, which would sign you out of Teachouse as a side effect of disconnecting TPT.
Our own copy of the session is gone either way, and the card and "Your machines" should both say so.

## Confirm it is one machine and not two

Close the app fully — from the recent-apps list, not by pressing back — reopen it, and return to Settings → Preferences → Device sign-ins.
Back is not a substitute here even now that it leaves the app once the pages behind you run out: an Activity that finishes leaves the process, and everything the app is holding, alive.

There must still be one phone row.
A second row means the device identity did not survive the restart, and every token bound to the first row is stranded.
That is worth stopping for.

## Open a kept original

In Resources → Files, choose an original already kept on this device and press Open.
If Android offers a chooser, select a local viewer and "Just once"; do not change the default app.
The viewer must display the document or the archive's contents. A chooser alone is not a pass.
Return to Teachouse and confirm the session and original remain available.

With a cable, `adb -s SERIAL shell dumpsys activity activities` should show the receiver using a `content://io.teachouse.desktop.fileprovider/…` URI and the file's MIME type.
Its intent grants read access, not write, persistent or prefix access.

Do not test Remove on the only holder of an original.
First complete a copy to another authorized native device and establish the peer-transfer recovery path.
Re-import is not that recovery path: resources already in Resources are skipped before their files are downloaded.

## One notification, once a cycle has settled something

With a sync queued for a marketplace this phone holds, close the app fully and open it again so the start-up cycle claims the work, and confirm four things: the permission prompt appears once and only now, one notification appears naming the count of what settled, a second cycle with nothing due raises none, and refusing the prompt leaves the app working with the email arriving as before.
The same check on Windows needs an installed build rather than a development run, because the plugin's own manifest records that Windows notifications work only for installed applications and show PowerShell's name and icon otherwise.

## If no phone row appears

Press "Check in now", beside "Device sign-ins" in Settings → Preferences, on the device.
It asks the app to register and check in again, and it is the same call the console makes when it loads.

If it fails, a line appears under the panel's description reading "This machine could not tell us it is here:" and then the app's own sentence.
There are four of them and each says something different: no way to reach the server in this build, the server refused, this device is not registered, and nobody is signed in on this device.
That sentence is the report.

Then read the "Browser sign-ins" panel immediately above it.
A phone sign-in listed there as a row of its own, rather than absorbed into a machine, is the signature of the failure: the sign-in reached us and the registration did not.

Send screenshots of both sign-in panels and the refusal sentence.
That is enough to name the cause without attaching the phone to anything.

## If you have a cable and want more

Neither of these is required, and neither is a substitute for the screenshots above.

`adb -s SERIAL logcat -s RustStdoutStderr` carries this application's own stdout and stderr, including the `starting <version> android aarch64` line that every launch writes.
`adb -s SERIAL shell run-as io.teachouse.desktop cat startup.log` is the same log as it sits on disk, in the app's private storage.
No directory prefix: `run-as` starts in the app's data directory, which is where Tauri resolves `app_data_dir` to on Android (`activity.dataDir`, tauri 2.11.5 `PathPlugin.kt`), and the log sits directly in it.

## When a page says "That page could not be opened"

That sentence is the console's own error page, so the cause is in the phone's WebView, not the app. The page now prints what was thrown under the sentence; read it out. To see the whole console log, the debug build's WebView takes Chrome's devtools protocol over adb:

```sh
adb shell cat /proc/net/unix | grep -o 'webview_devtools_remote_[0-9]*'   # once the app is open
adb forward tcp:9444 localabstract:webview_devtools_remote_<pid>
curl -s 127.0.0.1:9444/json/version    # "Browser": the WebView's Chrome version
curl -s 127.0.0.1:9444/json            # pages and their websocket URLs
```

Any CDP client on that websocket (`Runtime.enable`, then `Runtime.exceptionThrown` and `Runtime.consoleAPICalled`) shows the exception behind the page. The 0.13.0 error on a Galaxy Note10+ was found this way on the `api31` emulator, whose System WebView is Chrome 91: `Object.hasOwn is not a function`, a method that engine lacks. `web/src/app.html` now defines each such method and `web/src/lib/polyfills.test.ts` keeps the list against the source; a new one shows here first.

## When the app says "Teachouse can’t be reached"

That card is the app's own start page. From 0.15.0 it also comes back when the console was sent for but had drawn nothing after 8 seconds, so a white window no longer lasts until the app is killed. "Why?" says which of the two happened: the connection check failing prints its own error; "The server answered, but the Teachouse page did not finish loading within 8 seconds" means the network reached Teachouse and the page itself stalled.

Check the connection the device is actually using, not the status bar: airplane mode can be on with Wi-Fi still connected, and that is a working connection. With a cable, `adb -s SERIAL shell dumpsys connectivity | grep -E "Active default network|VALIDATED"` shows it without changing anything, and `adb -s SERIAL logcat -s RustStdoutStderr` carries the line "the console did not appear within 8s (last seen: …)" with the address the window was on. A stall on a validated network is a console fault; send that line with the time.

## When the app says "Set a screen lock on this device"

The banner reads "Set a screen lock on this device to keep your files on it, then open Teachouse again." It means the phone has no PIN, pattern, password or biometric lock, and Android will not hold the key the app's files are sealed under without one; the phone keeps and serves no files until a lock is set. Press **Open settings**, set any screen lock, then close and open Teachouse again: the banner is gone and the Files page lists this device's files. A phone that has a lock never shows this banner, even while locked. With a cable, `adb -s SERIAL logcat -s RustStdoutStderr` carries "the library on this machine could not be opened: the library key needs a screen lock, and this device has none".
