# login-probe

A read-only probe that answers one question, decision D12 in
`docs/notes/design/vendoo-for-teachers-rethink.md`: can an embedded webview,
the same webview stack the Tauri v2 desktop client will ship, load the
TeachersPayTeachers and Tes login pages without hitting a bot challenge?

It is the kill gate on Phase 2, the Windows desktop client, so nothing is
committed to that surface before it answers.
The tool opens a URL in the operating system's own webview through `wry` and
`tao`, the two layers Tauri v2 is itself built on, waits for redirects and
challenges to settle, then reads the page and writes down what it found.
The versions are pinned to what `tauri` 2.11.5 resolves to, `wry` 0.55.1 and
`tao` 0.35.3, so the answer is an answer about the shipping stack rather than
about whatever the registry happens to offer on the day.

It never fills a form, never submits anything, and carries no credential.
Navigating and reading are all it does.

This directory is its own Cargo workspace.
The repository root manifest lists its members explicitly, so the empty
`[workspace]` table in `Cargo.toml` here is what keeps cargo from refusing to
build a nested package that is neither a member nor excluded; the root manifest
is untouched.

## Using it

```
login-probe <url> [--wait-secs N] [--out report.json]
```

A window opens on the given URL and stays visible, so a human can watch what
happens rather than trusting the summary alone.
After `--wait-secs` seconds, default 15, a script runs in the page and reports
back; the tool then writes `report.json`, writes the full page HTML beside it
as `report.html`, prints a one-line verdict, and exits 0.
The exit status is 0 whatever the verdict, because the report is the result and
a challenge is a finding rather than a failure; the only non-zero status is 2,
for a usage error, raised before any window opens.

The verdict is one of three words.
CHALLENGE means at least one bot-protection marker was found, and the report
names each one along with where it was seen.
CLEAR means no marker was found and the page carries a password input, which is
what an ordinary login page looks like.
UNKNOWN means neither, which on a login URL usually means the page had not
finished loading, so re-run with a longer `--wait-secs`.
On TPT the two markers that fire are Cloudflare's always-present bot-management
script under `/cdn-cgi/challenge-platform` and the reCAPTCHA widget on the form
itself, so CHALLENGE alongside a password input means a widget the human
satisfies while logging in rather than a wall; a wall is CHALLENGE with no
password input at all.

The report also carries an `outcome`, which is separate from the verdict.
`reported` means the page answered.
`timeout` means the injected script never reported within 20 seconds of being
run, which on a hostile page is itself a signal; the window was visible while
that happened, so what was on screen is part of the evidence.
`window_closed` means a human closed the window before the probe finished.

Markers are searched for in the page HTML, in the final URL, and in the cookies
the page exposes to script: Cloudflare's interstitial and Turnstile, hCaptcha,
reCAPTCHA, DataDome, PerimeterX, Akamai Bot Manager, and Kasada.
One limit matters when reading a report.
`document.cookie` cannot see HttpOnly cookies, and TPT's Cloudflare cookie
`__cf_bm` is HttpOnly and is already known to be set on `/Login`
(`docs/research/feasibility-report.md:238`), so it will never appear in the
cookie evidence.
Cookie evidence is a lower bound; the HTML and URL evidence carries the weight.

## Building and running on Linux

The Linux webview is WebKitGTK 4.1, which `wry` binds through the `webkit2gtk`
2.0 crate, and it needs that library's development headers at build time.
On NixOS one command supplies all of them:

```
nix-shell -p pkg-config gtk3 webkitgtk_4_1 libsoup_3 --run 'cargo build --release'
```

`nix-shell -p` rather than `nix shell nixpkgs#...` is deliberate, and is the one
place this file departs from the obvious command.
`nix shell` puts binaries on `PATH` and nothing else, so `pkg-config` runs but
finds no `.pc` files and the build dies in `webkit2gtk-sys`; `nix-shell -p` runs
the stdenv setup hooks that populate `PKG_CONFIG_PATH` from each package's `dev`
output.
Verified against nixpkgs from the system flake registry: WebKitGTK 2.52.5,
GTK 3.24.52, libsoup 3.6.6, rustc 1.97.1.

Running needs one thing the build does not.
WebKitGTK gets TLS from a GIO module supplied by `glib-networking`, and without
it every `https` navigation lands on a page reading "TLS support is not
available" while the probe cheerfully reports UNKNOWN on `about:blank`.
The ambient `GIO_EXTRA_MODULES` on this machine carries gvfs and dconf but not
that module, and no setup hook adds it, so point at it explicitly:

