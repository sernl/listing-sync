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
  # script tests: an empty path means no token is configured and the Android
  # half is deliberately absent, which is not the same as failing to read one.
  downloadsTokenFile =
    if cfg.downloads.githubReleaseTokenFile == null then "" else cfg.downloads.githubReleaseTokenFile;

  # The one unit in this module that reaches the internet, and the reason the
  # download surface is two pieces rather than one. tam-server denies IP egress
  # outright and must keep doing so, so it cannot fetch a release; this fetches,
  # verifies and publishes, and tam-server only reads what it finds.
  #
  # Nothing is published that did not verify, and nothing that verified once is
  # removed because something else failed later. Each component — the Windows
  # installer, the Android package and its two sums files — updates its own
  # entry only when its bytes were fetched and its digest checked by name; a
  # component that did not is left exactly as it was, the manifest keeps naming
  # it, the cleanup keeps it, and the run ends non-zero. The alternative, which
  # this had, was a listing failure deleting a working Android download and
  # publishing `"android": null` — taking the surface down to fix nothing.
  #
  # Staging is `.staging/` inside the served directory: the same filesystem, so
  # the rename out of it is atomic, and a name tam-server refuses, so nothing
  # half-written is ever reachable. Each file is put in place by a rename and
  # `downloads.json` goes last, so no client reads a manifest naming a file that
  # is not there yet.
  downloadsScript = pkgs.writeShellApplication {
    name = "teachouse-downloads-refresh";
    runtimeInputs = [
      pkgs.curl
      pkgs.jq
      pkgs.coreutils
      pkgs.gawk
      pkgs.findutils
    ];
    text = ''
      target=${lib.escapeShellArg downloadsDir}
      repository=${lib.escapeShellArg cfg.downloads.repository}
      token_file=${lib.escapeShellArg downloadsTokenFile}
      channel=${lib.escapeShellArg cfg.downloads.updateUrl}
      prerelease_ok=${lib.boolToString cfg.downloads.prerelease}

      # Inside the served directory so that a rename out of it is atomic, and
      # named with a leading dot because tam-server refuses every download name
      # that begins with one. Cleared first: a run killed between writing and
      # renaming leaves partial files here, and they are this run's to clear.
      staging="$target/.staging"
      rm -rf "$staging"
      mkdir -p "$staging"

      work="$(mktemp -d)"
      trap 'rm -rf "$work"' EXIT

      # Two caps rather than one. --max-time bounds the whole operation
      # including retries, so a single value big enough for a sixty-megabyte
      # package would also let a hung metadata request sit for half an hour.
      # The file cap carries a stall check as well: below ten kilobytes a second
      # for a minute the transfer is not progressing and the retry is the point.
      fetch_json() { curl --fail --silent --show-error --location --max-time 60 --retry 2 "$@"; }
      fetch_file() {
          curl --fail --silent --show-error --location --max-time 1800 --retry 2 \
               --speed-limit 10240 --speed-time 60 "$@"
      }

      # What is published now. Every component below updates its own entry only
      # when its bytes were fetched and verified; anything that did not is left
      # exactly as it is, and the run ends non-zero. A degraded refresh that
      # deleted a working download would take the surface down to fix nothing.
      manifest="$target/downloads.json"
      # A manifest that will not parse yields nothing rather than aborting the
      # run: errexit fires on a failed command substitution in an assignment, so
      # without this a single corrupt file would lock the surface at that file
      # for good. Empty previous values mean this run republishes from scratch.
      previous() {
          if [ -f "$manifest" ]; then jq -r "$1 // empty" "$manifest" 2>/dev/null || true; fi
      }
      windows_file="$(previous .windows.file)"
      windows_sum="$(previous .windows.sha256)"
      windows_version="$(previous .version)"
      android_file="$(previous .android.file)"
      android_sum="$(previous .android.sha256)"
      android_version="$(previous .android.version)"
      linux_file="$(previous .linux.file)"
      linux_sum="$(previous .linux.sha256)"
      linux_version="$(previous .linux.version)"

      degraded=0
      # Whether this run has anything new to say. `refreshed_at` is the time the
      # published set last changed, so a run where every component failed leaves
      # the manifest alone rather than restamping a week-old set as fresh — the
      # one field a console would read to decide whether the surface is stale.
      changed=0
      staged=()

      # The digest a sums file records for one exact name, or nothing.
      #
      # By name rather than through `sha256sum --check --ignore-missing`, which
      # asserts only that *something* named in the file was present and matched
      # — true whenever any other artefact of the release happens to sit in the
      # same directory, and therefore no evidence at all about the file at hand.
      # sha256sum writes `HASH  NAME` in text mode and `HASH *NAME` in binary
      # mode; the name starts at column 67 either way, so both are read here and
      # a sums file written with -b does not silently skip the check. The name
      # is the path sha256sum was given, so a file summed as `./NAME` — which
      # is how every release from v0.1.1 to v0.4.0 wrote it — is read as NAME:
      # the leading `./` names the same file and carries no information, and
      # refusing it left the manifest stuck on the last release that matched.
      expected_digest() {
          gawk -v want="$2" '
              length($0) >= 67 && !found {
                  name = substr($0, 67)
                  sub(/^\*/, "", name)
                  sub(/^\.\//, "", name)
                  if (name == want) { print substr($0, 1, 64); found = 1 }
              }
              END { exit(found ? 0 : 1) }
          ' "$1"
      }

      # A staged file against the digest its own name has in the sums file.
      verify_staged() {
          local sums="$1" name="$2" digest
          if ! digest="$(expected_digest "$sums" "$name")"; then
              echo "$(basename "$sums") carries no digest line for $name" >&2
              return 1
          fi
          ( cd "$staging" && printf '%s  %s\n' "$digest" "$name" | sha256sum --check --strict --status - )
      }

      # The release assets, which exist only where a token does. Fetched before
      # the Windows half, because that half verifies against the sums this
      # brings back.
      release_tag=""
      new_android_file=""
      github_assets() {
          # Read on its own line, where errexit fires. Inside a command
          # substitution in an argument position a failure is discarded, which
          # turns an unreadable secret into `Authorization: Bearer ` and a
          # misconfiguration that stays invisible for exactly as long as the
          # repository stays public.
          local token
          token="$(cat "$token_file")" || {
              echo "$token_file could not be read; it must exist and be readable by the refresh account" >&2
              return 1
          }
          if [ -z "$token" ]; then
              echo "$token_file is empty; it holds a GitHub token with read access to releases" >&2
              return 1
          fi
          # A subshell, so the unit's own 0077 is restored after it rather than
          # left widened for the rest of the run to undo.
          ( umask 077
            printf 'header = "Authorization: Bearer %s"\n' "$token" > "$work/curl.conf" ) || return 1

          fetch_json --config "$work/curl.conf" \
                     --header "Accept: application/vnd.github+json" \
                     --header "X-GitHub-Api-Version: 2022-11-28" \
                     --output "$work/releases.json" \
                     -- "https://api.github.com/repos/$repository/releases?per_page=20" || return 1

          # By version rather than by publish date, never a draft, and a
          # prerelease only where `downloads.prerelease` says so. A hotfix
          # to an older line published after a newer release would otherwise
          # regress the public download; the release workflow marks every
          # channelled release a prerelease, so a refresh reading a channel
          # has to accept them or it publishes no Android package at all.
          # Twenty rather than five, so a run of prereleases cannot push
          # every conforming tag off the page.
          local release
          release="$(jq -c --argjson prerelease_ok "$prerelease_ok" '
              [ .[]
                | select(.draft | not)
                | select((.prerelease | not) or $prerelease_ok)
                | select(.tag_name | test("^v[0-9]+\\.[0-9]+\\.[0-9]+$")) ]
              | sort_by(.tag_name | ltrimstr("v") | split(".") | map(tonumber))
              | last // empty
          ' "$work/releases.json")" || return 1
          if [ -z "$release" ]; then
              echo "no release of $repository carries a released vN.N.N tag" >&2
              return 1
          fi
          release_tag="$(printf '%s' "$release" | jq -r '.tag_name')"
          new_android_file="$(printf '%s' "$release" | jq -r '
              [ .assets[] | select(.name | endswith(".apk")) ] | first | .name // empty
          ')"
          if [ -z "$new_android_file" ]; then
              echo "release $release_tag carries no .apk asset" >&2
              return 1
          fi

          local asset url destination
          for asset in "$new_android_file" SHA256SUMS.txt SHA256SUMS-android.txt; do
              url="$(printf '%s' "$release" | jq -r --arg name "$asset" '
                  [ .assets[] | select(.name == $name) ] | first | .url // empty
              ')" || return 1
              if [ -z "$url" ]; then
                  echo "release $release_tag carries no asset named $asset" >&2
                  return 1
              fi
              # The sums land in the work directory and the package in staging:
              # only what is published is staged.
              if [ "$asset" = "$new_android_file" ]; then
                  destination="$staging/$asset"
              else
                  destination="$work/$asset"
              fi
              # The asset API url with this Accept is the only way to read an
              # asset of a private release; the browser url is not one.
              fetch_file --config "$work/curl.conf" \
                         --header "Accept: application/octet-stream" \
                         --output "$destination" \
                         -- "$url" || return 1
          done

          # The Linux AppImage rides the same release. Absent on a release
          # older than the Linux job, which is not a failure of the Android
          # half: the Linux entry is simply not published from that release.
          new_linux_file="$(printf '%s' "$release" | jq -r '
              [ .assets[] | select(.name | endswith(".AppImage")) ] | first | .name // empty
          ')"
          if [ -z "$new_linux_file" ]; then
              echo "release $release_tag carries no .AppImage asset; the Linux half is not published from it" >&2
              return 0
          fi
          for asset in "$new_linux_file" SHA256SUMS-linux.txt; do
              url="$(printf '%s' "$release" | jq -r --arg name "$asset" '
                  [ .assets[] | select(.name == $name) ] | first | .url // empty
              ')" || return 1
              if [ -z "$url" ]; then
                  echo "release $release_tag carries no asset named $asset; the Linux half is not published from it" >&2
                  new_linux_file=""
                  return 0
              fi
              if [ "$asset" = "$new_linux_file" ]; then
                  destination="$staging/$asset"
              else
                  destination="$work/$asset"
              fi
              fetch_file --config "$work/curl.conf" \
                         --header "Accept: application/octet-stream" \
                         --output "$destination" \
                         -- "$url" || { new_linux_file=""; return 0; }
          done
      }

      windows_component() {
          fetch_json --output "$work/update.json" -- "$channel" || return 1
          local channel_version url candidate
          channel_version="$(jq -r '.version // empty' "$work/update.json")" || return 1
          url="$(jq -r '.url // empty' "$work/update.json")" || return 1
          if [ -z "$channel_version" ] || [ -z "$url" ]; then
              echo "the update endpoint named no version or no url" >&2
              return 1
          fi
          # Named what the vendor named it rather than composed here, so the
          # file a seller receives carries the publisher's own name. A url that
          # does not end in one gets a composed name rather than a query string.
          candidate="$(basename "''${url%%\?*}")"
          case "$candidate" in
              *.exe) ;;
              *) candidate="Teachouse_''${channel_version}_x64-setup.exe" ;;
          esac
          fetch_file --output "$staging/$candidate" -- "$url" || return 1

          # The installer comes from the update channel and the sums from a
          # GitHub release, so the two can sit at different versions. Where a
          # sums file exists it must name this file: publishing an executable
          # that a sums file declines to vouch for is the one outcome a download
          # surface must not have, and a channel that has run ahead of the
          # release is a state to fix rather than to publish through. Where no
          # sums file exists at all — no token configured — it is published on
          # the vendor's TLS alone, which is what that half of the design has
          # always said.
          if [ -f "$work/SHA256SUMS.txt" ]; then
              verify_staged "$work/SHA256SUMS.txt" "$candidate" || return 1
          else
              echo "no release sums are available, so $candidate is published on the vendor's TLS alone" >&2
          fi
          windows_file="$candidate"
          windows_version="$channel_version"
          windows_sum="$(sha256sum "$staging/$candidate" | cut -d' ' -f1)"
          staged+=("$candidate")
          changed=1
      }

      android_component() {
          verify_staged "$work/SHA256SUMS-android.txt" "$new_android_file" || return 1
          cp -f "$work/SHA256SUMS.txt" "$work/SHA256SUMS-android.txt" "$staging/" || return 1
          android_file="$new_android_file"
          android_version="''${release_tag#v}"
          android_sum="$(sha256sum "$staging/$new_android_file" | cut -d' ' -f1)"
          staged+=("$new_android_file" SHA256SUMS.txt SHA256SUMS-android.txt)
          changed=1
      }

      linux_component() {
          [ -n "''${new_linux_file:-}" ] || return 1
          verify_staged "$work/SHA256SUMS-linux.txt" "$new_linux_file" || return 1
          cp -f "$work/SHA256SUMS-linux.txt" "$staging/" || return 1
          linux_file="$new_linux_file"
          linux_version="''${release_tag#v}"
          linux_sum="$(sha256sum "$staging/$new_linux_file" | cut -d' ' -f1)"
          staged+=("$new_linux_file" SHA256SUMS-linux.txt)
          changed=1
      }

      new_linux_file=""
      if [ -n "$token_file" ]; then
          if github_assets && android_component; then
              echo "android: $android_file from $release_tag"
          else
              echo "the Android half did not refresh; the previous one keeps serving" >&2
              degraded=1
          fi
          if linux_component; then
              echo "linux: $linux_file from $release_tag"
          else
              echo "the Linux half did not refresh; the previous one keeps serving" >&2
          fi
      else
          # Deliberately absent rather than degraded: no token is a
          # configuration, and the previous Android and Linux entries go
          # with it.
          echo "no GitHub token configured; the Android and Linux halves are not published"
          if [ -n "$android_file" ] || [ -n "$linux_file" ]; then changed=1; fi
          android_file=""
          android_sum=""
          android_version=""
          linux_file=""
          linux_sum=""
          linux_version=""
      fi

      if windows_component; then
          echo "windows: $windows_file at $windows_version"
      else
          echo "the Windows half did not refresh; the previous one keeps serving" >&2
          degraded=1
      fi

      if [ -z "$windows_file" ]; then
          echo "no Windows installer has ever been published here and this run fetched none" >&2
          exit 1
      fi

      if [ "$changed" -eq 0 ] && [ -f "$manifest" ]; then
          echo "nothing changed; the published set and its manifest are left as they are" >&2
          rm -rf "$staging"
          exit 1
      fi

      jq -n \
         --arg version "$windows_version" \
         --arg windows_file "$windows_file" \
         --arg windows_sum "$windows_sum" \
         --arg linux_file "$linux_file" \
         --arg linux_sum "$linux_sum" \
         --arg linux_version "$linux_version" \
         --arg android_file "$android_file" \
         --arg android_sum "$android_sum" \
         --arg android_version "$android_version" \
         --arg refreshed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
         '{
            version: $version,
            windows: { file: $windows_file, sha256: $windows_sum },
            linux: (if $linux_file == "" then null
                    else { file: $linux_file, sha256: $linux_sum, version: $linux_version } end),
            android: (if $android_file == "" then null
                      else { file: $android_file, sha256: $android_sum, version: $android_version } end),
            apple: null,
            refreshed_at: $refreshed_at
          }' > "$staging/downloads.json"

      # Renamed inside the target directory, so no client sees a partial file,
      # and the manifest last, so no client reads one naming a file that is not
      # there yet. The mode is stated rather than inherited: the unit's umask is
      # 0077 and tam-server reads these as a different account.
      publish() {
          chmod 0644 "$staging/$1"
          mv -f "$staging/$1" "$target/$1"
      }
      for file in "''${staged[@]}"; do publish "$file"; done
      publish downloads.json

      # Only now, and only names nothing published still refers to. A file the
      # manifest names is kept whether or not this run fetched it, which is what
      # makes a degraded refresh leave a working download alone. The predicate
      # is `! -type d` rather than `-type f`, so a symlink planted here is
      # removed rather than skipped over.
      keep=(downloads.json "$windows_file")
      if [ -n "$android_file" ]; then
          keep+=("$android_file" SHA256SUMS.txt SHA256SUMS-android.txt)
      fi
      if [ -n "$linux_file" ]; then
          keep+=("$linux_file" SHA256SUMS-linux.txt)
      fi
      find "$target" -maxdepth 1 ! -type d -printf '%f\0' |
          while IFS= read -r -d "" existing; do
              keeping=0
              for name in "''${keep[@]}"; do
                  if [ "$existing" = "$name" ]; then keeping=1; break; fi
              done
              if [ "$keeping" -eq 0 ]; then rm -f -- "$target/$existing"; fi
          done

      rm -rf "$staging"

      if [ "$degraded" -ne 0 ]; then
          echo "one or more components did not refresh; their previous files keep serving" >&2
          exit 1
      fi
      echo "published: ''${keep[*]}"
    '';
  };
