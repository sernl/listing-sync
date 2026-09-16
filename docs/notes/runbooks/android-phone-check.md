---
title: Checking a real Android device
---

# Checking a real Android device

Use a real arm64 Android phone or tablet to check registration, marketplace sign-in and original-file opening.
Emulator-only checks do not establish that these paths work on a seller's device.
Marketplace sign-in must run from the authenticated control-plane origin; do not widen the native command grant to use a stand-in console.

- date: 2026-09-06
- applies to: an arm64 Android phone or tablet
- prerequisite: a Teachouse account you can sign in to, and — for the second exercise — a TPT or Tes account of your own

## Before you start

Open the release page for the version you are testing and confirm it carries two files: `Teachouse_<version>_arm64.apk` and `SHA256SUMS-android.txt`.

If either file is missing, inspect the complete tagged-release logs before installing anything.
Missing signing credentials, a build failure and a skipped release job are distinct failures; do not substitute an unsigned or debug APK for an existing signed installation.

The APK is arm64 only, deliberately.
An x86 Android device — an emulator image, or one of the handful of Intel tablets — has no asset to install and is out of scope for this check.

## Install

Download the APK onto the phone, from the release page in the phone's own browser.
Android asks whether to allow installing from that source the first time; allow it, install, and open Teachouse.

Update an existing installation in place, with the same package and signing certificate and a version code no lower than the installed one.
For a cabled device, use `adb -s SERIAL install -r APK`.
Never uninstall or clear app data to work around a signing mismatch: that destroys the Android Keystore key used to protect the saved sessions and library.

The app opens its own window straight onto the console.
If it shows a bundled page saying it cannot reach us instead, that is the network probe rather than a failure of the app, and the page's own button retries.

## Sign in

An in-place update should retain the existing sign-in. On a fresh installation, sign in in the window the app opened.
Wait for the console to finish loading.
Registration happens on that load, so a page that is still loading has not registered yet.

## Look

Go to Settings → Preferences and find "Machine sign-ins", below "Browser sign-ins".

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

Close the app fully — from the recent-apps list, not by pressing back — reopen it, and return to Settings → Preferences → Machine sign-ins.
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

Press "Check in now", beside "Machine sign-ins" in Settings → Preferences, on the device.
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
