-- A product file may name a marketplace resource instead of a blob.
--
-- D27 puts the seller's file bytes on the seller's own device: a migrated file
-- is fetched from the source marketplace under the seller's own session and
-- uploaded to the target in the same run, and never reaches our servers. So a
-- `product_file` needs a way to say where its bytes are without our holding
-- them, and `hash` alone cannot say it, because `hash` is a digest we computed
-- over bytes we have.
--
-- The two digests are two different claims and that is why they are two
-- columns. `hash` is a digest the server verified over bytes it holds;
-- `observed_hash` is a digest a device asserted over bytes we never held. The
-- payload manifest's `ControlPlane` and `Marketplace` arms map onto exactly
-- those two, and conflating them would let a device's assertion be read as our
-- verification.
--
-- `hash` becomes nullable and KEEPS its foreign key to `blob`. A composite
-- foreign key under MATCH SIMPLE, which is the default, is satisfied whenever
-- any of its columns is NULL, so a sourced file passes the key without a blob
-- row while a blob-backed file is still held to it exactly as before. Nothing
-- frozen changes and no row needs backfilling: every existing row is
-- blob-backed and satisfies the first branch of the CHECK below.
ALTER TABLE product_file ALTER COLUMN hash DROP NOT NULL;

-- `scan_state` goes with it, for the same reason and not as a convenience.
-- It is our verdict, written in our own voice: on a sourced row it would be a
-- statement that we scanned bytes we never held, which is exactly the
-- conflation the two digest columns exist to prevent, one column over. Not
-- even 'pending', which would imply a scan still to come and there is none.
-- A sourced row carries the device's assertion in the `asserted_*` group and
-- no server scan state at all.
ALTER TABLE product_file ALTER COLUMN scan_state DROP NOT NULL;

-- `product_file.kind` is not forked and is device-asserted on a sourced row,
-- where every other row's is a fact the server probed. That is the same
-- conflation the two digest columns exist to prevent, and it is tolerable here
-- for a reason worth writing down rather than trusting: `hash` decides
-- integrity, so letting a device's word stand where the server's is claimed
-- would be a security property quietly weakened, while `kind` decides only
-- presentation and routing, and is barely read at all on a sourced row —
-- `describe_files` takes that row's name and type from the payload columns
-- instead. Tolerable is not invisible: a sourced row's `kind` must not later
-- be read as something we checked.
ALTER TABLE product_file
    ADD COLUMN source_marketplace         text,
    ADD COLUMN source_connection          uuid,
    ADD COLUMN source_resource            text,
    ADD COLUMN source_entry               text,
    ADD COLUMN observed_hash              bytea,
    ADD COLUMN observed_byte_len          bigint,
    -- The device's own scan verdict, recorded as the device's and never
    -- restated as a clean bill of ours. We did not see these bytes, so the
    -- assertion is advisory: it is evidence about what the seller's machine
    -- found, not a warrant we can issue on it.
    ADD COLUMN asserted_scan_state        text,
    ADD COLUMN asserted_scan_signature    text,
    ADD COLUMN asserted_scan_failure_code text,
    ADD COLUMN asserted_scanned_at        timestamptz,
    ADD COLUMN observed_by_device         text,
    ADD COLUMN observed_at                timestamptz,
    ADD COLUMN recorded_at                timestamptz,
    -- What the upload is called and what type it declares.
    --
    -- One rule, the same one `observed_hash` follows, stated once so the two
    -- cannot drift: these describe the bytes handed onward after the unwrap
    -- decision. With `source_entry` present they are the entry's own name and
    -- type; with it absent the bytes handed onward are the bundle itself, so
    -- they are the bundle's name and `application/zip`. Never the wrapper's
    -- name against the entry's bytes, which is how a seller's worksheet gets
    -- uploaded to their own storefront named as a zip.
    ADD COLUMN payload_file_name          text,
    ADD COLUMN payload_content_type       text;

-- The invariant, as a database fact rather than a rule in application code: a
-- file is blob-backed or marketplace-sourced, never neither and never both.
--
-- The server's scan columns are named in both directions alongside `hash`,
-- rather than left unnamed because they are nullable anyway. The point is the
-- same one the column list makes everywhere else here: a sourced row that
-- acquired a server scan verdict later would be a row claiming we examined
-- bytes we never had, and the constraint should refuse it rather than the
-- reviewer catching it.
--
-- `source_entry` is the one member exempt from the second branch, because a
-- resource whose bytes are the file itself has no entry within a container;
-- its presence is what says an unwrap happened.
--
-- `observed_hash` and `observed_byte_len` digest the bytes handed onward after
-- that decision — the sole entry where a bundle reduces to one file, the
-- bundle otherwise — and never the container. The reason is not tidiness: a
-- marketplace that re-zips a bundle with different timestamps changes the
-- container's digest while the entry's is unchanged, so digesting the
-- container would stall a single-file resource on a mismatch that means
-- nothing. Nothing in SQL can check which of the two was digested, which is
-- why it is written here.
ALTER TABLE product_file ADD CONSTRAINT product_file_blob_or_source CHECK (
    (hash IS NOT NULL AND scan_state IS NOT NULL
        AND source_marketplace IS NULL AND source_connection IS NULL
        AND source_resource IS NULL AND source_entry IS NULL
        AND observed_hash IS NULL AND observed_byte_len IS NULL
        AND asserted_scan_state IS NULL AND asserted_scan_signature IS NULL
        AND asserted_scan_failure_code IS NULL AND asserted_scanned_at IS NULL
        AND observed_by_device IS NULL AND observed_at IS NULL
        AND recorded_at IS NULL
        AND payload_file_name IS NULL AND payload_content_type IS NULL)
    OR
    (hash IS NULL AND scan_state IS NULL
        AND scan_signature IS NULL AND scan_failure_code IS NULL
        AND scanned_at IS NULL
        AND source_marketplace IS NOT NULL AND source_connection IS NOT NULL
        AND source_resource IS NOT NULL
        AND observed_hash IS NOT NULL AND observed_byte_len IS NOT NULL
        AND asserted_scan_state IS NOT NULL
        AND observed_by_device IS NOT NULL AND observed_at IS NOT NULL
        AND recorded_at IS NOT NULL
        AND payload_file_name IS NOT NULL AND payload_content_type IS NOT NULL)
);