in
{
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
        The GitHub repository whose releases carry the Android package, as
        `owner/name`. Only the Android half reads this; the Windows installer
        comes from `updateUrl`, which names no repository.
      '';
    };

    updateUrl = lib.mkOption {
      type = lib.types.str;
      default = "https://cdn.crabnebula.app/update/teachouse/teachouse/windows-x86_64/0.0.0?channel=beta";
      description = ''
        The vendor update endpoint the Windows installer is taken from, which
        is the same one the desktop updater reads. It answers one JSON object
        naming the current version and where its installer is.

        The zero in the path is the asking client's version, which the
        endpoint compares against; zero means "what is current", which is what
        a publisher wants rather than an updater. The platform and the channel
        are both in this url, so moving from the beta channel to a stable one,
        or publishing a different platform's installer, is a change here
        rather than a change to this module.
      '';
    };

    prerelease = lib.mkOption {
      type = lib.types.bool;
      default = lib.hasInfix "channel=" cfg.downloads.updateUrl;
      defaultText = lib.literalExpression ''lib.hasInfix "channel=" config.services.teachouse.downloads.updateUrl'';
      description = ''
        Whether a release GitHub marks as a pre-release may be published.

        The release workflow marks every release it publishes to a named
        channel as a pre-release, so that a beta never presents itself as
        the repository's latest release. A refresh whose `updateUrl` names
        that channel serves exactly those releases on its Windows half, so
        refusing them here left the Android half empty while the Windows
        installer of the same release was published beside it. The default
        therefore follows `updateUrl`: a channelled url accepts pre-releases
        and an unchannelled one does not. While pre-releases are accepted,
        nothing on the repository separates a beta from a package cut for
        internal testing, so such a package must be a draft rather than a
        pre-release.
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
        releases, as a bare token on one line. Null, the default, disables the
        Android half: the refresh still runs, the Windows installer is still
        published, and `downloads.json` carries `"android": null`.

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

    interval = lib.mkOption {
      type = lib.types.str;
      default = "15m";
      example = "1h";
      description = ''
        How often the refresh runs, as a systemd time span. A release becomes
        downloadable within one interval of being published; nothing else
        depends on the value.
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
    # reach is a vendor CDN and GitHub's release API, and what it can write is
    # one directory tam-server reads.
    systemd.services.teachouse-downloads-refresh = {
      description = "Teachouse desktop download refresh";
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];
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
        # nothing. Randomised so a fleet does not arrive at the vendor's CDN
        # together.
        OnBootSec = "2m";
        OnUnitActiveSec = cfg.downloads.interval;
        RandomizedDelaySec = "60s";
        Unit = "teachouse-downloads-refresh.service";
      };
    };
  };
}
