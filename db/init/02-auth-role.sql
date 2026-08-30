-- The identity role and the schema it owns. better-auth reaches Postgres only
-- as tam_auth, and the boundary between platform identity and everything else
-- is this file's grant list rather than a convention in the code that
-- connects. Idempotent for the reason 01-app-role.sql is: the compose
-- entrypoint runs the directory once at initdb while the ephemeral devshell
-- path re-runs it on every start.
--
-- tam_auth is deliberately neither a superuser nor BYPASSRLS. A superuser
-- bypasses row-level security and BYPASSRLS is exactly the crossing
-- tam_engine and tam_broker were each given for one narrow reason; the
-- identity service has no such reason, because it never reads a tenant row at
-- all.
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'tam_auth') THEN
        CREATE ROLE tam_auth LOGIN PASSWORD 'tam_auth_dev';
    END IF;
END
$$;

-- better-auth's Kysely adapter emits unqualified table names, so search_path
-- alone decides where its tables land. Setting it on the role rather than in
-- the application means the placement holds for every connection it opens,
-- including the CLI that generates the DDL. public is absent from the path
-- deliberately: an unqualified name that escapes the auth schema resolves to
-- nothing rather than to one of ours.
ALTER ROLE tam_auth SET search_path = auth;

\connect tam

-- Ownership carries USAGE and CREATE, and that is the whole privilege set
-- tam_auth holds: it applies its own reviewed DDL from db/auth/ into this
-- schema and reads and writes only what it created there.
CREATE SCHEMA IF NOT EXISTS auth AUTHORIZATION tam_auth;

-- The boundary, stated rather than merely left unstated. tam_auth is granted
-- nothing on public and nothing on any table in it, so it cannot reach
-- app_user, organisation, connection_secret or any tenant table; writing the
-- revocation down means a later grant into public is a visible reversal of
-- this line rather than an omission nobody notices. tam_auth retains the
-- USAGE on schema public that Postgres grants to PUBLIC, which permits name
-- lookup and, without a privilege on any table, nothing else.
REVOKE ALL ON SCHEMA public FROM tam_auth;
