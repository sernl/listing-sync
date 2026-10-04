# The desktop download surface, on its own so a host can mirror the installers
# without serving the product: a oneshot on a timer that fetches, verifies and
# publishes each release into one directory. `nixosModules.teachouse` imports
# this and has tam-server serve that directory at `/downloads/`; a host whose
# tam-server runs elsewhere imports only this and serves the directory itself.
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.teachouse;
  downloadsUser = "teachouse-downloads";
  group = "teachouse";
  downloadsDir = cfg.downloads.directory;

  # Empty rather than null at the shell boundary, because that is the shape the
  # script tests.
  orEmpty = value: if value == null then "" else value;

  # The one unit in this module that reaches the internet, and the reason the
  # download surface is two pieces rather than one. tam-server denies IP egress
  # outright and must keep doing so, so it cannot fetch a release; this fetches,
  # verifies and publishes, and tam-server only reads what it finds.
  #
  # The script is its own file so the same bytes run by hand against a
  # directory of local artefacts; its header states the rules it keeps.
  downloadsScript = pkgs.writeShellApplication {
    name = "teachouse-downloads-refresh";
    runtimeInputs = [
      pkgs.curl
      pkgs.jq
      pkgs.coreutils
      pkgs.gawk
      pkgs.findutils
    ];
    text = builtins.readFile ./downloads-refresh.sh;
  };

  # A store listing is a url a seller is sent to, so only https is accepted.
  storeLink =
    description:
    lib.mkOption {
      type = lib.types.nullOr (lib.types.strMatching "https://[^[:space:]]+");
      default = null;
      inherit description;
    };
