-- The credential broker's role loses every privilege it was granted.
--
-- `tam_broker` was the only role that could read `connection_secret`. D1
-- leaves no server-side seller session for a marketplace with no official
-- API, so there is no vault to unseal and no process to unseal it: the
-- session broker is deleted and its role holds nothing after this.
--
-- This migration revokes and does not drop. `DROP ROLE` is cluster-level
-- while a migration is per-database, so a drop here would run once per
-- `sqlx::test` database and race every concurrent one, and it fails outright
-- while the role still holds a privilege in any other database of the
-- cluster. The drop is a production operator act instead, in
-- `docs/notes/runbooks/retire-tam-broker-role.md`.
--
-- The role therefore survives in dev and CI as an inert name. Migrations
-- 0010, 0017 and 0032 grant to it by name, they are applied and frozen, and
-- Postgres errors on a GRANT naming a role that does not exist — so a cluster
-- without `tam_broker` cannot replay this migration set at all, and every
-- `sqlx::test` database would fail at 0010 rather than at the tests that care.
-- `db/init/01-app-role.sql` keeps the name for exactly that reason, creating
-- it inert on a fresh cluster and converging an existing one on every run.
--
-- Table privileges are per-database, so the revokes below reach them on every
-- database this migration is applied to. Role attributes are cluster-level and
-- altering them needs a privilege the applying role may not hold: in dev and
-- CI migrations run as `tam_app` (`justfile` db_url), which is neither
-- superuser nor CREATEROLE. So the attributes are converged here only when the
-- executing role may, and a notice is left otherwise. The dev and CI path is
-- covered by `db/init/01-app-role.sql`, which both provisioning paths run as
-- the superuser, and production by the runbook's first step.
--
-- The existence guard is what makes this correct on a cluster that never had
-- the role, which is production after the drop and any future CI image.
--
-- `connection_secret` is deliberately untouched. Its rows are the only copy of
-- the credentials sealed before this, dropping them is a later founder call,
-- and this migration is frozen the moment it is applied: a later decision to
-- dispose of them is a new migration rather than an edit to this one.
DO $$
DECLARE
    still_held text;
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'tam_broker') THEN
        -- 0010_broker_custody.sql
        REVOKE SELECT, INSERT, UPDATE ON connection_secret FROM tam_broker;
        REVOKE SELECT, UPDATE ON connection FROM tam_broker;
        REVOKE SELECT ON organisation, marketplace_inventory FROM tam_broker;
        -- 0017_broker_links_fresh_connections.sql
        REVOKE INSERT ON connection FROM tam_broker;
        -- 0032_connection_audit.sql
        REVOKE SELECT, INSERT ON connection_audit FROM tam_broker;

        -- REVOKE fails soft: Postgres warns rather than errors when the
        -- executing role is neither the object owner nor the grantor, and a
        -- warning inside a DO block does not fail a migration. Asserting the
        -- outcome is what makes the claim severe instead of assumed, and the
        -- executing identity is not pinned by anything in this tree outside
        -- dev and CI. has_table_privilege is true when any listed privilege is
        -- held, so one call per relation covers the whole verb set granted to
        -- it.
        SELECT string_agg(relation, ', ' ORDER BY relation)
        INTO still_held
        FROM (VALUES
            ('connection_secret', 'SELECT, INSERT, UPDATE'),
            ('connection', 'SELECT, INSERT, UPDATE'),
            ('organisation', 'SELECT'),
            ('marketplace_inventory', 'SELECT'),
            ('connection_audit', 'SELECT, INSERT')
        ) AS granted(relation, privileges)
        WHERE has_table_privilege('tam_broker', relation, privileges);

        IF still_held IS NOT NULL THEN
            RAISE EXCEPTION
                '0051: tam_broker still holds privileges on: %', still_held;
        END IF;

        IF EXISTS (
            SELECT FROM pg_roles
            WHERE rolname = current_user AND (rolsuper OR rolcreaterole)
        ) THEN
            BEGIN
                ALTER ROLE tam_broker NOLOGIN NOBYPASSRLS PASSWORD NULL;
            EXCEPTION WHEN insufficient_privilege THEN
                -- NOBYPASSRLS is superuser-only, so a CREATEROLE role that is
                -- not a superuser satisfies the guard above without satisfying
                -- the statement. Catching that leaves it the same operator
                -- notice the else branch leaves, rather than failing a
                -- migration on privilege grounds.
                RAISE NOTICE 'tam_broker attributes not set: % lacks superuser',
                    current_user;
            END;
        ELSE
            -- db/init/01-app-role.sql converges dev and CI as postgres; in
            -- production the runbook's first step does, or a migration applied
            -- by a role that may alter roles.
            RAISE NOTICE 'tam_broker attributes not set: % may not alter roles',
                current_user;
        END IF;
    END IF;
END
$$;
