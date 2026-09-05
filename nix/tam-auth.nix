# The identity service as a store path.
#
# There is no build step, and that is a property of the runtime rather than an
# omission: `auth/package.json` requires node >= 22.18, the release that strips
# TypeScript types without a flag, and every import in `auth/src` carries its
# `.ts` extension, which is what that stripping requires. So this derivation
# resolves node_modules and places the sources beside it.
#
# With `runTests`, the same derivation also runs `npm test` in the sandbox. It
# is a parameter rather than an unconditional `doCheck` so that a deployment
# does not wait on the test run, and it is this file rather than a separate one
# so that the tests execute against the node_modules the deployed artefact gets
# -- a test lane resolved from a different lock would prove something about a
# tree nobody ships.
{
  lib,
  stdenv,
  nodejs,
  importNpmLock,
  makeWrapper,
  runTests ? false,
}:
stdenv.mkDerivation {
  pname = if runTests then "tam-auth-test" else "tam-auth";
  version = "0.0.0";

  src = ../auth;

  nativeBuildInputs = [
    nodejs
    importNpmLock.npmConfigHook
    makeWrapper
  ];

  npmDeps = importNpmLock.importNpmLock {
    package = lib.importJSON ../auth/package.json;
    packageLock = lib.importJSON ../auth/package-lock.json;
  };

  dontBuild = true;

  doCheck = runTests;

  # `npm test` rather than `node --test` with the file list repeated here: the
  # script's own guard checks that list against the files on disk, and a copy
  # of it in this file would be the thing that silently drifts.
  checkPhase = ''
    runHook preCheck
    npm test
    runHook postCheck
  '';

  installPhase = ''
    runHook preInstall
    mkdir -p $out/lib/tam-auth
    cp -r src node_modules package.json $out/lib/tam-auth/
    makeWrapper ${lib.getExe nodejs} $out/bin/tam-auth \
      --add-flags $out/lib/tam-auth/src/server.ts
    runHook postInstall
  '';

  meta = {
    description = "Platform identity and browser session for Teachouse (better-auth over /api/auth/*)";
    mainProgram = "tam-auth";
    platforms = lib.platforms.all;
  };
}
