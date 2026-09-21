# The console as a store path: the SvelteKit single-page application that
# `tam-server --ui-dir` serves as its router fallback.
#
# The committed tree cannot build on its own. `web/.gitignore` excludes
# `src/lib/core/generated`, which `just web-wasm` produces by building
# `tam-core-wasm` for wasm32 and running wasm-bindgen over it, so the caller
# passes that tree in as `coreWasm` and it is placed before vite runs.
{
  lib,
  stdenv,
  nodejs,
  importNpmLock,
  coreWasm,
  # Every value below is substituted into the bundle the browser downloads, so
  # none of them is a secret. `web/src/lib/captcha.ts` and
  # `social-providers.ts` each read one at build time and treat an absent
  # value as the feature being off, which is why the defaults are empty: a
  # console built with no argument has a dormant widget and no social buttons.
  #
  # Billing carries no build-time value at all. It did until the rail moved to
  # Stripe: the console now posts a price key to its own origin and follows
  # the URL the server answers, so there is no publishable token and no price
  # map in the bundle, and the two-sided key-rename hazard between them went
  # with it.
  turnstileSiteKey ? "",
  socialProviders ? "",
  # The PostHog project key. Public by PostHog's own description (the browser
  # ships it) and off when empty: `posthog.ts` never calls `init` without it.
  posthogKey ? "",
}:
stdenv.mkDerivation {
  pname = "teachouse-console";
  version = "0.0.0";

  src = ../web;

  nativeBuildInputs = [
    nodejs
    importNpmLock.npmConfigHook
  ];

  # The lockfile is read directly rather than through `npmRoot`, so the resolved
  # dependency set depends on the two manifests and not on every file under
  # `web/`; an edit to a Svelte component then rebuilds the console without
  # refetching node_modules.
  npmDeps = importNpmLock.importNpmLock {
    package = lib.importJSON ../web/package.json;
    packageLock = lib.importJSON ../web/package-lock.json;
  };

  env = {
    VITE_TURNSTILE_SITE_KEY = turnstileSiteKey;
    VITE_SOCIAL_PROVIDERS = socialProviders;
    PUBLIC_POSTHOG_KEY = posthogKey;
  };

  preBuild = ''
    mkdir -p src/lib/core/generated
    cp ${coreWasm}/* src/lib/core/generated/
  '';

  buildPhase = ''
    runHook preBuild
    npm run build
    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall
    # tam-server reads index.html once at start-up and refuses to serve without
    # it, so an adapter change that stopped emitting the single-page shell has to
    # fail here rather than on the box at the first request.
    test -f build/index.html
    cp -r build $out
    runHook postInstall
  '';

  meta = {
    description = "Teachouse console, built for one origin behind tam-server --ui-dir";
    platforms = lib.platforms.all;
  };
}
