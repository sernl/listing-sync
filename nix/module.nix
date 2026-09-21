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
  hardening = import ./hardening.nix;
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

  # The completion mail is on exactly when a relay key is named; the binary
  # takes its five mail values together or not at all, and the assertions
  # below hold the rest of the set to this one switch.
  mailEnabled = cfg.server.mail.resendApiKeyFile != null;

  # The store both blob-holding binaries are pointed at, rendered once: the
  # server and the ingest worker hold the same objects, so a unit that named
  # one store and a unit that named another would be a worker reading a bucket
  # the server never wrote to.
  blobStoreArgs =
    if cfg.server.blobStore.kind == "s3" then
      [
        "--blob-store-s3"
        cfg.server.blobStore.endpoint
        "--blob-store-bucket"
        cfg.server.blobStore.bucket
        "--blob-store-region"
        cfg.server.blobStore.region
        "--blob-store-credentials"
        cfg.server.blobStore.credentialsFile
      ]
    else
      [
        "--blob-store-root"
        cfg.server.blobStore.root
      ];

  # Where the objects are, in prose, for the warning that names it.
  blobStoreWhere =
    if cfg.server.blobStore.kind == "s3" then
      "bucket ${toString cfg.server.blobStore.bucket} at ${toString cfg.server.blobStore.endpoint}"
    else
      cfg.server.blobStore.root;

  # The endpoint's host as systemd's address filter would have to name it:
  # scheme and path removed, port left on, because `IPAddressAllow` takes an
  # address or a prefix and neither carries one.
  blobStoreHost =
    let
      afterScheme = lib.last (lib.splitString "//" (toString cfg.server.blobStore.endpoint));
      authority = lib.head (lib.splitString "/" afterScheme);
    in
    lib.head (lib.splitString ":" authority);

  # An address the filter can name, as against a name it cannot: a host that
  # resolves at run time is not expressible as a prefix, and a filter written
  # around one would deny the store instead of allowing it.
  blobStoreHostIsAddress = builtins.match "[0-9.]+|[0-9a-fA-F:]+" blobStoreHost != null;

  # tam-server's egress. The deny stands unless something the binary must
  # reach is off this machine: the completion mail's relay is one such thing,
  # for the reason `loopbackOnly` states, and an object store in a bucket is
  # the other. Where that endpoint is an address the filter names it rather
  # than being dropped; where it is a hostname there is nothing to name, and
  # the warning below says so.
  serverEgress =
    if mailEnabled then
      { }
    else if cfg.server.blobStore.kind != "s3" then
      loopbackOnly
    else if blobStoreHostIsAddress then
      loopbackOnly
      // {
        IPAddressAllow = [
          "localhost"
          blobStoreHost
        ];
      }
    else
      { };

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
    cfg.downloads.directory
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
  ++ lib.optionals (cfg.server.blobKekFile != null) (
    [
      "--blob-kek-path"
      cfg.server.blobKekFile
    ]
    ++ blobStoreArgs
  )
  ++ lib.optionals (cfg.server.stripeWebhookSecret != null) [
    "--stripe-webhook-secret"
    cfg.server.stripeWebhookSecret
  ]
  ++ lib.optionals (cfg.server.stripeSecretKeyFile != null) [
    "--stripe-secret-key"
    cfg.server.stripeSecretKeyFile
  ]
  ++ lib.optionals (cfg.server.stripePriceMap != { }) [
    "--stripe-price-map"
    (pkgs.writeText "teachouse-stripe-price-map.json" (builtins.toJSON cfg.server.stripePriceMap))
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

  # tam-server and tam-worker reach nothing off this machine while their stores
  # are here: tam-server's outbound calls are the key-set fetch aimed at
  # loopback above and the identity service's address route on the same
  # loopback, and tam-worker reaches no marketplace by decision D1. So the
  # charter's egress constraint is achievable on both as an actual deny rather
  # than an aspiration. Two exceptions, each stated where it is taken: with the
  # completion mail configured, tam-server posts to the relay, which is off
  # this machine and resolves to no address a filter could name, so that unit
  # drops the filter and the constraint rests on the binary reaching nothing
  # else — which is what `--resend-api-key-file`'s own header records. And with
  # `blobStore.kind = "s3"` the blob bytes go to an object store rather than a
  # directory, so `serverEgress` allows that endpoint's address beside
  # loopback, or drops the filter where the endpoint is a name.
  loopbackOnly = {
    IPAddressDeny = "any";
    IPAddressAllow = "localhost";
  };

  migrateScript = pkgs.writeShellApplication {
    name = "teachouse-migrate";
    runtimeInputs = [
      pkgs.sqlx-cli
      config.services.postgresql.package
      cfg.adminPackage
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

      # The normalizations the schema set cannot express, run after it and
      # under the same role the migrations applied as. A legacy migration's
      # event anchor is found by re-deriving its idempotency key, which is a
      # UUIDv5 this database has no extension to compute, so the pass is the
      # Rust helper that minted it rather than SQL that reimplements it.
      #
      # Idempotent and counted: a second boot over an already-normalized
      # database links nothing and says so. It is ordered here rather than
      # left to an operator because the deletion fence reads the column it
      # writes, and a service started before it would answer a seller's
      # Delete without fencing their migration's next page.
      tam-admin "$appUrl" backfill-workflow-owners

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
      example = "teachouse.io";
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
        site key and the social-provider list, each of
        which is substituted into the bundle at build time and is public.
      '';
    };

    migrationsPackage = lib.mkOption {
      type = lib.types.package;
      default = packages.teachouse-migrations;
      defaultText = lib.literalMD "`packages.teachouse-migrations` from this flake";
      description = "Both migration sets under one path.";
    };

    adminPackage = lib.mkOption {
      type = lib.types.package;
      default = packages.tam-admin;
      defaultText = lib.literalMD "`packages.tam-admin` from this flake";
      description = ''
        The operator one-shot. Runs under no unit of its own; the migration
        runner invokes its `backfill-workflow-owners` pass after the schema
        set, and an operator on the box uses the same binary to grant and
        withdraw the platform-operator marking.
      '';
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
        description = ''
          Directory sealed objects are written beneath. The local form's root,
          and what `blobStore.root` defaults to; ignored where
          `blobStore.kind` is "s3".
        '';
      };
      blobStore = lib.mkOption {
        default = { };
        description = ''
          Which store the sealed objects live in. The default is the local
          directory every deployment has had; "s3" points both units at an
          S3-compatible bucket instead, which is a Garage or MinIO endpoint
          here rather than AWS.

          The binary takes the two forms as mutually exclusive sets and refuses
          a partial one, and the assertions below say the same thing before a
          unit is written.
        '';
        type = lib.types.submodule {
          options = {
            kind = lib.mkOption {
              type = lib.types.enum [
                "local"
                "s3"
              ];
              default = "local";
              description = "Whether objects go into a directory on this host or into a bucket.";
            };
            root = lib.mkOption {
              type = lib.types.str;
              default = cfg.server.blobStoreRoot;
              defaultText = lib.literalExpression "config.services.teachouse.server.blobStoreRoot";
              description = "The local form's directory.";
            };
            endpoint = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "http://10.147.21.2:3900";
              description = "The bucket's endpoint, carrying its scheme.";
            };
            bucket = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "teachouse";
              description = "The one bucket every object of this deployment is written into.";
            };
            region = lib.mkOption {
              type = lib.types.str;
              default = "garage";
              description = "The signing region, which is part of a signature's scope on every S3-compatible endpoint.";
            };
            credentialsFile = lib.mkOption {
              type = lib.types.nullOr lib.types.str;
              default = null;
              example = "/run/safix/teachouse-api/teachouse-blob-s3-credentials";
              description = ''
                A file holding two lines, the access key then the secret key,
                read once at start-up. A file rather than an inline value for
                the reason blobKekFile is one: a secret in argv is in every
                process listing on the host.
              '';
            };
          };
        };
      };
      stripeWebhookSecret = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = ''
          The billing webhook's whole authentication, Stripe's endpoint
          signing secret. The one billing option here that takes a value
          rather than a path, because `tam-server` offers no path-shaped flag
          for it — so setting it publishes the secret in `/proc/<pid>/cmdline`
          to every local account. Left null the webhook answers 503, which is
          the intended state until the binary grows a
          `--stripe-webhook-secret-path`.
        '';
      };

      stripeSecretKeyFile = lib.mkOption {
        type = lib.types.nullOr lib.types.str;
        default = null;
        description = ''
          A path to Stripe's secret API key, which is what opens a checkout,
          re-reads a completed session and mints a billing-portal link. A path
          rather than a value, for the reason the blob and entitlement keys
          are paths: a secret on a command line is in every process listing on
          the host. Left null, checkout and the portal answer 503.
        '';
      };

      stripePriceMap = lib.mkOption {
        type = lib.types.attrsOf (
          lib.types.enum [
            "sync_monthly"
            "sync_yearly"
            "founding_yearly"
            "pack_20"
            "pack_50"
            "pack_100"
            "pack_250"
            "pack_500"
            "move_with_me"
          ]
        );
        default = { };
        example = {
          "price_01abc" = "sync_monthly";
          "price_01def" = "pack_100";
        };
        description = ''
          Stripe price identifiers and what each one sells. Rendered to a file
          in the store, which is fine: a price id is public. Empty, checkout
          refuses every key and a completed session grants nothing, and the
          server says so at startup.
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
          example = "Teachouse <no-reply@notify.teachouse.io>";
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
        example = "Teachouse <no-reply@notify.teachouse.io>";
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

  imports = [ ./module-downloads.nix ];

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
      {
        assertion =
          cfg.server.blobStore.kind == "s3"
          -> (
            cfg.server.blobStore.endpoint != null
            && cfg.server.blobStore.bucket != null
            && cfg.server.blobStore.credentialsFile != null
          );
        message = ''
          services.teachouse.server.blobStore.kind is "s3" but one of endpoint,
          bucket or credentialsFile is null. The binaries take those three
          together or not at all, so this would refuse to start.
        '';
      }
      {
        assertion =
          cfg.server.blobStore.kind == "local"
          -> (
            cfg.server.blobStore.endpoint == null
            && cfg.server.blobStore.bucket == null
            && cfg.server.blobStore.credentialsFile == null
          );
        message = ''
          services.teachouse.server.blobStore.kind is "local" while an S3
          endpoint, bucket or credentials file is also set. Two stores are not
          a store: set kind = "s3" to write into the bucket, or clear those
          values.
        '';
      }
    ];

    warnings =
      lib.optional (cfg.server.stripeWebhookSecret != null) ''
        services.teachouse.server.stripeWebhookSecret puts the billing webhook's
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
            object store at ${blobStoreWhere} is separately uncovered,
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
          ''
      ++ lib.optional (cfg.server.blobStore.kind == "s3" && !mailEnabled && !blobStoreHostIsAddress) ''
        services.teachouse.server.blobStore.endpoint names the host
        ${blobStoreHost} rather than an address, so tam-server's
        IPAddressDeny is dropped entirely: systemd's filter takes addresses
        and prefixes, and a name resolved at run time is neither. Give the
        endpoint as an address to keep the deny, and the store's own
        address allowed beside loopback.
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
        # The blob subdirectory only where the objects are local: in the bucket
        # form an empty 0700 directory here would say the store is on this host
        # when it is not.
        StateDirectory = [
          "teachouse"
        ]
        ++ lib.optional (cfg.server.blobStore.kind == "local") "teachouse/blobs";
        StateDirectoryMode = "0700";
        MemoryDenyWriteExecute = true;
      }
      // hardening
      // serverEgress;
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
        # The local directory only. A bucket is the store's own to protect and
        # restic cannot read it from here, so naming it would back up a path
        # that does not exist.
        paths = lib.optional (cfg.server.blobStore.kind == "local") cfg.server.blobStore.root ++ [
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