```
export GIO_EXTRA_MODULES="$(nix build --no-link --print-out-paths nixpkgs#glib-networking)/lib/gio/modules:$GIO_EXTRA_MODULES"
nix-shell -p pkg-config gtk3 webkitgtk_4_1 libsoup_3 glib-networking \
  --run './target/release/login-probe https://example.com --wait-secs 10 --out /tmp/probe.json'
```

The Linux result is indicative, not decisive.
The desktop client ships on Windows first, where the webview is WebView2 and
the user agent, the TLS fingerprint and the behaviour all differ, so a Linux
CLEAR does not promise a Windows CLEAR and a Linux CHALLENGE does not condemn
Windows.
The user agent WebKitGTK presents here, recorded in every report, is
`Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)
Version/60.5 Safari/605.1.15`.

Two things this build prints on Linux are noise rather than findings.
Every load prints "GStreamer element appsink not found", which is WebKitGTK's
media pipeline reporting an absent element and has no bearing on a probe that
never plays media.
A second run in the same shell has printed "free(): corrupted unsorted chunks"
while exiting, after the report and the HTML were already on disk, so a
teardown crash following a written report is not a failed probe: check the
report, which is the result.

## Running on Windows

Windows is the half that decides D12, because Windows is the first shipping
surface.
The webview there is WebView2, which ships with Windows 10 and Windows 11, so
nothing needs installing for the probe to run.

The executable cross-compiles from NixOS, which is D29's route for Windows
bundles generally, and it is built and staged at `dist/login-probe.exe`:
890368 bytes, SHA-256 `ebd2f7b175c03ff80a2f181b388ae8701841e6c8d5fba9d573eff1a3e384494e`.
Copy that one file to the Windows machine, open a terminal in the folder holding
it, and run:

```
.\login-probe.exe "https://www.teacherspayteachers.com/Login" --wait-secs 20 --out tpt.json
.\login-probe.exe "https://www.tes.com/authn/sign-in?rtn=https%3A%2F%2Fwww.tes.com%2Fteaching-resources" --wait-secs 20 --out tes.json
```

Each run opens a window; leave it alone and let it close itself.
Send back four files: `tpt.json`, `tpt.html`, `tes.json` and `tes.html`.
Both URLs are confirmed by a Linux run, and the Tes one is the sign-in form
rather than `https://www.tes.com/login`, which is a Drupal chooser page listing
Tes products and carries no password input at all.
The URLs are quoted because the Tes one carries a query string and percent
escapes that an unquoted command line can mangle.
If a page is slow, raise `--wait-secs`; nothing is lost by waiting longer.

The cross-compile is reproduced with `cargo-xwin` 0.23.0 and a rust 1.97.1
toolchain carrying the `x86_64-pc-windows-msvc` target.
The repository's own toolchain has only the Linux target and `rust-toolchain.toml`
is a shared file, so the Windows target comes from an ad-hoc shell pinned to the
same `rust-overlay` revision the repository flake already locks:

```
cat > /tmp/xwin-shell.nix <<'EOF'
let
  rustOverlay = builtins.getFlake "github:oxalica/rust-overlay/ab450d47a3f906d19de1b332915bfc6e5b29c853";
  pkgs = import (builtins.getFlake "nixpkgs") {
    system = "x86_64-linux";
    overlays = [ rustOverlay.overlays.default ];
  };
in
[
  (pkgs.rust-bin.stable."1.97.1".default.override { targets = [ "x86_64-pc-windows-msvc" ]; })
  pkgs.cargo-xwin
  pkgs.llvmPackages.clang
  pkgs.lld
]
EOF
nix shell --impure --file /tmp/xwin-shell.nix \
  --command cargo xwin build --release --target x86_64-pc-windows-msvc
cp target/x86_64-pc-windows-msvc/release/login-probe.exe dist/
```

If that ever stops working, the fallback needs no NixOS at all: install Rust on
the Windows machine from <https://rustup.rs>, copy this directory across, and
run `cargo run --release -- <url> --wait-secs 20 --out tpt.json` from it.
WebView2 is already present, so nothing else is required.

`dist/` and `target/` are ignored here; the root `.gitignore` ignores only
`/target` at the repository root, so this directory carries its own.
