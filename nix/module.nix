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
  group = "teachouse";

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
  # no HTTP client at all, tam-server's only outbound call is the key-set fetch
  # aimed at loopback above, and tam-worker reaches no marketplace by decision
  # D1. So the charter's egress constraint is achievable on both as an actual
  # deny rather than an aspiration.
  loopbackOnly = {
    IPAddressDeny = "any";
    IPAddressAllow = "localhost";
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
        [
          apiUser
          workerUser
          authUser
          migrateUser
        ]
        (_: {
          isSystemUser = true;
          inherit group;
        });

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
      // loopbackOnly;
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
        EnvironmentFile = cfg.auth.environmentFiles;
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
