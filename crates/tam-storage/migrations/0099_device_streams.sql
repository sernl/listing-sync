-- Imported files are served from the seller's own devices, and Teachouse's
-- copies of them are destroyed.
--
-- 0.15.0 had the seller's app copy every imported file into the blob store so
-- the console could show it anywhere. That put marketplace-derived files at
-- rest on our servers, which the product's founding rule forbids: they belong
-- on the devices of the seller who runs Teachouse
-- (`docs/notes/design/research/2026-09-29-file-custody.md`). From 0.16.0 the
-- server asks an online device for the bytes and passes them through to the
-- browser without keeping them. Two things make that work, and one thing
-- undoes 0.15.0.
--
-- 1. `device.stream_polled_at`: when the device last asked for streams to
--    answer. It is not `last_seen_at`, which is the heartbeat's and measures
--    whether a revocation reached the device; a stream poll delivers no
--    revocation, so it must not stamp that.
--
-- 2. `device_stream`: one ask, waiting for its device. Metadata only -- which
--    file, which bytes, the capability the device answers under, and which
--    server process is holding the browser's request open -- and short-lived:
--    a row is deleted when its answer ends and is useless past `expires_at`.
--    No byte of any file is ever written here.
--
-- 3. The copies. A server copy is a `blob` row under the digest a sourced
--    (imported) `product_file` names, that nothing else references. Deleting
--    the row destroys its wrapped data key, which is the only key that opens
--    the sealed object: the object left in the store is ciphertext nobody can
--    read (0092's crypto-shred). Files the seller uploaded themselves are
--    referenced by `product_file.hash` and are untouched, as are covers,
--    avatars and anything an import batch or run still names.

ALTER TABLE device ADD COLUMN stream_polled_at timestamptz;

CREATE TABLE device_stream (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    id          uuid        NOT NULL,
    device_id   text        NOT NULL,
    hash        bytea       NOT NULL,
    first_byte  bigint      NOT NULL,
    last_byte   bigint      NOT NULL,
    capability  text        NOT NULL,
    -- The address of the server process holding the browser's request, or
    -- NULL in a single-process deployment.
    pod         text,
    created_at  timestamptz NOT NULL,
    expires_at  timestamptz NOT NULL,
    claimed_at  timestamptz,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, device_id) REFERENCES device (org_id, id) ON DELETE CASCADE,

    CONSTRAINT device_stream_hash_len CHECK (octet_length(hash) = 32),
    CONSTRAINT device_stream_range CHECK (first_byte >= 0 AND last_byte >= first_byte),
    CONSTRAINT device_stream_capability_len CHECK (char_length(capability) <= 4096),
    CONSTRAINT device_stream_pod_len CHECK (pod IS NULL OR char_length(pod) <= 128)
);

CREATE INDEX device_stream_waiting ON device_stream (org_id, device_id)
    WHERE claimed_at IS NULL;

ALTER TABLE device_stream ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_stream FORCE ROW LEVEL SECURITY;
CREATE POLICY device_stream_org_isolation ON device_stream
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No grant to any other role: written and read on the API path alone.

-- The copies, across every tenant. Migrations run as `tam_app`, which owns
-- these tables under FORCE ROW LEVEL SECURITY with no tenant pinned, so the
-- fence is lifted for the length of this transaction and restored below --
-- `0076_import_selection_backfill.sql`'s pattern, for its reason.
CREATE TEMP TABLE server_copy_unfenced (name text PRIMARY KEY) ON COMMIT DROP;
INSERT INTO server_copy_unfenced (name) VALUES
    ('blob'), ('product_file'), ('import_batch_row'), ('import_run_item');

DO $$
DECLARE
    unfenced record;
BEGIN
    FOR unfenced IN SELECT name FROM server_copy_unfenced LOOP
        EXECUTE format('ALTER TABLE %I NO FORCE ROW LEVEL SECURITY', unfenced.name);
    END LOOP;
END $$;

DELETE FROM blob b
 WHERE EXISTS (
           SELECT 1 FROM product_file f
            WHERE f.org_id = b.org_id AND f.hash IS NULL AND f.observed_hash = b.hash
       )
   AND NOT EXISTS (SELECT 1 FROM product_file f WHERE f.org_id = b.org_id AND f.hash = b.hash)
   AND NOT EXISTS (
           SELECT 1 FROM import_batch_row r
            WHERE r.org_id = b.org_id AND (r.file_hash = b.hash OR r.cover_hash = b.hash)
       )
   AND NOT EXISTS (
           SELECT 1 FROM import_run_item i WHERE i.org_id = b.org_id AND i.cover_hash = b.hash
       )
   AND NOT EXISTS (
           SELECT 1 FROM app_user u WHERE u.org_id = b.org_id AND u.avatar_hash = b.hash
       );

DO $$
DECLARE
    unfenced record;
BEGIN
    FOR unfenced IN SELECT name FROM server_copy_unfenced LOOP
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', unfenced.name);
    END LOOP;
END $$;