ALTER TABLE product_file ADD CONSTRAINT product_file_source_marketplace
    CHECK (source_marketplace IS NULL OR source_marketplace IN ('tes', 'tpt', 'etsy'));

-- The asserted scan group is total, in the shape `product_file_scan_total`
-- has held the server's own group in since 0003.
--
-- Without it `asserted_scan_state = 'clean'` with a NULL `asserted_scanned_at`
-- is storable, and `scan_from_db` refuses that combination on the way back
-- out: the row goes in and then cannot be read, which turns a writer's mistake
-- into a corrupt row nobody can load. The absent case is first because a
-- blob-backed file carries none of these.
ALTER TABLE product_file ADD CONSTRAINT product_file_asserted_scan_total CHECK (
    (asserted_scan_state IS NULL AND asserted_scan_signature IS NULL
        AND asserted_scanned_at IS NULL AND asserted_scan_failure_code IS NULL)
 OR (asserted_scan_state = 'pending'  AND asserted_scan_signature IS NULL
        AND asserted_scanned_at IS NULL AND asserted_scan_failure_code IS NULL)
 OR (asserted_scan_state = 'clean'    AND asserted_scan_signature IS NULL
        AND asserted_scanned_at IS NOT NULL AND asserted_scan_failure_code IS NULL)
 OR (asserted_scan_state = 'infected' AND asserted_scan_signature IS NOT NULL
        AND asserted_scan_failure_code IS NULL)
 OR (asserted_scan_state = 'failed'   AND asserted_scan_failure_code IS NOT NULL
        AND asserted_scan_signature IS NULL)
);

ALTER TABLE product_file ADD CONSTRAINT product_file_source_connection_fk
    FOREIGN KEY (org_id, source_connection) REFERENCES connection (org_id, id);

ALTER TABLE product_file ADD CONSTRAINT product_file_observed_by_device_fk
    FOREIGN KEY (org_id, observed_by_device) REFERENCES device (org_id, id);

-- Every observation, kept rather than folded into the file's own row.
--
-- The file's source group records the first observation and is not overwritten
-- by a later one; a second device reporting a different digest for the same
-- resource is a disagreement to surface, not a write to refuse and not a value
-- to replace. The item view derives that disagreement from more than one
-- distinct `observed_hash` here, which is why nothing about this table
-- refuses on write:
-- a device that observed something different is reporting, not misbehaving,
-- and the seller is the one who needs to know.
--
-- `observed_at` is the device's own instant and `recorded_at` is our receipt of
-- it, two facts rather than one, per migration 0045.
CREATE TABLE product_file_observation (
    org_id            uuid        NOT NULL REFERENCES organisation (id),
    file_id           uuid        NOT NULL,
    observed_by_device text       NOT NULL,
    observed_at       timestamptz NOT NULL,
    recorded_at       timestamptz NOT NULL,
    -- Prefixed even though every row here is by construction a device's
    -- observation, so the prefix is locally redundant. The cost of dropping it
    -- is not inside this table: `hash` on `product_file` means a digest the
    -- server verified over bytes it holds, and one word meaning both that and
    -- a device's assertion across two tables is the conflation this whole
    -- design exists to prevent. A reader joining them should not have to know
    -- which table they are in to know what they have been promised.
    observed_hash     bytea       NOT NULL,
    observed_byte_len bigint      NOT NULL,
    scan_state        text        NOT NULL,
    scan_signature    text,
    scan_failure_code text,
    scanned_at        timestamptz,

    PRIMARY KEY (org_id, file_id, observed_by_device, observed_at),
    FOREIGN KEY (org_id, file_id) REFERENCES product_file (org_id, id),
    FOREIGN KEY (org_id, observed_by_device) REFERENCES device (org_id, id),

    -- Total rather than a closed set, for the reason the file's own asserted
    -- group is: a 'clean' with no instant stores and then cannot be read back.
    CONSTRAINT product_file_observation_scan_total CHECK (
        (scan_state = 'pending'  AND scan_signature IS NULL
            AND scanned_at IS NULL AND scan_failure_code IS NULL)
     OR (scan_state = 'clean'    AND scan_signature IS NULL
            AND scanned_at IS NOT NULL AND scan_failure_code IS NULL)
     OR (scan_state = 'infected' AND scan_signature IS NOT NULL
            AND scan_failure_code IS NULL)
     OR (scan_state = 'failed'   AND scan_failure_code IS NOT NULL
            AND scan_signature IS NULL)
    )
);

CREATE INDEX product_file_observation_by_file
    ON product_file_observation (org_id, file_id, observed_at DESC);

ALTER TABLE product_file_observation ENABLE ROW LEVEL SECURITY;
ALTER TABLE product_file_observation FORCE ROW LEVEL SECURITY;
CREATE POLICY product_file_observation_org_isolation ON product_file_observation
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No grant to any other role. The observation log is written by the device
-- import route and read by the item view, both on the API path under `tam_app`
-- and its forced row-level security. `tam_engine` drives writes and has no
-- reason to read what a device saw, and `tam_auth` is confined to the `auth`
-- schema and must hold no privilege on any table in `public`.
