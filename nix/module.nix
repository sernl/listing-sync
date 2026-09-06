# The deployment contract this repository exports: three long-running services,
# one migration runner ordered before them, the role and schema provisioning the
# development `db/init` files describe, and the backups the operational
# charter's section 10 requires.
#
# Every secret arrives as a path or as a `KEY=value` environment fragment, never
# as a value, with one deliberate exception named at its own option.
#
# The four unix identities below are part of the contract rather than an
# implementation detail: the fleet declares its safix service subjects against
# them, and the pg_ident map is what lets one identity authenticate as more than
# one postgres role. They are constants and not options because nothing
# downstream has a reason to choose other names, and three places would have to
# agree if it did.
{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.teachouse;
  packages = self.packages.${pkgs.stdenv.hostPlatform.system};

  apiUser = "teachouse-api";
  workerUser = "teachouse-worker";
  authUser = "teachouse-auth";
  migrateUser = "teachouse-migrate";
  downloadsUser = "teachouse-downloads";
  group = "teachouse";

  # A sibling of /var/lib/teachouse rather than a directory inside it, and the
  # reason is systemd's ownership rule rather than taste: tam-server's
  # StateDirectory already names `teachouse` and chowns it to the API account at
  # 0700, so a second unit declaring `teachouse/downloads` would fight it over
  # the parent on every start, and widening the parent to let a second account
  # in would widen the blob store beside it.
  downloadsDir = "/var/lib/teachouse-downloads";

  # Empty rather than null at the shell boundary, because that is the shape the
  # script tests: an empty path means no token is configured and the Android
  # half is deliberately absent, which is not the same as failing to read one.
  downloadsTokenFile =
    if cfg.downloads.githubReleaseTokenFile == null then "" else cfg.downloads.githubReleaseTokenFile;

  # Peer authentication over the unix socket, so no role password exists to be
  # read out of /proc. Every configuration value this workspace's binaries take
  # arrives as argv, and `/proc/<pid>/cmdline` is readable by every local
  # account, so a password-bearing url would publish the database to any process
  # on the box. Both client libraries resolve this form: sqlx routes a `host`
  # beginning with `/` to a socket and reads `user` from the query string
  # (sqlx-postgres `options/parse.rs`), and node-postgres takes both from the
  # search parameters (`pg-connection-string`).
  dbUrl = role: "postgres:///${cfg.database.name}?host=${cfg.database.socketDir}&user=${role}";

  # The two halves of `database.provision`, named once so the several places
  # that branch on it read as the same question.
  provisionsCluster = cfg.database.provision == "cluster";
  provisionsTenancy = cfg.database.provision != "none";

  # The completion mail is on exactly when a relay key is named; the binary
  # takes its five mail values together or not at all, and the assertions
  # below hold the rest of the set to this one switch.
  mailEnabled = cfg.server.mail.resendApiKeyFile != null;

  serverArgs = [
    (dbUrl "tam_app")
    "${cfg.server.bindAddress}:${toString cfg.server.port}"
    "--engine-db-url"
    (dbUrl "tam_engine")
    "--ui-dir"
    "${cfg.consolePackage}"
    "--auth-issuer"
    "https://${cfg.domain}"
    # The issuer is compared against the assertion's `iss`; the key set is only
    # fetched. Naming the loopback listener for the fetch keeps the identity
    # bridge off the public edge entirely, which is what lets this unit deny
    # every non-local address.
    "--auth-jwks-url"
    "http://127.0.0.1:${toString cfg.auth.port}/api/auth/jwks"
  ]
  ++ lib.optionals (cfg.landingPackage != null) [
    "--landing-dir"
    "${cfg.landingPackage}"
  ]
  ++ lib.optionals cfg.downloads.enable [
    "--downloads-dir"
    downloadsDir
  ]
  ++ lib.optionals cfg.server.backoffice [
    "--backoffice-db-url"
    (dbUrl "tam_backoffice")
  ]
  ++ lib.optionals (cfg.server.entitlementKeyFile != null) [
    "--entitlement-key-path"
    cfg.server.entitlementKeyFile
  ]
  ++ lib.optionals (cfg.server.entitlementPublicKey != null) [
    "--entitlement-public-key"
    cfg.server.entitlementPublicKey
  ]
  ++ lib.optional cfg.server.requireEntitlementKey "--require-entitlement-key"
  ++ lib.optionals (cfg.server.blobKekFile != null) [
    "--blob-kek-path"
    cfg.server.blobKekFile
    "--blob-store-root"
    cfg.server.blobStoreRoot
  ]
  ++ lib.optionals (cfg.server.paddleWebhookSecret != null) [
    "--paddle-webhook-secret"
    cfg.server.paddleWebhookSecret
  ]
  ++ lib.optionals mailEnabled [
    "--resend-api-key-file"
    cfg.server.mail.resendApiKeyFile
    "--email-from"
    cfg.server.mail.from
    "--console-url"
    cfg.server.mail.consoleUrl
    "--auth-internal-url"
    cfg.server.mail.authInternalUrl
    "--auth-internal-secret-file"
    cfg.server.mail.authInternalSecretFile
  ];

  # The protections every one of the three services takes. Two are stated per
  # unit instead, because they differ for a reason rather than by oversight:
  # `MemoryDenyWriteExecute`, which a JIT cannot run under, and the address
  # filter, which only a service with no off-box work can close.
  hardening = {
    NoNewPrivileges = true;
    CapabilityBoundingSet = "";
    AmbientCapabilities = "";
    ProtectSystem = "strict";
    ProtectHome = true;
    PrivateTmp = true;
    PrivateDevices = true;
    ProtectClock = true;
    ProtectHostname = true;
    ProtectKernelTunables = true;
    ProtectKernelModules = true;
    ProtectKernelLogs = true;
    ProtectControlGroups = true;
    ProtectProc = "invisible";
    RestrictNamespaces = true;
    RestrictRealtime = true;
    RestrictSUIDSGID = true;
    LockPersonality = true;
    RemoveIPC = true;
    SystemCallArchitectures = "native";
    SystemCallFilter = [
      "@system-service"
      "~@privileged"
      "~@resources"
    ];
    RestrictAddressFamilies = [
      "AF_INET"
      "AF_INET6"
      "AF_UNIX"
      # glibc's resolver opens a netlink socket to enumerate the machine's own
      # addresses before it answers a lookup, so a list without this turns every
      # name resolution into a failure whose message names neither DNS nor this
      # setting. It is not an egress path — what these units may reach is
      # decided by IPAddressDeny below.
      "AF_NETLINK"
    ];
    UMask = "0077";
  };

  # tam-server and tam-worker reach nothing off this machine: `tam-api` carries
  # no HTTP client at all, tam-server's outbound calls are the key-set fetch
  # aimed at loopback above and the identity service's address route on the
  # same loopback, and tam-worker reaches no marketplace by decision D1. So the
  # charter's egress constraint is achievable on both as an actual deny rather
  # than an aspiration. The one exception is stated where it is taken: with the
  # completion mail configured, tam-server posts to the relay, which is off
  # this machine and resolves to no address a filter could name, so that unit
  # drops the filter and the constraint rests on the binary reaching nothing
  # else — which is what `--resend-api-key-file`'s own header records.
  loopbackOnly = {
    IPAddressDeny = "any";
    IPAddressAllow = "localhost";
  };

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
      # a sums file written with -b does not silently skip the check.
      expected_digest() {
          gawk -v want="$2" '
              length($0) >= 67 && !found {
                  name = substr($0, 67)
                  sub(/^\*/, "", name)
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

      if [ -n "$token_file" ]; then
          if github_assets && android_component; then
              echo "android: $android_file from $release_tag"
          else
              echo "the Android half did not refresh; the previous one keeps serving" >&2
              degraded=1
          fi
      else
          # Deliberately absent rather than degraded: no token is a
          # configuration, and the previous Android entry goes with it.
          echo "no GitHub token configured; the Android half is not published"
          if [ -n "$android_file" ]; then changed=1; fi
          android_file=""
          android_sum=""
          android_version=""
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
         --arg android_file "$android_file" \
         --arg android_sum "$android_sum" \
         --arg android_version "$android_version" \
         --arg refreshed_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
         '{
            version: $version,
            windows: { file: $windows_file, sha256: $windows_sum },
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

  migrateScript = pkgs.writeShellApplication {
    name = "teachouse-migrate";
    runtimeInputs = [
      pkgs.sqlx-cli
      config.services.postgresql.package
    ];
    text = ''
      appUrl=${lib.escapeShellArg (dbUrl "tam_app")}
      authUrl=${lib.escapeShellArg (dbUrl "tam_auth")}
      rust=${lib.escapeShellArg "${cfg.migrationsPackage}/rust"}

      # Printed before and after, so the deployed schema version is a fact in the
      # deploy record rather than an assumption.
      sqlx migrate info --source "$rust" --database-url "$appUrl"
      sqlx migrate run  --source "$rust" --database-url "$appUrl"
      sqlx migrate info --source "$rust" --database-url "$appUrl"

      # The identity set has its own ledger and its own applying role, so it
      # cannot ride sqlx's table. This is the shape `just auth-migrate` uses,
      # kept identical to it deliberately: one transaction over each file and its
      # ledger row, so neither can land without the other.
      auth_psql() { psql "$authUrl" -X -q -v ON_ERROR_STOP=1 "$@"; }
      auth_psql -c 'SET client_min_messages = warning; CREATE TABLE IF NOT EXISTS auth.applied_migration (name text PRIMARY KEY, applied_at timestamptz NOT NULL DEFAULT now())'
      auth_psql -c "INSERT INTO auth.applied_migration (name) SELECT '0001_identity.sql' WHERE to_regclass('auth.\"user\"') IS NOT NULL ON CONFLICT DO NOTHING"
      auth_psql -c "INSERT INTO auth.applied_migration (name) SELECT '0002_audit_event.sql' WHERE to_regclass('auth.auth_event') IS NOT NULL ON CONFLICT DO NOTHING"
      for file in ${cfg.migrationsPackage}/auth/*.sql; do
          name="$(basename "$file")"
          if [ -n "$(auth_psql -At -c "SELECT 1 FROM auth.applied_migration WHERE name = '$name'")" ]; then
              echo "already applied $name"
              continue
          fi
          echo "applying $name"
          auth_psql --single-transaction -f "$file" \
              -c "INSERT INTO auth.applied_migration (name) VALUES ('$name')"
      done
    '';
  };

  # What `db/init/*.sql` states that role attributes cannot: the database's
  # ownership, the schema the identity service owns, the search path that keeps
  # an unqualified name from escaping it, the revocation that is the boundary
  # written down, and the one crossing in the other direction. The role passwords
  # those files carry are development credentials and have no counterpart here.
  #
  # The ownership is transferred here rather than through `ensureDBOwnership`,
  # which grants a role the database that shares its name and so cannot express
  # `tam_app` owning `tam`. It is the same end state `CREATE DATABASE tam OWNER
  # tam_app` gives development, and the migration set needs it: the first
  # migration creates tables in `public`, whose rights follow the database owner.
  provisionScript = pkgs.writeShellApplication {
    name = "teachouse-provision";
    runtimeInputs = [ config.services.postgresql.package ];
    text = ''
      psql -d postgres -X -v ON_ERROR_STOP=1 <<SQL
      ALTER DATABASE ${cfg.database.name} OWNER TO tam_app;
      ALTER ROLE tam_auth SET search_path = auth;
      SQL
      psql -d ${lib.escapeShellArg cfg.database.name} -X -v ON_ERROR_STOP=1 <<'SQL'
      CREATE SCHEMA IF NOT EXISTS auth AUTHORIZATION tam_auth;
      REVOKE ALL ON SCHEMA public FROM tam_auth;
      GRANT USAGE ON SCHEMA auth TO tam_app;
      SQL
    '';
  };
in
{
  options.services.teachouse = {
    enable = lib.mkEnableOption "the Teachouse control plane: API, console, identity service and maintenance worker";

    domain = lib.mkOption {
      type = lib.types.str;
      example = "teachouse.stowiq.io";
      description = ''
        The single public origin. The API, the console and `/api/auth/*` all
        answer here, which is what `tam-server --ui-dir` exists for and what
        makes the session cookie, the CORS posture and the passkey relying-party
        identifier one value each rather than three.
      '';
    };

    package = lib.mkOption {
      type = lib.types.package;
      default = packages.tam-server;
      defaultText = lib.literalMD "`packages.tam-server` from this flake";
      description = "The internet-facing HTTP process.";
    };

    workerPackage = lib.mkOption {
      type = lib.types.package;
      default = packages.tam-worker;
      defaultText = lib.literalMD "`packages.tam-worker` from this flake";
      description = "The ledger's maintenance pass.";
    };

    authPackage = lib.mkOption {
      type = lib.types.package;
      default = packages.tam-auth;
      defaultText = lib.literalMD "`packages.tam-auth` from this flake";
      description = "The platform identity service.";
    };

    consolePackage = lib.mkOption {
      type = lib.types.package;
      default = packages.teachouse-console;
      defaultText = lib.literalMD "`packages.teachouse-console` from this flake";
      description = ''
        The built console served from `--ui-dir`. Override this with
        `packages.teachouse-console.override { ... }` to supply the Turnstile
        site key, the social-provider list and the Paddle client values, each of
        which is substituted into the bundle at build time and is public.
      '';
    };

    migrationsPackage = lib.mkOption {
      type = lib.types.package;
      default = packages.teachouse-migrations;
      defaultText = lib.literalMD "`packages.teachouse-migrations` from this flake";
      description = "Both migration sets under one path.";
    };

    landingPackage = lib.mkOption {
      type = lib.types.nullOr lib.types.package;
      default = null;
      example = lib.literalMD "`packages.teachouse-landing` from this flake";
      description = ''
        The built public landing page, served from `--landing-dir`. Given, it
        takes the origin's root and every path it holds a file for — `/`, its
        page directories and its hashed assets — and the console keeps
        everything else, including `/app`, which is where the console's home
        moved to. Null, the default, is the arrangement that predates the
        landing page: the console answers the root like any other path.

        `packages.teachouse-landing` is what this is for; `nix/landing.nix`
        builds it and `checks.landing` proves it builds. Nullable rather than
        defaulted to it because the default decides what every deployment's
        root answers with, and that is a decision to take per host rather than
        by upgrading. The type is a package, so this takes a store path and not
        a mutable directory: the server reads the whole build once at start-up.
      '';
    };

    downloads = {
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

    database = {
      name = lib.mkOption {
        type = lib.types.str;
        default = "tam";
        description = "The database every role connects to.";
      };
      socketDir = lib.mkOption {
        type = lib.types.str;
        default = "/run/postgresql";
        description = "Directory holding the postgres unix socket.";
      };
      provision = lib.mkOption {
        type = lib.types.enum [
          "cluster"
          "roles"
          "none"
        ];
        default = "cluster";
        description = ''
          How much of postgres this deployment owns.

          `cluster` configures the server and provisions tenancy on it: the
          database, the five roles, the peer-authentication map, and the auth
          schema and its grants. Correct on a machine that exists to run this.

          `roles` provisions the same tenancy into a cluster somebody else
          configured, and defines nothing about the server itself. The split is
          not arbitrary: `enable`, `package` and `enableTCPIP` are single-valued
          options a second definition collides with, while the database list, the
          role list, the host-based authentication and the identity map are all
          merge-by-concatenation, so this mode composes with another module by
          construction rather than by luck.

          `none` leaves postgres entirely alone, which is the only correct answer
          when the database is not on this host.
        '';
      };
    };

    server = {
      bindAddress = lib.mkOption {
        type = lib.types.str;
        default = "127.0.0.1";
        description = "Loopback by default: the edge is Caddy, and this listener is never reachable off the machine.";
      };
      port = lib.mkOption {
        type = lib.types.port;
        default = 8080;
        description = "TCP port the API and the console are served on behind the edge.";
      };
      backoffice = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Serve the operator `/admin` surface, which refuses every route without its own SELECT-only role.";
      };
      entitlementKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "/run/safix/teachouse-api/teachouse-entitlement-key";
        description = ''
          Path to the Ed25519 PKCS#8 DER key entitlement tokens are signed under.
          Null mints no token and every desktop client's gate stays closed.
        '';
      };
      entitlementPublicKey = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = ''
          The public half this deployment is expected to sign under, as sixty-four
          lowercase hex characters. Not a secret — it is the value a desktop
          release already embeds — and given here so that a restored backup or a
          half-finished rotation refuses the start instead of closing every
          seller's gate while looking healthy.
        '';
      };
      requireEntitlementKey = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Refuse to start without the signing key. This workspace ships debug assertions in release, so production asserts rather than infers.";
      };
      blobKekFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "/run/safix/teachouse-api/teachouse-blob-kek";
        description = "Path to the key every tenant's blob data-encryption key is wrapped under. Null refuses uploads with 503 rather than storing anything unsealed.";
      };
      blobStoreRoot = lib.mkOption {
        type = lib.types.str;
        default = "/var/lib/teachouse/blobs";
        description = "Directory sealed objects are written beneath.";
      };
      paddleWebhookSecret = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = ''
          The billing webhook's whole authentication. The one option here that
          takes a value rather than a path, because `tam-server` offers no
          path-shaped flag for it — so setting it publishes the secret in
          `/proc/<pid>/cmdline` to every local account. Left null the webhook
          answers 503, which is the intended state until the binary grows a
          `--paddle-webhook-secret-path`.
        '';
      };

      mail = {
        resendApiKeyFile = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "/run/safix/teachouse-api/teachouse-resend-api-key";
          description = ''
            Path to a file holding the Resend API key the completion mail is
            sent with, as a bare key on one line, readable by the
            `teachouse-api` account. This is the switch: null, the default,
            configures no mail at all and the outbox drainer logs each
            completion and succeeds, which is what development and CI run.
            Given, the other four mail values are passed with it — the
            binary refuses four fifths of a mail path — so `from`,
            `authInternalSecretFile` and `auth.internalSecretFile` are
            asserted alongside it.

            A path rather than a value, like every other secret this module
            takes, because `/proc/<pid>/cmdline` is readable by every local
            account. Configured, tam-server's address filter is lifted for
            that unit alone: the relay is off this machine and resolves to no
            address the filter could name, so the charter's egress constraint
            then rests on the binary reaching nothing else.
          '';
        };
        from = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = cfg.auth.emailFrom;
          defaultText = lib.literalMD "`auth.emailFrom`";
          example = "Teachouse <no-reply@teachouse.stowiq.io>";
          description = ''
            The sender the completion mail carries, which the Resend account
            must own. Defaults to the identity service's own sender because
            they are one product's mail from one verified domain; a
            deployment that separates them states both.
          '';
        };
        consoleUrl = lib.mkOption {
          type = lib.types.str;
          default = "https://${cfg.domain}";
          defaultText = lib.literalMD "`https://\${domain}`";
          description = ''
            The console's public origin, which the mail's "Open the run"
            button resolves against. Stated as its own value rather than
            derived inside the binary from the issuer, because nothing holds
            the two to one origin and a button that goes nowhere is worse than
            a mail not sent; the default is the origin this module serves.
          '';
        };
        authInternalUrl = lib.mkOption {
          type = lib.types.str;
          default = "http://127.0.0.1:${toString cfg.auth.port}";
          defaultText = lib.literalMD "`http://127.0.0.1:\${auth.port}`";
          description = ''
            Where the identity service's internal address route is reached.
            The loopback listener this module binds, by default, so the
            seller's address never crosses the public edge; the shared secret
            is the fence either way.
          '';
        };
        authInternalSecretFile = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          example = "/run/safix/teachouse-api/teachouse-internal-secret";
          description = ''
            Path to a file holding the shared secret the address route is
            fenced by, as a bare value on one line, readable by the
            `teachouse-api` account. The same value the identity service
            reads through `auth.internalSecretFile`; the two files differ in
            form and in owner, not in content.
          '';
        };
      };
    };

    worker = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = true;
        description = "Run the maintenance pass: lease stealing, park revival and the fleet breaker.";
      };
      name = lib.mkOption {
        type = lib.types.str;
        default = "teachouse-1";
        description = "The worker name recorded on the leases this process takes.";
      };
      pollMs = lib.mkOption {
        type = lib.types.nullOr lib.types.ints.positive;
        default = null;
        description = "Poll interval in milliseconds. Null takes the binary's own default.";
      };
    };

    auth = {
      port = lib.mkOption {
        type = lib.types.port;
        default = 8081;
        description = "Loopback port the identity service serves `/api/auth/*` on.";
      };
      environmentFiles = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ ];
        example = [ "/run/safix/teachouse-auth/teachouse-auth-secret-env" ];
        description = ''
          Files in `KEY=value` form, read as the unit's environment. One file per
          secret rather than one bundled file, so each keeps its own mode, its own
          rotation and its own restart trigger. `BETTER_AUTH_SECRET` is required;
          `RESEND_API_KEY` and `TURNSTILE_SECRET_KEY` are required whenever
          `TAM_AUTH_ENV` is production, which it always is here.
        '';
      };
      emailFrom = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "Teachouse <no-reply@teachouse.stowiq.io>";
        description = "The From address verification and reset mail is sent from. Required by the service whenever a Resend key is set.";
      };
      internalSecretFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        example = "/run/safix/teachouse-auth/teachouse-auth-internal-env";
        description = ''
          A file in `KEY=value` form carrying `TAM_AUTH_INTERNAL_SECRET`, the
          shared secret the identity service's internal address route is
          fenced by, read as the unit's environment beside `environmentFiles`
          and readable by the `teachouse-auth` account. Null, the default,
          configures no secret and the route does not exist: the path answers
          the 404 every unknown path does. Given, it must hold the value
          `server.mail.authInternalSecretFile` holds for tam-server, and the
          host's secrets tooling is what keeps the two files saying the same
          thing.
        '';
      };
      passkeyRpName = lib.mkOption {
        type = lib.types.str;
        default = "Teachouse";
        description = "Relying-party name shown in the browser's passkey prompt. The relying-party identifier is the domain and is not separately configurable, because a passkey enrolled against one identifier does not carry to another.";
      };
      trustedOrigins = lib.mkOption {
        type = lib.types.listOf lib.types.str;
        default = [ "https://${cfg.domain}" ];
        defaultText = lib.literalMD "`[ \"https://\${domain}\" ]`";
        description = "Bare origins better-auth accepts a request from. Each must carry no path and no trailing slash, which the service checks at start-up.";
      };
    };

    backup = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Run the charter's two backup mechanisms. False until an offsite
          repository exists, because a backup with only a local copy is an
          archive of the disk that is going to fail.
        '';
      };
      pgbackrest = {
        repoPath = lib.mkOption {
          type = lib.types.str;
          default = "/var/lib/pgbackrest";
          description = "Local pgBackRest repository. Belongs on a filesystem other than the data directory.";
        };
        retentionFull = lib.mkOption {
          type = lib.types.ints.positive;
          default = 4;
          description = "Full backups retained. Stated rather than unbounded, which is a data-protection decision before it is a storage one.";
        };
      };
      restic = {
        repository = lib.mkOption {
          type = lib.types.str;
          example = "rclone:teachouse-offsite:teachouse";
          description = "The offsite restic repository holding the object store and the local pgBackRest repository.";
        };
        passwordFile = lib.mkOption {
          type = lib.types.str;
          description = "Path to the repository password. A copy of it must live off this machine, or the backup is an archive nobody can open.";
        };
        rcloneConfigFile = lib.mkOption {
          type = lib.types.nullOr lib.types.str;
          default = null;
          description = "rclone configuration for a target restic does not speak natively.";
        };
        timerConfig = lib.mkOption {
          type = lib.types.attrsOf lib.types.str;
          default = {
            OnCalendar = "hourly";
            RandomizedDelaySec = "10m";
          };
          description = "When the object-store snapshot runs. Hourly matches the charter's one-hour recovery point for object storage.";
        };
        checkOpts = lib.mkOption {
          type = lib.types.listOf lib.types.str;
          default = [ "--read-data-subset=1/8" ];
          description = ''
            Set explicitly and never left empty. `runCheck` defaults to
            `checkOpts != []`, so a repository configured without this runs no
            integrity check at all and reports success either way; reading an
            eighth per run means every pack file is actually read across eight
            runs.
          '';
        };
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.server.requireEntitlementKey -> cfg.server.entitlementKeyFile != null;
        message = "services.teachouse.server.requireEntitlementKey is set but entitlementKeyFile is null, so tam-server would refuse to start.";
      }
      {
        assertion = cfg.auth.environmentFiles != [ ];
        message = "services.teachouse.auth.environmentFiles is empty, so BETTER_AUTH_SECRET is unset and tam-auth refuses to start.";
      }
      {
        assertion = mailEnabled == (cfg.server.mail.authInternalSecretFile != null);
        message = ''
          services.teachouse.server.mail.resendApiKeyFile and
          services.teachouse.server.mail.authInternalSecretFile are given
          together or not at all: tam-server refuses a partial mail path.
        '';
      }
      {
        assertion = mailEnabled -> cfg.server.mail.from != null;
        message = ''
          services.teachouse.server.mail.resendApiKeyFile is set but no sender
          is: set services.teachouse.server.mail.from, or auth.emailFrom which
          it defaults to.
        '';
      }
      {
        assertion = mailEnabled -> cfg.auth.internalSecretFile != null;
        message = ''
          services.teachouse.server.mail is configured but
          services.teachouse.auth.internalSecretFile is null, so the identity
          service has no address route and every completion mail would be
          dropped for want of an address.
        '';
      }
      {
        assertion = cfg.backup.enable -> provisionsCluster;
        message = ''
          services.teachouse.backup.enable is set while database.provision is
          "${cfg.database.provision}", which means this deployment does not own
          the cluster. Both halves of the backup are cluster-scoped and neither
          can be aimed at one database inside it.

          pgBackRest backs up a cluster, not a database, so enabling it here
          would either capture every other tenant's data or capture nothing that
          could be restored on its own. And it reads the data directory as its
          own user, for which nixpkgs sets `initdbArgs = [ "--allow-group-access" ]`
          — an argument that applies at initdb and nowhere else, so on a cluster
          somebody else already created that definition is inert and pgBackRest
          would find a directory it cannot read.

          Back the cluster up where the cluster is configured. The host's own
          `services.postgresqlBackup.databases` is the place to name this
          deployment's database.
        '';
      }
    ];

    warnings =
      lib.optional (cfg.server.paddleWebhookSecret != null) ''
        services.teachouse.server.paddleWebhookSecret puts the billing webhook's
        secret in /proc/<pid>/cmdline, where every local account can read it.
        tam-server takes every configuration value as argv and offers no
        path-shaped flag for this one.
      ''
      ++
        lib.optional
          (
            provisionsTenancy
            && !provisionsCluster
            && !(lib.elem cfg.database.name config.services.postgresqlBackup.databases)
          )
          ''
            services.teachouse.database.provision is "roles", so this deployment
            provisions its tenancy into a cluster it does not back up, and
            services.postgresqlBackup.databases does not name ${cfg.database.name}.
            Nothing on this host is backing up the listing catalogue, the job ledger
            or the billing state. Adding it to that list gives a nightly dump; the
            object store under ${cfg.server.blobStoreRoot} is separately uncovered,
            because backup.enable is refused in this mode.
          ''
      ++
        lib.optional
          (
            provisionsTenancy
            && !provisionsCluster
            && lib.versions.major config.services.postgresql.package.version != "17"
          )
          ''
            services.teachouse.database.provision is "roles" and this host's
            cluster is PostgreSQL ${config.services.postgresql.package.version}.
            The migration set has been exercised only against 17 — in
            development, and in every deployment this module configures itself —
            so nothing here says whether it applies cleanly on major version
            ${lib.versions.major config.services.postgresql.package.version}.

            A warning rather than a refusal, deliberately: the cluster belongs to
            this host, and a module that only provisions tenancy into it has no
            standing to veto its version. Rehearse the migration set against this
            version before the first deploy, or pin services.postgresql.package
            to 17 on the host.
          '';

    users.groups.${group} = { };
    users.users =
      lib.genAttrs
        (
          [
            apiUser
            workerUser
            authUser
            migrateUser
          ]
          ++ lib.optional cfg.downloads.enable downloadsUser
        )
        (_: {
          isSystemUser = true;
          inherit group;
        });

    # Created here rather than by either unit's StateDirectory, and that is the
    # whole reason it is a sibling of /var/lib/teachouse: the refresh owns it
    # and tam-server only reads it, so no unit may chown it out from under the
    # other. It exists before either starts, which is what lets tam-server's
    # start-up check pass on a host whose first refresh has not run yet.
    systemd.tmpfiles.settings = lib.mkIf cfg.downloads.enable {
      "10-teachouse-downloads".${downloadsDir}.d = {
        user = downloadsUser;
        inherit group;
        mode = "0755";
      };
    };

    services.postgresql = lib.mkMerge [
      # The cluster half: three single-valued options, defined only where this
      # deployment owns the server.
      #
      # The package is deliberately not a plain definition and cannot be
      # `mkDefault`. A plain one would silently outrank a host's own choice and
      # fail at activation against an existing data directory rather than at
      # evaluation; `mkDefault` collides outright, because nixpkgs defines this
      # option at that same priority from `system.stateVersion` rather than
      # leaving it as an option default. 900 is the one place that says what is
      # meant: it outranks nixpkgs' guess, which would give 18 on a host whose
      # state version is 26.11 and which this workspace has never run against,
      # and it yields to any host that names its own version.
      (lib.mkIf provisionsCluster {
        enable = true;
        package = lib.mkOverride 900 pkgs.postgresql_17;
        enableTCPIP = false;
        settings = lib.mkIf cfg.backup.enable {
          # Forces a write-ahead-log segment at least every minute, so the
          # achieved recovery point sits far inside the fifteen minutes
          # committed for it. Cluster-wide, which is one of the two reasons
          # `backup.enable` is refused when the cluster is not ours.
          archive_timeout = "60s";
        };
      })

      # The tenancy half: every one of these merges, so it composes with whatever
      # else configures this cluster.
      (lib.mkIf provisionsTenancy {
        ensureDatabases = [ cfg.database.name ];
        ensureUsers = [
          # Ownership of `tam` is transferred by the provisioning unit rather than
          # claimed here: `ensureDBOwnership` grants the database that shares the
          # role's name, and this role's name is not the database's.
          { name = "tam_app"; }
          {
            # The one deliberate tenancy crossing: the drainer claims every
            # organisation's due messages and one prune pass covers the whole
            # ledger, which forced row-level security correctly hides from the
            # application role.
            name = "tam_engine";
            ensureClauses.bypassrls = true;
          }
          { name = "tam_auth"; }
          { name = "tam_backoffice"; }
          {
            # Retired, and kept only as a name to grant to. Migrations 0010, 0017
            # and 0032 GRANT to it, they are applied and frozen, and postgres
            # errors on a grant naming a role that does not exist — so a database
            # replaying the set from empty, which is exactly what a first
            # deployment does, fails at 0010 without it.
            name = "tam_broker";
            ensureClauses.login = false;
          }
        ];
        # `mkBefore` rather than a plain definition, and the safety here is
        # positional rather than incidental: pg_hba is first-match-wins, and
        # nixpkgs' default `local all all peer` refuses every one of these
        # because the unix account and the role deliberately do not share a name.
        # Added rules already sit above the defaults, so a plain definition would
        # work today by module ordering alone; pinning it means a later
        # contributor cannot reorder this into a broken deployment.
        authentication = lib.mkBefore "local ${cfg.database.name} all peer map=teachouse";
        # The rule above is scoped to this database and matches every user, so
        # being first it shadows the cluster's `local all postgres` rule for this
        # database alone. The map therefore admits the superuser too: the
        # provisioning oneshot runs as postgres because `ALTER DATABASE … OWNER`
        # needs one, and `services.postgresqlBackup` runs `pg_dump` as postgres.
        identMap = ''
          teachouse ${apiUser}     tam_app
          teachouse ${apiUser}     tam_engine
          teachouse ${apiUser}     tam_backoffice
          teachouse ${workerUser}  tam_engine
          teachouse ${authUser}    tam_auth
          teachouse ${migrateUser} tam_app
          teachouse ${migrateUser} tam_auth
          teachouse postgres       postgres
        '';
      })
    ];

    systemd.services.teachouse-provision = lib.mkIf provisionsTenancy {
      description = "Teachouse schema and grant provisioning";
      requires = [ "postgresql-setup.service" ];
      after = [ "postgresql-setup.service" ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = "postgres";
        Group = "postgres";
        ExecStart = lib.getExe provisionScript;
      };
    };

    systemd.services.teachouse-migrate = {
      description = "Teachouse database migrations";
      requires = lib.optional provisionsTenancy "teachouse-provision.service";
      after = [
        "postgresql.service"
      ]
      ++ lib.optional provisionsTenancy "teachouse-provision.service";
      wantedBy = [ "multi-user.target" ];
      # One runner, once, ordered ahead of everything that serves: an application
      # that migrates on boot means a rollback restarts the old binary against
      # the new schema with no gate, and means two processes can race one
      # migration.
      before = [
        "tam-server.service"
        "tam-worker.service"
        "tam-auth.service"
      ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        User = migrateUser;
        Group = group;
        ExecStart = lib.getExe migrateScript;
      }
      // hardening
      // loopbackOnly;
    };

    # The one unit here allowed off the machine, and the only one. It holds no
    # database url, no marketplace credential and no seller session; what it can
    # reach is a vendor CDN and GitHub's release API, and what it can write is
    # one directory tam-server reads.
    systemd.services.teachouse-downloads-refresh = lib.mkIf cfg.downloads.enable {
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
      // hardening;
    };

    systemd.timers.teachouse-downloads-refresh = lib.mkIf cfg.downloads.enable {
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

    systemd.services.tam-server = {
      description = "Teachouse API and console";
      requires = [ "teachouse-migrate.service" ];
      after = [
        "network.target"
        "teachouse-migrate.service"
      ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        ExecStart = lib.escapeShellArgs ([ "${cfg.package}/bin/tam-server" ] ++ serverArgs);
        User = apiUser;
        Group = group;
        Restart = "on-failure";
        RestartSec = "5s";
        StateDirectory = [
          "teachouse"
          "teachouse/blobs"
        ];
        StateDirectoryMode = "0700";
        MemoryDenyWriteExecute = true;
      }
      // hardening
      // lib.optionalAttrs (!mailEnabled) loopbackOnly;
    };

    systemd.services.tam-worker = lib.mkIf cfg.worker.enable {
      description = "Teachouse ledger maintenance pass";
      requires = [ "teachouse-migrate.service" ];
      after = [
        "network.target"
        "teachouse-migrate.service"
      ];
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        ExecStart = lib.escapeShellArgs (
          [
            "${cfg.workerPackage}/bin/tam-worker"
            (dbUrl "tam_engine")
            cfg.worker.name
          ]
          ++ lib.optional (cfg.worker.pollMs != null) (toString cfg.worker.pollMs)
        );
        User = workerUser;
        Group = group;
        Restart = "on-failure";
        RestartSec = "10s";
        MemoryDenyWriteExecute = true;
      }
      // hardening
      // loopbackOnly;
    };

    systemd.services.tam-auth = {
      description = "Teachouse platform identity and browser session";
      requires = [ "teachouse-migrate.service" ];
      after = [
        "network.target"
        "teachouse-migrate.service"
      ];
      wantedBy = [ "multi-user.target" ];
      environment = {
        TAM_AUTH_ENV = "production";
        TAM_AUTH_BIND = "127.0.0.1";
        TAM_AUTH_PORT = toString cfg.auth.port;
        TAM_AUTH_BASE_URL = "https://${cfg.domain}";
        TAM_AUTH_TRUSTED_ORIGINS = lib.concatStringsSep "," cfg.auth.trustedOrigins;
        TAM_AUTH_PASSKEY_RP_NAME = cfg.auth.passkeyRpName;
        TAM_AUTH_DATABASE_URL = dbUrl "tam_auth";
      }
      // lib.optionalAttrs (cfg.auth.emailFrom != null) {
        TAM_AUTH_EMAIL_FROM = cfg.auth.emailFrom;
      };
      serviceConfig = {
        ExecStart = lib.getExe cfg.authPackage;
        EnvironmentFile =
          cfg.auth.environmentFiles
          ++ lib.optional (cfg.auth.internalSecretFile != null) cfg.auth.internalSecretFile;
        User = authUser;
        Group = group;
        Restart = "on-failure";
        RestartSec = "5s";
        # V8 maps executable pages it has just written, so this service cannot
        # take the protection the two Rust ones do. It is also the only one of
        # the three with off-box work — Resend, Turnstile and the OAuth token
        # endpoints — so it takes no address filter either.
        MemoryDenyWriteExecute = false;
      }
      // hardening;
    };

    services.pgbackrest = lib.mkIf cfg.backup.enable {
      enable = true;
      # The stanza is named `default` because the module's own archive_command
      # names that stanza; a different name here would archive into nothing.
      repos.localhost = {
        path = cfg.backup.pgbackrest.repoPath;
        retention-full = toString cfg.backup.pgbackrest.retentionFull;
      };
      stanzas.default = {
        instances.localhost = { };
        jobs = {
          weekly = {
            schedule = "Sun 02:30";
            type = "full";
          };
          daily = {
            schedule = "Mon..Sat 02:30";
            type = "diff";
          };
          hourly = {
            schedule = "hourly";
            type = "incr";
          };
        };
      };
    };

    services.restic.backups.teachouse = lib.mkIf cfg.backup.enable (
      {
        initialize = true;
        inherit (cfg.backup.restic)
          repository
          passwordFile
          timerConfig
          checkOpts
          ;
        paths = [
          cfg.server.blobStoreRoot
          cfg.backup.pgbackrest.repoPath
        ];
        pruneOpts = [
          "--keep-hourly 24"
          "--keep-daily 7"
          "--keep-weekly 5"
          "--keep-monthly 12"
        ];
      }
      // lib.optionalAttrs (cfg.backup.restic.rcloneConfigFile != null) {
        inherit (cfg.backup.restic) rcloneConfigFile;
      }
    );
  };
}
