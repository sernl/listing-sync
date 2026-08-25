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
END
$$;

SELECT 'CREATE DATABASE tam OWNER tam_app'
WHERE NOT EXISTS (SELECT FROM pg_database WHERE datname = 'tam')\gexec
