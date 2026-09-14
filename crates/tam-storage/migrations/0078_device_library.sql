-- What each of the seller's machines holds in its own library, and where
-- another of their machines can reach it directly.
--
-- Coordination only. The seller's files stay on the seller's own devices and
-- move between them over a direct, encrypted connection the devices make to
-- each other; the server carries a device's node address and the digests it
-- holds, never bytes. There is deliberately no column here a file could
-- travel in, for the reason migration 0042 gives about credentials: a
-- server-side column able to hold one would reintroduce the custody the
-- architecture exists to avoid.
--
-- `direct_addrs` is the list of socket addresses the device's endpoint is
-- reachable at, as the device reported them; there is no relay and none will
-- be added here. A transfer between two devices that cannot reach each other
-- directly waits, and the console says so.

CREATE TABLE device_node_addr (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    device_id    text        NOT NULL,
    node_id      text        NOT NULL,
    direct_addrs jsonb       NOT NULL,
    reported_at  timestamptz NOT NULL,

    PRIMARY KEY (org_id, device_id),
    FOREIGN KEY (org_id, device_id) REFERENCES device (org_id, id) ON DELETE CASCADE,

    CONSTRAINT device_node_addr_node_bounded CHECK (char_length(node_id) BETWEEN 1 AND 128),
    CONSTRAINT device_node_addr_addrs_list   CHECK (jsonb_typeof(direct_addrs) = 'array')
);

-- One row per file a device reports holding, replaced whole at every report
-- as the session rows are: a digest the device stops naming is a file it no
-- longer holds.
CREATE TABLE device_library_holding (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    device_id   text        NOT NULL,
    hash        bytea       NOT NULL,
    byte_len    bigint      NOT NULL,
    reported_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, device_id, hash),
    FOREIGN KEY (org_id, device_id) REFERENCES device (org_id, id) ON DELETE CASCADE,

    CONSTRAINT device_library_holding_hash_len CHECK (octet_length(hash) = 32),
    CONSTRAINT device_library_holding_len_positive CHECK (byte_len >= 0)
);

-- A device the seller asked to fetch one file from whichever of their other
-- devices holds it. Cleared by the device once the file lands, or by the
-- seller cancelling.
CREATE TABLE device_library_want (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    device_id    text        NOT NULL,
    hash         bytea       NOT NULL,
    requested_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, device_id, hash),
    FOREIGN KEY (org_id, device_id) REFERENCES device (org_id, id) ON DELETE CASCADE,

    CONSTRAINT device_library_want_hash_len CHECK (octet_length(hash) = 32)
);

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form.
ALTER TABLE device_node_addr ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_node_addr FORCE ROW LEVEL SECURITY;
CREATE POLICY device_node_addr_org_isolation ON device_node_addr
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE device_library_holding ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_library_holding FORCE ROW LEVEL SECURITY;
CREATE POLICY device_library_holding_org_isolation ON device_library_holding
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE device_library_want ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_library_want FORCE ROW LEVEL SECURITY;
CREATE POLICY device_library_want_org_isolation ON device_library_want
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No grant to any other role: written and read on the API path alone, under
-- `tam_app` and its forced row-level security.
