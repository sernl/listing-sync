-- The picture a seller sets for themselves.
--
-- On `app_user` rather than `organisation` because the picture is the
-- person's: it is what the console draws in the account cell beside the
-- organisation's name, and an invited second user would otherwise inherit a
-- face that is not theirs.
--
-- A content hash and the same foreign key the import row's cover carries
-- (0062), for the same reason: `blob` is keyed `(org_id, hash)` per tenant,
-- so the column can name only bytes this organisation sealed, and a hash
-- another tenant holds or nobody uploaded is refused by the database rather
-- than only by the route. The bytes arrive through `POST /{version}/uploads`
-- as every picture does, and the route reads them back before it writes here.
--
-- No grant lines, deliberately. `tam_app` owns the table. The engine's read
-- on this table is the column-scoped `SELECT` migration 0063 grants, which a
-- new column does not join, and nothing on an engine path draws a picture.
-- `tam_auth` never reaches `public`.
ALTER TABLE app_user ADD COLUMN avatar_hash bytea;

ALTER TABLE app_user
    ADD CONSTRAINT app_user_avatar_blob_held
    FOREIGN KEY (org_id, avatar_hash) REFERENCES blob (org_id, hash);
