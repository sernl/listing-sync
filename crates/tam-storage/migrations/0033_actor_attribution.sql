-- Who did it, on every audit-bearing row. Mandatory from the first
-- deployment rather than deferred to the first multi-seat tenant: an audit
-- trail that gains attribution later cannot attribute anything written
-- before it, and the rows written before it are exactly the ones a first
-- incident asks about.
--
-- The actor is a closed sum encoded as two columns rather than a nullable
-- user reference. A nullable user column cannot distinguish "the engine did
-- this" from "we failed to record who did this", and those are the two
-- readings a single incident turns on. `actor_kind` names the half of the
-- sum, `actor_id` carries its payload: a user's uuid for 'person', the
-- component's own name for 'system'.
--
-- connection_audit is absent from the loop because migration 0032 created it
-- carrying these columns already; the same two constraints are spelled there.

DO $$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['field_audit', 'job', 'job_event', 'write_attempt'] LOOP
        -- The backfill rides the column defaults rather than an UPDATE:
        -- migration 0006 revoked UPDATE on field_audit from tam_app, and
        -- that revocation binds the owner too because privileges are
        -- checked against the ACL rather than ownership, so a migration
        -- running as tam_app cannot write that table's existing rows at all.
        -- ADD COLUMN ... DEFAULT fills them without DML, which is also the
        -- form that does not rewrite the table.
        --
        -- Every row already here predates attribution. It is recorded as
        -- such, under a name no live writer uses, rather than guessed at:
        -- attributing them to the founder or to the engine would be this
        -- migration inventing evidence, and 'pre-attribution' is the one
        -- honest answer available to it.
        --
        -- Both defaults are then dropped, so a later insert that names no
        -- actor fails on the NOT NULL instead of quietly inheriting one.
        EXECUTE format(
            'ALTER TABLE %I ADD COLUMN actor_kind text NOT NULL DEFAULT ''system''', t
        );
        EXECUTE format(
            'ALTER TABLE %I ADD COLUMN actor_id text DEFAULT ''pre-attribution''', t
        );
        EXECUTE format('ALTER TABLE %I ALTER COLUMN actor_kind DROP DEFAULT', t);
        EXECUTE format('ALTER TABLE %I ALTER COLUMN actor_id DROP DEFAULT', t);

        EXECUTE format(
            'ALTER TABLE %I ADD CONSTRAINT %I CHECK (actor_kind IN (''person'', ''system''))',
            t, t || '_actor_kind'
        );

        -- actor_id stays nullable so the constraint carries the invariant
        -- rather than the column's nullability approximating it: a person is
        -- always identified, and that is a statement about the sum, not
        -- about whether a string is present.
        EXECUTE format(
            'ALTER TABLE %I ADD CONSTRAINT %I '
            'CHECK (actor_kind <> ''person'' OR actor_id IS NOT NULL)',
            t, t || '_person_identified'
        );
    END LOOP;
END
$$;
