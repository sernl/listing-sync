"""Assemble the reviewable source package AMO requires for compiled code.

Mozilla's rule is that a reviewer must be able to rebuild the submitted
artefact from the source they are given, with build instructions, environment,
tool versions, the full command list and lockfiles
(https://extensionworkshop.com/documentation/publish/source-code-submission/).
A Rust-to-wasm toolchain is inside that rule even though WebAssembly is not
named in it.

The package is self-sufficient rather than a pointer at this repository: a
flake pinning the same nixpkgs and rust-overlay revisions, the workspace
lockfile unchanged, a minimal workspace root, and the extension crate. The
recipe text is extracted from the real justfile rather than retyped, so the
commands a reviewer runs cannot drift from the ones we run.
"""

import json
import shutil
import subprocess
import sys
from pathlib import Path

import pack

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
STAGE = HERE / "build" / "source"
KEPT_INPUTS = ("nixpkgs", "rust-overlay")

FLAKE = """{
  description = "Reviewable source for the Teachouse listing sync extension";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ (import rust-overlay) ];
      };
      toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          toolchain
          pkgs.wasm-bindgen-cli
          pkgs.python3
        ];
      };
    };
}
"""


def tables(text: str, prefix: str) -> str:
    """Every TOML table whose header starts with `prefix`, verbatim."""
    kept, keeping = [], False
    for line in text.splitlines(keepends=True):
        if line.startswith("["):
            keeping = line.startswith(prefix)
        if keeping:
            kept.append(line)
    return "".join(kept)


def recipe(text: str, name: str) -> str:
    """One justfile recipe or assignment, verbatim, with its comment block."""
    lines = text.splitlines(keepends=True)
    starts = [i for i, line in enumerate(lines) if line.startswith(name)]
    if not starts:
        raise SystemExit(f"source-package: no `{name}` in the justfile")
    start = starts[0]
    while start > 0 and lines[start - 1].startswith("#"):
        start -= 1
    end = starts[0] + 1
    while end < len(lines) and (
        not lines[end].strip() or lines[end].startswith((" ", "\t"))
    ):
        end += 1
    return "".join(lines[start:end]).rstrip() + "\n"


def trimmed_lock(lock: dict) -> dict:
    nodes = {name: lock["nodes"][name] for name in KEPT_INPUTS}
    nodes["root"] = {"inputs": {name: name for name in KEPT_INPUTS}}
    return {"nodes": nodes, "root": "root", "version": lock["version"]}


def tool_version(*command: str) -> str:
    try:
        return subprocess.run(
            command, capture_output=True, text=True, check=True
        ).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return "not found in this shell"


def main() -> int:
    justfile = (ROOT / "justfile").read_text()
    cargo = (ROOT / "Cargo.toml").read_text()
    lock = json.loads((ROOT / "flake.lock").read_text())
    version = tool_version("sh", "-c", "grep '^version' " + str(HERE / "Cargo.toml"))

    if STAGE.exists():
        shutil.rmtree(STAGE)
    (STAGE / "apps" / "extension").mkdir(parents=True)

    for name in ("Cargo.toml", "pack.py", "source-package.py", ".gitignore"):
        shutil.copy2(HERE / name, STAGE / "apps" / "extension" / name)
    for directory in ("src", "static", "tests"):
        shutil.copytree(HERE / directory, STAGE / "apps" / "extension" / directory)

    shutil.copy2(ROOT / "Cargo.lock", STAGE / "Cargo.lock")
    shutil.copy2(ROOT / "rust-toolchain.toml", STAGE / "rust-toolchain.toml")

    # The profile tables matter as much as the lints: `[profile.release]` turns
    # on overflow-checks and debug-assertions, and a root without them builds a
    # different program from the one submitted.
    (STAGE / "Cargo.toml").write_text(
        '[workspace]\nresolver = "2"\nmembers  = ["apps/extension"]\n\n'
        + tables(cargo, "[workspace.lints.")
        + "\n"
        + tables(cargo, "[profile.")
    )
    (STAGE / "flake.nix").write_text(FLAKE)
    (STAGE / "flake.lock").write_text(
        json.dumps(trimmed_lock(lock), indent=2, sort_keys=True) + "\n"
    )
    (STAGE / "justfile").write_text(
        "\n".join(
            recipe(justfile, name)
            for name in (
                "extension_static :=",
                "extension_built :=",
                "extension-wasm:",
                "extension-dev:",
                "extension-build:",
            )
        )
    )
    (STAGE / "BUILD.md").write_text(
        build_instructions(lock, version)
    )

    names = sorted(
        str(path.relative_to(STAGE))
        for path in STAGE.rglob("*")
        if path.is_file()
    )
    out = HERE / "build" / "tam-extension-source.zip"
    digest = pack.pack(out, STAGE, names)
    print(f"{digest}  {out}")
    print(f"{len(names)} files staged at {STAGE}")
    return 0


def build_instructions(lock: dict, version: str) -> str:
    nixpkgs = lock["nodes"]["nixpkgs"]["locked"]
    overlay = lock["nodes"]["rust-overlay"]["locked"]
    return f"""# Building this extension from source

This package rebuilds the submitted `tam-extension.zip` byte for byte.
The artefact is deterministic by construction: entries are written in sorted
order with a fixed 1980-01-01 timestamp and fixed 0644 permissions, so two
builds of this source produce one file.

## What the build needs

Linux with Nix and flakes enabled, and nothing else installed by hand.
Every tool comes from the pinned flake in this directory.

- nixpkgs, pinned to `{nixpkgs['rev']}` ({nixpkgs['type']}:{nixpkgs['owner']}/{nixpkgs['repo']})
- rust-overlay, pinned to `{overlay['rev']}`
- the Rust toolchain named in `rust-toolchain.toml`, resolved by rust-overlay
- `wasm-bindgen-cli` from that nixpkgs, whose version the crate pins exactly
  in `apps/extension/Cargo.toml`; the CLI refuses a crate whose version differs

Recorded from the shell that produced the submitted build:

```
{version}
rustc            {tool_version('rustc', '--version')}
cargo            {tool_version('cargo', '--version')}
wasm-bindgen     {tool_version('wasm-bindgen', '--version')}
python3          {tool_version('python3', '--version')}
```

## The commands

```
nix develop --command just extension-build
```

That runs, in order:

1. `cargo build --target wasm32-unknown-unknown --release --lib -p tam-extension`
2. `wasm-bindgen --target web --out-name extension --out-dir apps/extension/dist <the .wasm>`
3. copies the four hand-written files from `apps/extension/static` into `dist`
4. `python3 apps/extension/pack.py` over the six shipped files

The result is `apps/extension/build/tam-extension.zip`, and `pack.py` prints
its SHA-256 so it can be compared with the submitted file without extracting
either.

To run the same steps without `just`, read `justfile`: it holds those four
commands and nothing else.

## Notes for a reviewer

`Cargo.lock` is this repository's lockfile unchanged, so every dependency
resolves to the version the submitted build used. Cargo prunes it on first
build to the subset this crate needs; that rewrite does not change any version.

`apps/extension/dist` also receives `extension.d.ts` and
`extension_bg.wasm.d.ts`, which wasm-bindgen writes for TypeScript consumers.
They are deliberately not in the package: `pack.py` is given the six file names
explicitly rather than told to walk the directory.

The `.wasm` is compiled from the Rust sources in `apps/extension/src`, which
are the whole of the extension's compiled logic. `extension.js` is generated by
wasm-bindgen and is not hand-edited.
"""


if __name__ == "__main__":
    sys.exit(main())
