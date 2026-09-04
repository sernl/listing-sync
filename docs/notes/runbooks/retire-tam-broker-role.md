# Retiring the tam_broker role in production

Dropping the database role the session broker ran as, once nothing authenticates as it.

- date: 2026-09-04
- applies to: the production cluster only
- prerequisite: migration `0051_retire_tam_broker_grants.sql` applied, and the deploy that removed `crates/tam-session-broker` live

## Why this is an operator step and not a migration

`DROP ROLE` is cluster-level and a migration is per-database.
A drop inside a migration would run once per database the migration set is applied to, which under `sqlx::test` is one fresh database per test, and every concurrent one would race it.
It also fails outright while the role still holds a privilege in any other database of the cluster; the error's DETAIL names that database and counts the objects, but not which objects they are.

Dev and CI do not do this at all.
Migrations 0010, 0017 and 0032 grant to `tam_broker` by name, they are applied and frozen, and Postgres errors on a `GRANT` naming a role that does not exist — so a cluster that replays the migration set needs the role to exist or it fails at 0010, in every test database, rather than in the tests that care.
There `db/init/01-app-role.sql` creates it inert on a fresh cluster and converges an existing one to no login, no password and no `BYPASSRLS` every time that file runs, which is every start on the ephemeral devshell path and initdb only on the compose path.
So a compose data directory older than this change still holds the role with `LOGIN`, the dev password published in that file's history, and `BYPASSRLS`, until `just db-reset` re-initialises it; migration 0051 does not converge it either, because dev and CI apply migrations as `tam_app`, which may not alter a role and leaves a notice instead.
Production is different in the one way that matters: it applies migrations incrementally and never replays 0010 against a fresh database, so the role can go for real.

## The steps

Run as a superuser, after the last deploy that could still authenticate as the role.

1. Fence it, so that what step 2 counts cannot grow behind the count:

       ALTER ROLE tam_broker NOLOGIN;

   No new connection can be opened as the role after this.
   It is the same statement migration 0051 attempts and skips when the applying role may not alter roles.

2. Confirm nothing is still connected as it, and enumerate the databases that may hold a grant:

       SELECT count(*) FROM pg_stat_activity WHERE usename = 'tam_broker';
       SELECT datname FROM pg_database WHERE datallowconn;

   Any session the count returns opened before step 1: wait for it or terminate it.
   A database with `datallowconn = false` is absent from that list and still blocks the drop if it holds a grant, so flip the flag, clean it in step 3, and flip it back.

3. In each database returned above, disown. This is per-database, and `DROP ROLE` fails while any of it is outstanding:

       DROP OWNED BY tam_broker;

   That revokes every privilege granted to the role in the current database as well as dropping what it owns, and it reaches sequences, functions and schemas other than `public`, which a `REVOKE ... ON ALL TABLES IN SCHEMA public` would miss.

4. Once every database is clear, drop the role once:

       DROP ROLE tam_broker;

## What to expect afterwards

Nothing in the running application changes.
`connection_secret` keeps its rows, which are the only copy of the credentials sealed before the broker was retired; disposing of them is a later founder call and a new migration, not part of this.
The unique index `connection_platform_account_exclusive` is untouched, and so is every other constraint.

The cluster does change, in one way that outlasts the drop.
Migrations 0010, 0017 and 0032 grant to `tam_broker` by name, so once the role is gone the migration set can no longer bootstrap a fresh database here: a staging refresh, a new region, a disaster-recovery rebuild from migrations rather than from a base backup, and anyone pointing `sqlx::test` at this cluster all fail at 0010 with `role "tam_broker" does not exist`.
Do this only on a cluster where no fresh database will be created from the migration set.
If one must be, recreate the inert name first — `CREATE ROLE tam_broker NOLOGIN` — which is enough, because the frozen grants then replay against it and migration 0051 revokes them again.

The irreversibility is narrow rather than dramatic.
The name can be recreated by hand at any time, but anything still authenticating as the role is broken from the moment of the drop until it is, and the privileges it held are not restored by recreating the name, because migration 0051 revoked them and re-running that migration revokes them again.
So steps 1 and 2 are the ones that matter: the fence is what makes the count mean something, and the count is what makes this safe.
The failure mode of doing it early is an outage rather than data loss.
