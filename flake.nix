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

      # The deployment contract, consumed by the fleet flake that owns the
      # machine. It resolves this flake's own packages for the machine's system,
      # so a consumer needs no overlay and cannot deploy a binary built from a
      # different tree than the module it read.
      flake.nixosModules.teachouse = import ./nix/module.nix { inherit (inputs) self; };

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

          # D2's second surface. A separate nixpkgs import rather than a
          # `config` on the one above, because the Android SDK is unfree and
          # its licence has to be accepted, and neither belongs on the shell
          # every other lane uses. Acceptance is the attribute rather than the
          # `NIXPKGS_ACCEPT_ANDROID_SDK_LICENSE` environment variable, because
          # androidenv reads `config.android_sdk.accept_license` first and only
          # falls back to the impure `getEnv` when it is absent
          # (nixpkgs `pkgs/development/mobile/androidenv/license.nix`).
          pkgsAndroid = import inputs.nixpkgs {
            inherit system;
            config = {
              allowUnfree = true;
              android_sdk.accept_license = true;
            };
          };
          # The two versions the generated Gradle project and the workflow both
          # name, bound once here so a bump is one edit rather than four.
          #
          # 35.0.0 rather than the latest, and the pin is not cosmetic: the
          # Android Gradle Plugin the generated project carries is 8.11.0,
          # whose default build-tools is 35.0.0, and a Gradle that wants a
          # version the SDK does not hold tries to install it into the Nix
          # store and fails with "The SDK directory is not writable". The
          # generated `app/build.gradle.kts` names the same version back, so
          # the two cannot drift into that failure again.
          androidBuildToolsVersion = "35.0.0";
          androidNdkVersion = "29.0.14206865";
          # Every version is pinned. `latest` in this composition resolves
          # through nixpkgs' `repo.json`, so an input bump would silently move
          # the SDK a build was proven against; naming them makes that a diff.
          androidComposition = pkgsAndroid.androidenv.composeAndroidPackages {
            cmdLineToolsVersion = "19.0";
            platformToolsVersion = "37.0.1";
            buildToolsVersions = [ androidBuildToolsVersion ];
            # 36 is `compileSdk` and `targetSdk` in the generated project.
            platformVersions = [ "36" ];
            includeNDK = true;
            ndkVersions = [ androidNdkVersion ];
            # Nothing here builds C++ through CMake -- Tauri drives cargo from
            # a Gradle task -- and the emulator and its system images are
            # gigabytes we do not need to compile an APK.
            includeCmake = false;
            includeEmulator = false;
            includeSystemImages = false;
          };
          androidSdkRoot = "${androidComposition.androidsdk}/libexec/android-sdk";
          androidNdkRoot = "${androidSdkRoot}/ndk/${androidNdkVersion}";
          # rust-toolchain.toml carries one Android triple, because
          # `just check-portable` proves one per surface rather than one per
          # ABI. A bundle needs all four, so this shell's toolchain widens the
          # same file's channel rather than pinning a second version.
          androidRustToolchain = rustToolchain.override {
            targets = [
              "aarch64-linux-android"
              "armv7-linux-androideabi"
              "i686-linux-android"
              "x86_64-linux-android"
            ];
          };
          bin = craneLib.buildPackage (commonArgs // { inherit cargoArtifacts; });

          # One binary per deployed process rather than the whole workspace, so
          # the closure copied to a 2 GB box carries the two processes that run
          # there and not the seed, crawl and operator tools beside them.
          # `cargoArtifacts` is shared with `bin`, so the marginal build is the
          # leaf crates.
          serviceBin =
            crate:
            craneLib.buildPackage (
              commonArgs
              // {
                inherit cargoArtifacts;
                pname = crate;
                cargoExtraArgs = "--locked -p ${crate}";
                # `checks.nextest` runs the suite once for the whole workspace;
                # running it again per binary would prove the same thing twice.
                doCheck = false;
              }
            );

          # The browser's copy of the core, built by the two commands
          # `just web-wasm` runs. No `cargoArtifacts`: crane's host-target
          # artifacts are not reusable across a `--target`, which is the same
          # reason `checks.portable` builds its own.
          coreWasm = craneLib.mkCargoDerivation (
            commonArgs
            // {
              cargoArtifacts = null;
              pnameSuffix = "-core-wasm";
              doInstallCargoArtifacts = false;
              nativeBuildInputs = [ pkgs.wasm-bindgen-cli ];
              buildPhaseCargoCommand = ''
                cargo build --locked --target wasm32-unknown-unknown --release --lib -p tam-core-wasm
              '';
              installPhaseCommand = ''
                wasm-bindgen --target web --out-name core --out-dir $out \
                  "''${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/tam_core_wasm.wasm"
              '';
            }
          );

          teachouseConsole = pkgs.callPackage ./nix/console.nix {
            nodejs = pkgs.nodejs_22;
            inherit coreWasm;
          };
          tamAuth = pkgs.callPackage ./nix/tam-auth.nix { nodejs = pkgs.nodejs_22; };
          teachouseMigrations = pkgs.callPackage ./nix/migrations.nix { };
        in
        {
          packages = {
            default = bin;
            tam-server = serviceBin "tam-server";
            tam-worker = serviceBin "tam-worker";
            tam-auth = tamAuth;
            teachouse-console = teachouseConsole;
            teachouse-migrations = teachouseMigrations;
            teachouse-core-wasm = coreWasm;
          };

          checks = {
            inherit bin;

            # The two deployed artefacts no Rust lane covers. The console's own
            # source gates — the vocabulary freshness diff, svelte-check and
            # vitest — stay in `just web-check`, because they judge the source
            # rather than the artefact; this proves the artefact builds.
            console = teachouseConsole;
            tam-auth = tamAuth;

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
            # Without this, sqlx's compile-time query macros connect to the
            # DATABASE_URL in .env, so a bare `cargo check` in this shell
            # validates against the development database. The justfile exports
            # the same value, and db-prepare and db-verify set it back to false
            # where a live connection is the point.
            SQLX_OFFLINE = "true";
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

          # D29's Android route: the SDK, the NDK and a JDK from androidenv
          # rather than from an Android Studio install, which the Tauri
          # prerequisites page is otherwise the only documented way to get.
          # Deliberately not `craneLib.devShell`: this shell builds for a
          # foreign target through cargo-ndk and Gradle, and crane's host-target
          # environment is not what that wants.
          devShells.android = pkgsAndroid.mkShell {
            packages = [
              androidRustToolchain
              androidComposition.androidsdk
              pkgs.cargo-tauri
              pkgs.jdk17
              pkgs.just
              pkgs.nodejs_22
            ];
            ANDROID_HOME = androidSdkRoot;
            ANDROID_SDK_ROOT = androidSdkRoot;
            ANDROID_NDK_ROOT = androidNdkRoot;
            NDK_HOME = androidNdkRoot;
            JAVA_HOME = pkgs.jdk17.home;
            # The Android Gradle Plugin resolves aapt2 from Maven by default,
            # and that copy is an unpatched ELF binary a NixOS host cannot
            # execute. Pointing it at the SDK's own, which androidenv has
            # patched, is the nixpkgs-documented override.
            GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidSdkRoot}/build-tools/${androidBuildToolsVersion}/aapt2";
          };

          formatter = pkgs.nixfmt-rfc-style;
        };
    };
}
