# `teachouse-edge-warm`: fetches every hashed console and landing asset once
# through the public hosts so Cloudflare's edge holds them before a teacher
# asks (`edge-warm.sh` says why). The two builds are inputs, so the list it
# walks is the one the deployment serves; a console built with other
# arguments (`teachouse-console.override { ... }`) has other hashes and is
# passed in here the same way, as the module's `warmEdge` does.
#
# The same binary is what a scheduler outside NixOS calls -- a Kubernetes
# CronJob, say -- which is why it carries its own CA bundle rather than
# relying on the host's `/etc/ssl`.
{
  lib,
  writeShellApplication,
  curl,
  coreutils,
  findutils,
  gnused,
  gawk,
  gnugrep,
  cacert,
  consolePackage,
  # Null warms the console alone.
  landingPackage ? null,
}:
writeShellApplication {
  name = "teachouse-edge-warm";
  runtimeInputs = [
    curl
    coreutils
    findutils
    gnused
    gawk
    gnugrep
  ];
  # Defaults only: `--ui-dir`/`--landing-dir` and an `SSL_CERT_FILE` the
  # caller sets win.
  text = ''
    export TEACHOUSE_EDGE_WARM_UI_DIR="''${TEACHOUSE_EDGE_WARM_UI_DIR:-${consolePackage}}"
    export TEACHOUSE_EDGE_WARM_LANDING_DIR="''${TEACHOUSE_EDGE_WARM_LANDING_DIR:-${
      lib.optionalString (landingPackage != null) "${landingPackage}"
    }}"
    export SSL_CERT_FILE="''${SSL_CERT_FILE:-${cacert}/etc/ssl/certs/ca-bundle.crt}"
  ''
  + builtins.readFile ./edge-warm.sh;
  meta = {
    description = "Fetch every hashed Teachouse asset through the edge once";
    mainProgram = "teachouse-edge-warm";
  };
}
