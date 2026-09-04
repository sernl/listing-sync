-- The application role and database. Idempotent because the compose entrypoint
-- runs this once at initdb while the ephemeral devshell path re-runs it on
-- every start. tam_app is deliberately NOT a superuser: superusers bypass
-- row-level security, so connecting as one would make the tenancy tests
-- meaningless. CREATEDB is for sqlx::test's throwaway per-test databases.
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'tam_app') THEN
        CREATE ROLE tam_app LOGIN PASSWORD 'tam_dev_password' CREATEDB;
    END IF;
    -- The engine role: the worker scans queued items ACROSS tenants, which
    -- forced row-level security correctly hides from tam_app, so the engine
    -- is a second role with BYPASSRLS — one deliberate, narrow crossing, and
    -- the only one on any cluster this file has run against since the
    -- broker's retirement. The API path never uses it.
    -- Table privileges are granted per table in migration 0007; BYPASSRLS is
    -- cluster-level and must be created by the superuser here.
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'tam_engine') THEN
        CREATE ROLE tam_engine LOGIN PASSWORD 'tam_engine_dev' BYPASSRLS;
    END IF;
    -- The broker role, retired and kept only as a name to grant to. It read
    -- connection_secret for tam-session-broker, which D1 deleted along with
    -- every server-side seller session, and migration 0051 revokes everything
    -- it holds.
    --
    -- It is still created because migrations 0010, 0017 and 0032 grant to it
    -- by name, those are applied and frozen, and Postgres errors on a GRANT
    -- naming a role that does not exist. Without this block a fresh cluster
    -- could not replay the migration set: every sqlx::test database would fail
    -- at 0010, not at the tests that care about the broker.
    --
    -- So it is inert rather than absent, and the ALTER is what makes that true
    -- of a cluster that already has the role rather than only of a fresh one.
    -- The guard short-circuits for an existing tam_broker, and on a cluster
    -- built before its retirement that role still holds LOGIN, the password
    -- published in this file's history and BYPASSRLS; setting the attributes
    -- unconditionally converges it. Wherever this file runs, the role
    -- afterwards cannot authenticate and is not a tenancy crossing even in
    -- principle — which is every start on the ephemeral devshell path, and
    -- initdb only on the compose path, so a data directory that outlives this
    -- change converges when it is re-initialised, when migration 0051 runs as
    -- a role that may alter roles, or by the runbook's first step. Production
    -- drops it outright instead, by the operator step in
    -- docs/notes/runbooks/retire-tam-broker-role.md, which is safe there
    -- because production applies migrations incrementally and never replays
    -- 0010 against a fresh database.
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'tam_broker') THEN
        CREATE ROLE tam_broker NOLOGIN;
    END IF;
    ALTER ROLE tam_broker NOLOGIN NOBYPASSRLS PASSWORD NULL;
END
$$;

SELECT 'CREATE DATABASE tam OWNER tam_app'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'tam')\gexec
