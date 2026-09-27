-- An operator deletes a seller, and the seller's organisation goes with them.
--
-- The route that calls this (DELETE /v1/admin/users/{auth_subject}) refuses
-- before it gets here when the organisation has another member, when the
-- seller is an active operator, or when a subscription is still live at the
-- billing provider. What reaches `erase_organisation` is therefore always a
-- one-person tenant nobody is paying for, and erasing it means every row it
-- owns, in every table that carries an org_id.
--
-- The table list is read from the catalogue rather than written out here. A
-- hand-kept list of seventy-odd tables would be wrong the first time a later
-- migration added a tenant table and forgot this file; walking pg_constraint
-- instead deletes children before parents for whatever tables exist when it
-- runs, and a table it cannot order (a foreign-key cycle) is an error that
-- names the table rather than a half-erased tenant.
--
-- Four tables were append-only by revoke: field_audit, connection_audit and
-- mapping_loss had UPDATE and DELETE revoked from tam_app, and
-- connection_secret had everything revoked. Erasure is the one legitimate
-- reason for a row to leave them, so tam_app gets DELETE back on all four --
-- no SELECT on the vault, so it still cannot read a sealed credential -- and
-- a statement-level trigger refuses any DELETE outside an erasure of the
-- pinned organisation. That trigger is a guard against an ordinary code path
-- deleting history by accident, not against tam_app itself: the role that
-- owns the tables can always set the flag it checks. The honest statement of
-- the property is "only this function deletes from them", and the function
-- is short enough to read.
--
-- UPDATE comes back on the three audit tables too, because deleting an
-- organisation makes Postgres lock the referencing rows FOR KEY SHARE as the
-- tables' owner, and that lock needs UPDATE. So append-only against UPDATE
-- moves from the privilege to a trigger that refuses every UPDATE,
-- unconditionally: nothing has ever legitimately rewritten an audit row.
--
-- Stored file bytes: a sealed blob row holds its own wrapped data key, so
-- deleting the row destroys the only key that opens the object. The object
-- itself is left to the store's own lifecycle. A blob from before 0011's
-- per-tenant keys (dek_key_version 0, no wrapped key) is plain in the store
-- and stays readable there until that lifecycle removes it.

GRANT DELETE ON field_audit, connection_audit, mapping_loss, connection_secret TO tam_app;
GRANT UPDATE ON field_audit, connection_audit, mapping_loss TO tam_app;

CREATE FUNCTION refuse_update_append_only() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '% is append-only; a row is never rewritten', TG_TABLE_NAME
        USING ERRCODE = 'insufficient_privilege';
END
$$;

CREATE TRIGGER field_audit_never_updated
    BEFORE UPDATE ON field_audit
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_update_append_only();

CREATE TRIGGER connection_audit_never_updated
    BEFORE UPDATE ON connection_audit
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_update_append_only();

CREATE TRIGGER mapping_loss_never_updated
    BEFORE UPDATE ON mapping_loss
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_update_append_only();

CREATE FUNCTION refuse_delete_outside_erasure() RETURNS trigger
    LANGUAGE plpgsql AS $$
DECLARE
    erasing uuid := nullif(current_setting('app.erasing_org', true), '')::uuid;
    pinned  uuid := nullif(current_setting('app.current_org', true), '')::uuid;
BEGIN
    IF erasing IS NULL OR pinned IS DISTINCT FROM erasing THEN
        RAISE EXCEPTION '% is append-only; its rows leave only when their organisation is erased',
            TG_TABLE_NAME
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NULL;
END
$$;

CREATE TRIGGER field_audit_delete_only_on_erasure
    BEFORE DELETE ON field_audit
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_delete_outside_erasure();

CREATE TRIGGER connection_audit_delete_only_on_erasure
    BEFORE DELETE ON connection_audit
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_delete_outside_erasure();

CREATE TRIGGER mapping_loss_delete_only_on_erasure
    BEFORE DELETE ON mapping_loss
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_delete_outside_erasure();

-- 0010 revoked every privilege on the vault, TRIGGER included, so attaching
-- the guard needs it for the length of this statement and no longer.
GRANT TRIGGER ON connection_secret TO tam_app;
CREATE TRIGGER connection_secret_delete_only_on_erasure
    BEFORE DELETE ON connection_secret
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_delete_outside_erasure();
REVOKE TRIGGER ON connection_secret FROM tam_app;

