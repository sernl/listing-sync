-- The operator backoffice's role. The admin surface reads across tenants,
-- which is exactly what forced row-level security hides from tam_app, so the
-- crossing is a second role rather than a relaxation of the first.
-- Idempotent for the reason 01-app-role.sql and 02-auth-role.sql are: the
-- compose entrypoint runs this directory once at initdb while the ephemeral
-- devshell path re-runs every file in it on every start.
--
-- tam_backoffice is deliberately neither a superuser nor BYPASSRLS, which is
-- where it parts company with tam_engine. That role crosses tenants through a
-- cluster-level attribute that no migration can scope: once granted, it
-- applies to every table the role is ever granted a privilege on.
-- This role's crossing is enumerated per table instead, in the grant list and
-- the read policies of migration 0037, so a table absent from either list
-- stays invisible to it even if the other list later names it. It holds no
-- CREATEDB and owns no schema: it creates nothing and only ever reads.
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'tam_backoffice') THEN
        CREATE ROLE tam_backoffice LOGIN PASSWORD 'tam_backoffice_dev';
    END IF;
END
$$;
