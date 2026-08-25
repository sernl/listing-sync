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

          # cleanCargoSource would drop sqlx's offline query metadata and the
          # SQL migrations, which the sandboxed build needs.
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter =
              path: type:
              (craneLib.filterCargoSources path type)
              || (builtins.match ".*/\\.sqlx/query-.*\\.json" path != null)
              || (builtins.match ".*/migrations/.*\\.sql" path != null);
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
            clippy = craneLib.cargoClippy (
              commonArgs
              // {
                inherit cargoArtifacts;
                cargoClippyExtraArgs = "--all-targets -- --deny warnings";
              }
            );
            fmt = craneLib.cargoFmt { inherit src; };
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
              pkgs.shellcheck
              pkgs.sqlx-cli
            ];
          };

          formatter = pkgs.nixfmt-rfc-style;
        };
    };
}
