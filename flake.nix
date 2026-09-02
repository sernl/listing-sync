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
              || (builtins.match ".*/docs/design/data/.*\\.json" path != null);
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

            # The client-target gate, the same five triples and the same two
            # crate sets `just check-portable` runs. No cargoArtifacts: crane's
            # host-target artifacts are not reusable across a --target, so this
            # compiles its own from the vendored source.
            portable = craneLib.mkCargoDerivation (
              commonArgs
              // {
                cargoArtifacts = null;
                pnameSuffix = "-portable";
                doInstallCargoArtifacts = false;
                buildPhaseCargoCommand = ''
                  crates="-p tam-types -p tam-marketplace -p tam-domain -p tam-taxonomy -p tam-marketplace-tpt -p tam-marketplace-tes"
                  for target in wasm32-unknown-unknown x86_64-pc-windows-msvc aarch64-apple-darwin aarch64-apple-ios aarch64-linux-android; do
                    # tam-limits asserts usize::BITS >= 64, which wasm32 is not
                    case "$target" in
                      wasm32-*) extra="" ;;
                      *) extra="-p tam-limits" ;;
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
              pkgs.podman
              pkgs.podman-compose
              pkgs.postgresql_17
              pkgs.nodejs_22
              pkgs.shellcheck
              pkgs.sqlx-cli
            ];
          };

          formatter = pkgs.nixfmt-rfc-style;
        };
    };
}