-- The vault's two foreign keys go. Deleting an organisation (or a connection)
-- makes Postgres check the vault for referencing rows as the table's owner,
-- and the owner, tam_app, holds no SELECT there by design -- so while the keys
-- stood, no organisation that had ever been created could be deleted by
-- anyone short of a superuser. The keys also guard nothing any more: 0051
-- retired the vault's last writer, so no row can be inserted to dangle. The
-- erasure below deletes the organisation's vault rows under the pin before
-- the organisation itself, so none is left behind either.
ALTER TABLE connection_secret DROP CONSTRAINT connection_secret_org_id_fkey;
ALTER TABLE connection_secret DROP CONSTRAINT connection_secret_org_id_connection_id_fkey;

-- Erase one organisation and everything it owns.
--
-- Runs as the caller (tam_app) inside the caller's transaction, which must
-- already have pinned app.current_org to the same organisation: forced
-- row-level security is still the fence, and the explicit org_id predicate on
-- every statement is a second one. connection_secret is the exception to the
-- predicate, because tam_app holds no SELECT there and so cannot name its
-- columns; it is deleted under the pin alone, and only because it carries
-- forced row-level security -- a table that did not would be refused.
--
-- Two references to a departing user live outside the tenant: a guide's
-- updated_by, which is nulled (the guide is the platform's, and "last edited
-- by someone who has left" is what a null there means), and a revoked
-- operator marking, which goes with the user. An active marking is refused by
-- the route before this runs.
--
-- Answers how many rows left, for the log line the route writes.
CREATE FUNCTION erase_organisation(target uuid) RETURNS bigint
    LANGUAGE plpgsql AS $$
DECLARE
    pinned    uuid := nullif(current_setting('app.current_org', true), '')::uuid;
    remaining regclass[];
    ready     regclass[];
    rel       regclass;
    erased    bigint := 0;
    affected  bigint;
BEGIN
    IF target IS NULL OR pinned IS DISTINCT FROM target THEN
        RAISE EXCEPTION 'erase_organisation(%) needs app.current_org pinned to the same organisation', target
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    PERFORM set_config('app.erasing_org', target::text, true);

    UPDATE guide SET updated_by = NULL
        WHERE updated_by IN (SELECT id FROM app_user WHERE org_id = target);
    DELETE FROM platform_operator
        WHERE user_id IN (SELECT id FROM app_user WHERE org_id = target);

    SELECT array_agg(c.oid::regclass) INTO remaining
        FROM pg_class c
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attname = 'org_id' AND NOT a.attisdropped
        WHERE c.relkind IN ('r', 'p')
          AND c.relnamespace = 'public'::regnamespace
          AND c.oid <> 'organisation'::regclass;

    WHILE cardinality(remaining) > 0 LOOP
        -- A table is ready once no other remaining table references it.
        SELECT array_agg(r) INTO ready
            FROM unnest(remaining) AS r
            WHERE NOT EXISTS (
                SELECT 1 FROM pg_constraint k
                WHERE k.contype = 'f'
                  AND k.confrelid = r
                  AND k.conrelid <> r
                  AND k.conrelid = ANY (remaining)
            );
        IF ready IS NULL THEN
            RAISE EXCEPTION 'erase_organisation: cannot order the deletion of %', remaining;
        END IF;

        FOREACH rel IN ARRAY ready LOOP
            IF has_column_privilege(rel, 'org_id', 'SELECT') THEN
                EXECUTE format('DELETE FROM %s WHERE org_id = $1', rel) USING target;
            ELSIF (SELECT relforcerowsecurity FROM pg_class WHERE oid = rel) THEN
                EXECUTE format('DELETE FROM %s', rel);
            ELSE
                RAISE EXCEPTION 'erase_organisation: % is neither readable nor fenced by row security', rel;
            END IF;
            GET DIAGNOSTICS affected = ROW_COUNT;
            erased := erased + affected;
        END LOOP;

        SELECT coalesce(array_agg(r), '{}') INTO remaining
            FROM unnest(remaining) AS r
            WHERE r <> ALL (ready);
    END LOOP;

    DELETE FROM organisation WHERE id = target;
    GET DIAGNOSTICS affected = ROW_COUNT;
    erased := erased + affected;

    PERFORM set_config('app.erasing_org', '', true);
    RETURN erased;
END
$$;
