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
    -- is a second role with BYPASSRLS — one deliberate, narrow crossing,
    -- mirroring the broker's own coming role (M1e). The API path never uses
    -- it. Table privileges are granted per table in migration 0007; BYPASSRLS
    -- is cluster-level and must be created by the superuser here.
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'tam_engine') THEN
        CREATE ROLE tam_engine LOGIN PASSWORD 'tam_engine_dev' BYPASSRLS;
    END IF;
END
$$;

SELECT 'CREATE DATABASE tam OWNER tam_app'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'tam')\gexec
