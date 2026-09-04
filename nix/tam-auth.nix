# The identity service as a store path.
#
# There is no build step, and that is a property of the runtime rather than an
# omission: `auth/package.json` requires node >= 22.18, the release that strips
# TypeScript types without a flag, and every import in `auth/src` carries its
# `.ts` extension, which is what that stripping requires. So this derivation
# resolves node_modules and places the sources beside it.
{
  lib,
  stdenv,
  nodejs,
  importNpmLock,
  makeWrapper,
}:
stdenv.mkDerivation {
  pname = "tam-auth";
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
