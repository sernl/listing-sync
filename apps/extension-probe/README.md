# extension-probe

Throwaway probes for the browser-extension kill gates G1 to G3, stated in
`docs/research/rethink/oxichrome-extension-client.md` section 6.
The measurements they produced, and the commands that produced them, are in
`docs/research/rethink/oxichrome-extension/gates.md`; this directory is the
apparatus rather than the result.

These are probes, not product.
They are plain JavaScript with no build step, no workspace crate and no place in
`just check`, because their job is to answer a question once and then stand as
the evidence for the answer.
`apps/extension` is where an extension would actually be built if the gates say
build one.

Three unpacked extensions and their servers:

- `g2-redirect/` reads a 302 `Location` through non-blocking `webRequest` while
  the fetch resolves opaque; `run-g2.sh` runs it.
- `g3-payload/` drives a 256 MiB multipart upload from an offscreen document or
  a runner tab while the service worker idles; `run-g3.sh` runs it, selecting
  the host with `G3_VARIANT` and the profile with `G3_PROFILE`.
- `g1-tpt-envelope/` is the founder-run probe against a live
  TeachersPayTeachers session; its README is written for the founder and is the
  file to read before running it. `verify-g1.sh` exercises it end to end against
  the local mimic instead.

Every server under `servers/` binds `127.0.0.1` only.
Nothing in this directory contacts a marketplace: `g1-tpt-envelope` is the one
that can, and only when a human clicks its buttons with the marketplace origin
in its panel.

The browser throughout is Chromium 151.0.7922.71 from nixpkgs, reached as
`nix shell nixpkgs#chromium --command chromium`, and `--headless=new` loads an
unpacked MV3 extension and runs its service worker on that version, so no
virtual display is needed.
Every runner leaves its Chrome profile under `/tmp` for inspection and deletes
nothing.
