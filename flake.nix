{
  description = "Listing Sync: server-side bulk upload and cross-listing for teaching-resource marketplaces";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # cargo audit runs with -n against this directory, so the database is only
    # ever as fresh as flake.lock; the bump is a scheduled job, not a fetch.
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
        "x86_64-darwin"
      ];

      perSystem =
        { system, ... }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ (import inputs.rust-overlay) ];
          };
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchain;

          # cleanCargoSource would drop sqlx's offline query metadata, the
          # SQL migrations, the cassette fixtures and the captured taxonomy
          # data, all of which the sandboxed build needs.
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter =
              path: type:
              (craneLib.filterCargoSources path type)
              || (builtins.match ".*/\\.sqlx/query-.*\\.json" path != null)
              || (builtins.match ".*/migrations/.*\\.sql" path != null)
              || (builtins.match ".*/tests/cassettes/.*\\.json" path != null)
              || (builtins.match ".*/docs/design/data/.*\\.jsonl?" path != null)
              # The verdict-fixtures bin and equivalence test in tam-core-wasm
              # include drafts.json and verdicts.json from outside src/.
              || (builtins.match ".*/crates/[^/]+/fixtures/.*\\.json" path != null);
          };
          # filterCargoSources keeps every .toml, so deny.toml and clippy.toml
          # are already in src; narrowing to the latter keeps the file-count
          # gate off the rebuild path of every Rust edit.
          clippyTomlTree = pkgs.lib.cleanSourceWith {
            inherit src;
            filter = path: type: type == "directory" || baseNameOf path == "clippy.toml";
          };
          commonArgs = {
            inherit src;
            strictDeps = true;
            # The desktop client is excluded from the sandboxed lanes rather
            # than built in them. Two reasons, both structural: `tauri.conf.json`
            # and the bundle icons are not Cargo sources, so `filterCargoSources`
            # drops them and `generate_context!` has nothing to read; and its
            # `frontendDist` is `web/build`, a gitignored npm artefact that no
            # Cargo-source filter can produce. It is covered by `just check`
            # and by `just desktop-build` instead.
            cargoExtraArgs = "--locked --workspace --exclude tam-desktop";
          };
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          bin = craneLib.buildPackage (commonArgs // { inherit cargoArtifacts; });
        in
        {
          packages.default = bin;

          checks = {
            inherit bin;

            # The default --ignore yanked stands. Measured: -n leaves the
            # sandbox without a crates.io index, so cargo-audit logs "couldn't
            # check if the package is yanked" once per dependency whether the
            # flag is set or not, and cannot detect a yanked crate here at all.
            # deny.toml's yanked = "deny" is enforced by the networked just deny.
            audit = craneLib.cargoAudit (commonArgs // { advisory-db = inputs.advisory-db; });

            ban-strings = pkgs.runCommand "ban-strings" { nativeBuildInputs = [ pkgs.ripgrep ]; } ''
              for entry in 'path = "str::split_at",' 'path = "str::split_at_mut",'; do
                if ! rg -qF "$entry" ${./clippy.toml}; then
                  echo "clippy.toml no longer carries the entry: $entry" >&2
                  echo "crates/ban-probe catches a misspelt path; nothing but this catches a deleted one." >&2
                  exit 1
                fi
              done
              touch $out
            '';

            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--all-targets -- --deny warnings";
              }
            );

            # Reported, never gated: -W without -D exits 0 on a hit, so this
            # fails only when the tree stops compiling. crane splices the extra
            # args straight into the build phase, which is why the tee rides
            # along in the same string.
            clippy-advisory = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                doInstallCargoArtifacts = false;
                cargoClippyExtraArgs = ''
                  --all-targets -- \
                    -W clippy::nursery -W clippy::cargo \
                    -W clippy::too_many_lines -W clippy::cognitive_complexity -W clippy::type_complexity \
                    2>&1 | tee advisory-report.txt'';
                installPhaseCommand = "install -Dm444 advisory-report.txt $out/advisory-report.txt";
              }
            );

            clippy-toml-count = pkgs.runCommand "clippy-toml-count" { nativeBuildInputs = [ pkgs.fd ]; } ''
              found=$(cd ${clippyTomlTree} && fd -HI -g clippy.toml --exclude target)
              if [ "$found" != "clippy.toml" ]; then
                echo "a crate-local clippy.toml replaces the root file rather than merging with it," >&2
                echo "and it does so at exit 0 with no diagnostic, so the tree may hold exactly one." >&2
                echo "found:" >&2
                printf '%s\n' "$found" >&2
                exit 1
              fi
              touch $out
            '';

            # Leave cargoDenyChecks at its default. Advisories are cargoAudit's
            # lane; adding them here makes cargo-deny git-clone the RustSec
            # database at build time, and the sandbox has neither network nor git.
            deny = craneLib.cargoDeny commonArgs;

            fmt = craneLib.cargoFmt { inherit src; };

            # The client-target gate, running exactly what `just check-portable`
            # runs. The triples and the two crate sets are read out of the
            # justfile rather than copied, because the copy that used to live
            # here drifted: it was still proving six crates after the recipe had
            # grown to eight, so a green check said less than it claimed.
            # No cargoArtifacts: crane's host-target artifacts are not reusable
            # across a --target, so this compiles its own from the vendored
            # source.
            portable =
              let
                justfileLines = pkgs.lib.splitString "\n" (builtins.readFile ./justfile);
                # The quoted value of a `name := "..."` line in the justfile.
                justVar =
                  name:
                  let
                    hits = builtins.filter (m: m != null) (
                      map (line: builtins.match "${name} := \"([^\"]*)\" *" line) justfileLines
                    );
                  in
                  if hits == [ ] then
                    throw "flake.nix: the justfile has no `${name} := \"...\"` line, which checks.portable reads its crate list from"
                  else
                    builtins.head (builtins.head hits);
              in
              craneLib.mkCargoDerivation (
                commonArgs
                // {
                  cargoArtifacts = null;
                  pnameSuffix = "-portable";
                  doInstallCargoArtifacts = false;
                  buildPhaseCargoCommand = ''
                    crates="${justVar "portable_crates"}"
                    crates64="${justVar "portable_crates_64"}"
                    for target in ${justVar "portable_targets"}; do
                      # The 64-bit-only leg, kept off wasm32 for the reason the
                      # justfile records beside portable_crates_64.
                      case "$target" in
                        wasm32-*) extra="" ;;
                        *) extra="$crates64" ;;
                      esac
                      cargo check --target "$target" --no-default-features $crates $extra
                    done
                  '';
                  installPhaseCommand = "touch $out";
                }
              );

            nextest = craneLib.cargoNextest (commonArgs // { inherit cargoArtifacts; });
          };

          devShells.default = craneLib.devShell {
            inputsFrom = [ bin ];
            packages = [
              pkgs.cargo-nextest
              pkgs.cargo-deny
              pkgs.cargo-machete
              pkgs.just
              # `just deny`'s runtime-graph check reads cargo metadata through it
              pkgs.jq
              pkgs.podman
              pkgs.podman-compose
              pkgs.postgresql_17
              pkgs.nodejs_22
              pkgs.shellcheck
              # The browser's copy of the core. Pinned by nixpkgs rather than
              # by us: wasm-bindgen refuses a CLI whose version differs from
              # the crate's, so crates/tam-core-wasm pins the crate to exactly
              # what this carries.
              pkgs.wasm-bindgen-cli
              pkgs.sqlx-cli
              # The desktop client. `cargo-tauri` drives the bundle,
              # `cargo-xwin` and `nsis` are D29's local Windows cross-compile,
              # and `pkg-config` finds the Linux webview below.
              pkgs.cargo-tauri
              pkgs.cargo-xwin
              pkgs.nsis
              pkgs.pkg-config
            ];
            # buildInputs rather than packages: the pkg-config setup hook
            # populates PKG_CONFIG_PATH from a package's `dev` output only for
            # inputs at the host offset, and wry's webkit2gtk-sys build fails
            # without those .pc files (tools/login-probe/README.md).
            buildInputs = [
              pkgs.gtk3
              pkgs.webkitgtk_4_1
              pkgs.libsoup_3
              pkgs.glib-networking
            ];
            # WebKitGTK takes TLS from a GIO module that no setup hook adds,
            # and without it every https navigation lands on "TLS support is
            # not available" instead of the login page. Same finding, same file.
            shellHook = ''
              export GIO_EXTRA_MODULES="${pkgs.glib-networking}/lib/gio/modules''${GIO_EXTRA_MODULES:+:$GIO_EXTRA_MODULES}"
            '';
            # The Windows cross-compile needs an *unwrapped* clang. Measured:
            # nixpkgs' cc-wrapper is not multi-target aware, so it reads
            # cargo-xwin's MSVC-style `/imsvc` include flags as filenames and
            # adds `-fPIC`, which clang-cl rejects for a windows-msvc target;
            # ring's C sources are where that first bites. It is deliberately
            # not on PATH, because an unwrapped clang ahead of the wrapper
            # breaks every native C build in the tree. `just
            # desktop-build-windows` prepends it for exactly one command.
            TAURI_WINDOWS_TOOLCHAIN_BIN = pkgs.lib.makeBinPath [
              pkgs.llvmPackages.clang-unwrapped
              pkgs.lld
              pkgs.llvmPackages.llvm
            ];
          };

          formatter = pkgs.nixfmt-rfc-style;
        };
    };
}
