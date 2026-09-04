# The teachouse.io landing page as a store path: the static Astro build that
# `services.teachouse.landing.package` serves at the origin root.
#
# It is a separate build from the console for the reason recorded in
# `apps/landing/README.md`, and it takes no build-time argument, because
# nothing on the page is configured: there is no checkout, no captcha and no
# social provider, and every price is a literal in `src/pricing.js`.
{
  lib,
  stdenv,
  nodejs,
  importNpmLock,
}:
stdenv.mkDerivation {
  pname = "teachouse-landing";
  version = "0.0.0";

  # A developer's tree carries `node_modules`, `dist` and `.astro`, all three
  # gitignored and together a quarter of a gigabyte. Copying them in would put
  # a locally-resolved `node_modules` in front of the one `npmConfigHook`
  # installs, so the filter is correctness rather than only store hygiene.
  src = lib.cleanSourceWith {
    src = ../apps/landing;
    name = "teachouse-landing-source";
    filter =
      path: _type:
      let
        base = baseNameOf path;
      in
      !(builtins.elem base [
        "node_modules"
        "dist"
        ".astro"
      ]);
  };

  nativeBuildInputs = [
    nodejs
    importNpmLock.npmConfigHook
  ];

  # As in `console.nix`: the lockfile is read directly rather than through
  # `npmRoot`, so the resolved dependency set depends on the two manifests and
  # not on every file under `apps/landing`.
  npmDeps = importNpmLock.importNpmLock {
    package = lib.importJSON ../apps/landing/package.json;
    packageLock = lib.importJSON ../apps/landing/package-lock.json;
  };

  env.ASTRO_TELEMETRY_DISABLED = "1";

  buildPhase = ''
    runHook preBuild
    npm run build
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    # tam-server serves this directory as it stands, so an adapter or output
    # change that stopped emitting any of these has to fail here rather than as
    # a 404 on the box. Every page is linked from every footer, and the script
    # is referenced from every head.
    for emitted in \
      index.html \
      pricing/index.html \
      privacy/index.html \
      terms/index.html \
      app-redirect.js; do
      test -f "dist/$emitted"
    done
    cp -r dist $out
    runHook postInstall
  '';

  meta = {
    description = "Teachouse landing page, a static Astro build served at the origin root";
    platforms = lib.platforms.all;
  };
}
