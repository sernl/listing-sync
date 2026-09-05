# Vendored dependencies

One directory per patched crate, each a copy of a published version carrying
the smallest change that makes it usable here, reached through
`[patch.crates-io]` in the workspace `Cargo.toml`.

A crate belongs here only when the defect is upstream, the fix is small enough
to read in one sitting, and waiting for a release would block work that has to
ship.
Each entry below states the upstream version, the whole diff, the evidence that
motivated it, and the condition under which the entry is deleted.
Nothing else in a vendored copy is edited, so `diff -r` against the published
crate shows exactly the lines below and nothing more.

## wry 0.55.1

Upstream: <https://crates.io/crates/wry/0.55.1>, reached here through
`tauri` 2.11.5 → `tauri-runtime-wry` 2.11.4 → `wry`.

Removed when upstream ships all four fixes and `tauri-runtime-wry` allows that
version; the proof that it can go is a debug APK that launches on a device with
no cookie for the control-plane origin.

The copy is the published crate with three files changed, in four hunks, and one
file absent. Two of the hunks are in `src/android/main_pipe.rs`, which is why
the file count and the hunk count differ. The absent file is `.cargo-ok`, which
cargo writes when it extracts a crate and which is not part of the published
artefact.

### The defect

The Android client aborted before its first frame on every fresh install.
Reproduced on an API 31 x86_64 emulator, where the process died 1.2 seconds
after the window was drawn:

```
Displayed io.teachouse.desktop/.MainActivity: +420ms
RustStdoutStderr: starting 0.3.0 android x86_64
F java_vm_ext.cc:579] JNI DETECTED ERROR IN APPLICATION: JNI FindClass called with
  pending exception java.lang.NullPointerException: getCookie(...) must not be null
F java_vm_ext.cc:579]     in call to FindClass
F java_vm_ext.cc:579]     from void android.os.MessageQueue.nativePollOnce(long, int)
F runtime.cc:669] Runtime aborting...
```

Two independent faults stack, and either one alone is enough to abort.

`RustWebView.getCookies` declares a non-null Kotlin `String` over
`CookieManager.getCookie`, whose contract returns null when the cookie jar
holds nothing for that URL — which is every fresh install, before the seller
has signed in.
Kotlin's generated null check throws, and the message in the log is that
check's own wording, so the null return is observed rather than inferred.

`MainPipe::recv` then swallows the resulting JNI error with
`.unwrap_or_default()` and never clears it.
A pending exception stays pending in the `JNIEnv`, so the next JNI call — here
`FindClass`, from the looper's own `nativePollOnce` — trips ART's
pending-exception check and kills the process.
This is the fault that converts a recoverable "no cookies" into an abort, and
it would convert any other JNI failure in that call the same way.

This crate reaches the fault through `WebView::cookies_for_url`, which the
desktop client calls to read the console's session cookie before its first
check-in.

### The diff

`src/android/kotlin/RustWebView.kt`, in `getCookies`:

```diff
-        return cookieManager.getCookie(url)
+        return cookieManager.getCookie(url) ?: ""
```

An empty string is the right answer rather than a convenient one: the caller in
`main_pipe.rs` splits on `"; "` and `flat_map`s `Cookie::parse` over the
fields, which drops the single unparseable empty field and yields an empty
cookie list — exactly "this URL has no cookies".

`src/android/main_pipe.rs`, in the `WebViewMessage::GetCookies` arm, after the
`.unwrap_or_default()`:

```diff
+            if self.env.exception_check().unwrap_or(false) {
+              let _ = self.env.exception_clear();
+              #[cfg(debug_assertions)]
+              eprintln!("wry: cleared a pending JNI exception from RustWebView.getCookies");
+            }
```

`src/android/main_pipe.rs` again, in the `WebViewMessage::GetUrl` arm, after its
own `.unwrap_or_default()` and before `tx.send(url)`:

```diff
+            if self.env.exception_check().unwrap_or(false) {
+              let _ = self.env.exception_clear();
+              #[cfg(debug_assertions)]
+              eprintln!("wry: cleared a pending JNI exception from WebView.getUrl");
+            }
```

The fourth hunk is the same fault in a second arm, found by review rather than
by a crash. `GetUrl` calls `getUrl` over JNI and swallows any failure with the
identical `.unwrap_or_default()`, with nothing clearing a pending exception
before the next JNI call. It is reachable from this application: `open_console`
in `apps/desktop/src-tauri/src/lib.rs` calls `window.url()` whenever the control
plane is unreachable, which is the fallback path a seller with no network takes.
What is not claimed is that `WebView.getUrl` ever throws — it is an ordinary
platform getter and no throw has been observed — so this is structural parity
with a fault that did fire, not a second demonstrated crash.

The breadcrumb on both is `eprintln!` under `cfg(debug_assertions)` rather than
the `tracing::warn!` the same file uses at its `on_webview_created` hook, and
the choice is deliberate. Nothing in this application installs a `tracing`
subscriber, so a `tracing` call would be discarded; `eprintln!` reaches logcat,
because tao redirects the process's stdout and stderr there under the tag
`RustStdoutStderr` (`tao` 0.35.3, `src/platform_impl/android/ndk_glue.rs:327`).
The workspace sets `debug-assertions = true` under `[profile.release]`, so the
line survives into the shipping build rather than only the debug one. A
maintainer taking this upstream would reasonably prefer the `tracing` form,
where a subscriber is the caller's business; that variant is offered in the
upstream report.

`src/android/kotlin/proguard-wry.pro`, in the `RustWebView` keep block:

```diff
   void evalScript(...);
+  void clearAllBrowsingData(...);
+  java.lang.String getCookies(...);
 }
```

This third fault only shows in a minified build, which is what ships. wry calls
`getCookies` and `clearAllBrowsingData` from Rust by name over JNI, but its own
generated keep rules list only `loadUrlMainThread`, `loadHTMLMainThread` and
`evalScript`, so R8 is free to rename or drop the other two. It does: in the
released `Teachouse_0.3.0_arm64.apk`, the string `getCookies` does not appear in
`classes.dex` at all, while the kept `loadUrlMainThread` does. The JNI lookup
then fails with `NoSuchMethodError` instead of returning null, which — before
the `exception_clear` fix above — aborted the process by the same route, and
after it would have left cookies permanently unreadable on every release build.
So the debug and release builds were failing for two different reasons, and the
release one is the one the founder installed.

Kept free of comments so the diff stays four hunks a reviewer can check against
upstream in one pass; the reasoning is here instead.

### One consequence to expect

Cargo applies `--cap-lints allow` to registry dependencies and not to path ones,
so vendoring surfaces wry's own warnings — six of them at this version, all
`webkit2gtk` deprecations on the Linux backend — where they were previously
capped.
They are noise in the output rather than a gate failure: `just check` and the
flake's clippy check both pass their deny through as trailing arguments
(`-- --deny warnings`), which cargo gives to workspace members only, and a
local run of that exact form against the patched tree exits zero with the six
warnings printed.