in
{
  imports = [
    # The Windows installer used to come from the CrabNebula update endpoint
    # this option named. Every platform now comes from the GitHub release, so
    # a host still setting it is told why rather than evaluated into a
    # surprise.
    (lib.mkRemovedOptionModule [
      "services"
      "teachouse"
      "downloads"
      "updateUrl"
    ] "Every platform is now mirrored from the GitHub release; see docs/notes/runbooks/release.md.")
  ];

  options.services.teachouse.downloads = {
    directory = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/teachouse-downloads";
      readOnly = true;
      description = "Where the refresh publishes and tam-server reads. A sibling of /var/lib/teachouse: tam-server's StateDirectory chowns that one to the API account at 0700, so a directory inside it would be fought over on every start.";
    };
    enable = lib.mkEnableOption ''
      the desktop download surface: a oneshot on a timer that fetches the
      published installers, and `/downloads/` on the API origin serving what
      it fetched'';

    repository = lib.mkOption {
      # Two segments and nothing that would change the request: a value
      # carrying `?` or `#` is interpolated into an API url and would silently
      # ask a different question.
      type = lib.types.strMatching "[A-Za-z0-9._-]+/[A-Za-z0-9._-]+";
      default = "sernl/listing-sync";
      description = ''
        The GitHub repository whose newest release is mirrored, as
        `owner/name`. Every platform — the Windows installer and MSI, the macOS
        disk image, the Linux AppImage and .deb, the Android package and the
        Microsoft Store installer — comes from that release and is verified
        against the `SHA256SUMS` files it carries.
      '';
    };

    prerelease = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Whether a release GitHub marks as a pre-release may be published.

        The release workflow marks every release it cuts to a CrabNebula
        channel (the `CN_CHANNEL` repository variable) as a pre-release, so a
        beta never presents itself as the repository's latest release. While
        releases go to the beta channel, which is the state today, refusing
        pre-releases here would mirror nothing; set this to false once releases
        go to production. While pre-releases are accepted, nothing on the
        repository separates a beta from a package cut for internal testing, so
        such a package must be a draft rather than a pre-release.
      '';
    };

    githubReleaseTokenFile = lib.mkOption {
      # `str`, not `path`, and for the reason every other secret-file option
      # in this module is `str`: `path` accepts a nix path literal, and a path
      # literal is copied into the world-readable store. This names a file on
      # the running machine.
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "/run/secrets/teachouse-github-release-token";
      description = ''
        A file holding a GitHub token with read access to the repository's
        releases, as a bare token on one line. Null asks the API anonymously,
        which answers for a public repository only: against a private one the
        refresh fails, says so, and every previously published file keeps
        serving.

        The file must be readable by the `teachouse-downloads` account — a
        sops-nix secret needs `owner = "teachouse-downloads"`, because the
        default `root:keys` is not enough. A refresh that cannot read it fails
        the run and says so rather than sending an empty bearer, which would
        keep working against a public repository and start failing the day it
        became private.

        A path rather than a value, like every other secret this module takes,
        because `/proc/<pid>/cmdline` and the unit file are both world
        readable. The refresh reads it into a mode-0600 curl config file and
        never echoes it.
      '';
    };

    stores = {
      googlePlay = storeLink ''
        The Google Play listing, published beside the Android package, e.g.
        `https://play.google.com/store/apps/details?id=io.teachouse.desktop`.
        Set it once the app is live on a public track: an internal-track
        listing answers 404 to everyone outside the tester list.
      '';
      microsoftStore = storeLink ''
        The Microsoft Store listing, published beside the Windows installers,
        e.g. `https://apps.microsoft.com/detail/<store-id>`. The Store ID is on
        the product's Partner Center overview once the first submission is
        certified.
      '';
      macAppStore = storeLink ''
        The Mac App Store listing, published beside the disk image, e.g.
        `https://apps.apple.com/app/id<apple-id>`. The Apple ID is under App
        Information in App Store Connect.
      '';
    };

    interval = lib.mkOption {
      type = lib.types.str;
      default = "15m";
      example = "1h";
      description = ''
        How often the refresh runs, as a systemd time span. A release becomes
        downloadable within one interval of being published, and the release
        workflow's Microsoft Store job waits up to two intervals for its
        installer to appear here.
      '';
    };
  };

  config = lib.mkIf cfg.downloads.enable {
    users.groups.${group} = { };
    users.users.${downloadsUser} = {
      isSystemUser = true;
      inherit group;
    };

    # Created here rather than by either unit's StateDirectory, and that is the
    # whole reason it is a sibling of /var/lib/teachouse: the refresh owns it
    # and tam-server only reads it, so no unit may chown it out from under the
    # other. It exists before either starts, which is what lets tam-server's
    # start-up check pass on a host whose first refresh has not run yet.
    systemd.tmpfiles.settings = {
      "10-teachouse-downloads".${downloadsDir}.d = {
        user = downloadsUser;
        inherit group;
        mode = "0755";
      };
    };

    # The one unit here allowed off the machine, and the only one. It holds no
    # database url, no marketplace credential and no seller session; what it can
    # reach is GitHub's release API and the storage its asset downloads redirect
    # to, and what it can write is one directory tam-server reads.
    systemd.services.teachouse-downloads-refresh = {
      description = "Teachouse desktop download refresh";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
      # Configuration only: the token is a path to a file the script reads, and
      # never a value here, because the unit file is world readable.
      environment = {
        DOWNLOADS_DIR = downloadsDir;
        DOWNLOADS_REPOSITORY = cfg.downloads.repository;
        DOWNLOADS_TOKEN_FILE = orEmpty cfg.downloads.githubReleaseTokenFile;
        DOWNLOADS_PRERELEASE = lib.boolToString cfg.downloads.prerelease;
        DOWNLOADS_STORE_GOOGLE_PLAY = orEmpty cfg.downloads.stores.googlePlay;
        DOWNLOADS_STORE_MICROSOFT = orEmpty cfg.downloads.stores.microsoftStore;
        DOWNLOADS_STORE_MAC_APP_STORE = orEmpty cfg.downloads.stores.macAppStore;
      };
      serviceConfig = {
        Type = "oneshot";
        User = downloadsUser;
        Group = group;
        ExecStart = lib.getExe downloadsScript;
        # The only writable path it has. The hardening below mounts the rest of
        # the filesystem read-only, and this unit needs exactly one directory.
        ReadWritePaths = [ downloadsDir ];
      }
      // import ./hardening.nix;
    };

    systemd.timers.teachouse-downloads-refresh = {
      description = "Teachouse desktop download refresh";
      wantedBy = [ "timers.target" ];
      timerConfig = {
        # `OnBootSec` is what makes a host that was off over a release catch up
        # shortly after boot rather than waiting out a whole interval. There is
        # deliberately no `Persistent` here: systemd.timer(5) says it has an
        # effect only on a timer configured with `OnCalendar`, and this one is
        # monotonic, so setting it would read as load-bearing while doing
        # nothing. Randomised so a fleet does not arrive at GitHub together.
        OnBootSec = "2m";
        OnUnitActiveSec = cfg.downloads.interval;
        RandomizedDelaySec = "60s";
        Unit = "teachouse-downloads-refresh.service";
      };
    };
  };
}
