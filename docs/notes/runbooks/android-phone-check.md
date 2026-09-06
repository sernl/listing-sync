---
title: Checking a real Android phone appears as a machine
---

# Checking a real Android phone appears as a machine

Ten minutes, one phone, no cable.
It answers one question: does a phone that installs Teachouse and signs in appear under "Your machines" with its own name beside it?
Everything the Android client does has been exercised on an x86_64 emulator and on no real handset, so this is the first evidence that any of it works where it has to.

- date: 2026-09-06
- applies to: an arm64 Android phone, which is every current handset
- prerequisite: a Teachouse account you can sign in to

## Before you start

Open the release page for the version you are testing and confirm it carries two files: `Teachouse_<version>_arm64.apk` and `SHA256SUMS-android.txt`.
Version 0.3.2 carries both.

If they are missing, the Android job was skipped because the three `ANDROID_KEY_*` secrets are not set, and the run summary for that tag says so.
Set them and re-run the tag, or build locally with `just android-build` inside `nix develop .#android`, and come back.

The APK is arm64 only, deliberately.
An x86 Android device — an emulator image, or one of the handful of Intel tablets — has no asset to install and is out of scope for this check.

## Install

Download the APK onto the phone, from the release page in the phone's own browser.
Android asks whether to allow installing from that source the first time; allow it, install, and open Teachouse.

The app opens its own window straight onto the console.
If it shows a bundled page saying it cannot reach us instead, that is the network probe rather than a failure of the app, and the page's own button retries.

## Sign in

Sign in on the phone exactly as you would on a computer, in the window the app opened.
Wait for the console to finish loading.
Registration happens on that load, so a page that is still loading has not registered yet.

## Look

Go to Marketplaces and scroll to "Your machines".

The phone should be a row of its own, named by the phone rather than by a host name: "Google Pixel 8", "Samsung SM-G991B", "OnePlus CPH2451".
Under the name should be a line reading `Android · aarch64 · app <version> · last seen just now`.

Two labels are worth sending back and neither is a failure of registration.
A row reading `localhost` means the phone's own name could not be read and the host name was used instead.
A row reading "Android phone" means the phone reported neither a manufacturer nor a model.

## Confirm it is one machine and not two

Close the app fully — from the recent-apps list, not by pressing back — reopen it, and reload Marketplaces.

There must still be one phone row.
A second row means the device identity did not survive the restart, and every token bound to the first row is stranded.
That is worth stopping for.

## If no phone row appears

Press "Check in now", beside the "Your machines" heading, on the phone.
It asks the app to register and check in again, and it is the same call the console makes when it loads.

If it fails, a line appears under the panel's description reading "This machine could not tell us it is here:" and then the app's own sentence.
There are four of them and each says something different: no way to reach the server in this build, the server refused, this device is not registered, and nobody is signed in on this device.
That sentence is the report.

Then open Settings and read the "Browser sign-ins" panel.
A phone sign-in listed there as a row of its own, rather than absorbed into a machine, is the signature of the failure: the sign-in reached us and the registration did not.

Send two screenshots — Marketplaces "Your machines", and Settings "Browser sign-ins" — and the sentence.
That is enough to name the cause without attaching the phone to anything.

## If you have a cable and want more

Neither of these is required, and neither is a substitute for the screenshots above.

`adb logcat -s RustStdoutStderr` carries this application's own stdout and stderr, including the `starting <version> android aarch64` line that every launch writes.
`adb shell run-as io.teachouse.desktop cat startup.log` is the same log as it sits on disk, in the app's private storage.
No directory prefix: `run-as` starts in the app's data directory, which is where Tauri resolves `app_data_dir` to on Android (`activity.dataDir`, tauri 2.11.5 `PathPlugin.kt`), and the log sits directly in it.
