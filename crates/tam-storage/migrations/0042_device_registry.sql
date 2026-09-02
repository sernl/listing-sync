-- The device registry: which of the seller's own machines hold marketplace
-- sessions, and what each one holds.
--
-- Decision D14 in `docs/notes/design/vendoo-for-teachers-rethink.md` adds this
-- beside better-auth rather than inside it. better-auth's session row carries
-- `ipAddress` and `userAgent` and has no device-name concept, and its
-- `multiSession` plugin is several accounts in one browser rather than a
-- registry of machines, so the columns a "Your devices" page needs exist
-- nowhere until they exist here. The two planes stay separate: nothing in
-- `public` reads the `auth` schema, and the page joins the two in the browser.
--
-- Metadata only. Neither table has a column a cookie, a token or any other
-- credential could travel in, and that is the point rather than an oversight:
-- D1 puts every no-API marketplace session on the seller's own device, so a
-- server-side column able to hold one would reintroduce the custody the whole
-- architecture exists to avoid. `account_label` is the storefront name the
-- marketplace already shows the seller, not an identifier that authenticates
-- anything.
--
-- The device identifier is text rather than uuid because the device mints it
-- itself: D14 records that `machine-uid` covers neither Android nor iOS, so
-- the client writes a random value into its own application data and the
-- server takes it as given. It is scoped by organisation, so a collision is
-- only ever between one seller's own machines, and it is bounded so a client
-- cannot register an arbitrarily long key.

CREATE TABLE device (
    org_id        uuid        NOT NULL REFERENCES organisation (id),
    id            text        NOT NULL,
    name          text        NOT NULL,
    os            text        NOT NULL,
    arch          text        NOT NULL,
    app_version   text        NOT NULL,
    first_seen_at timestamptz NOT NULL,
    last_seen_at  timestamptz NOT NULL,
    -- Set by the seller's own sign-out on the "Your devices" page. The device
    -- learns of it on its next heartbeat and wipes then, so this instant is
    -- when we decided rather than when the device complied; `last_seen_at`
    -- past this instant is the only evidence that it did.
    revoked_at    timestamptz,

    PRIMARY KEY (org_id, id),

    CONSTRAINT device_id_bounded      CHECK (char_length(id)          BETWEEN 1 AND 64),
    CONSTRAINT device_name_bounded    CHECK (char_length(name)        BETWEEN 1 AND 200),
    CONSTRAINT device_os_bounded      CHECK (char_length(os)          BETWEEN 1 AND 64),
    CONSTRAINT device_arch_bounded    CHECK (char_length(arch)        BETWEEN 1 AND 64),
    CONSTRAINT device_version_bounded CHECK (char_length(app_version) BETWEEN 1 AND 64)
);

-- One row per marketplace a device holds a session for, as the device last
-- reported it. `status` is closed rather than free text so the page's
-- rendering is total, and every value is one the device can actually
-- distinguish without making a marketplace request:
--
--   connected  -- the device holds a session it captured and has not discarded
--   signed_out -- the seller disconnected it on the device itself
--   wiped      -- the device discarded it because a heartbeat said the device
--                 was revoked
--
-- There is deliberately no `expired`. Nothing on the device can tell a live
-- session from a dead one without contacting the marketplace, and a status
-- nothing can produce would be a claim the data does not support.
CREATE TABLE device_marketplace_session (
    org_id        uuid        NOT NULL REFERENCES organisation (id),
    device_id     text        NOT NULL,
    marketplace   text        NOT NULL,
    account_label text,
    linked_at     timestamptz NOT NULL,
    last_used_at  timestamptz NOT NULL,
    status        text        NOT NULL,

    PRIMARY KEY (org_id, device_id, marketplace),

    -- A session belongs to the device that captured it and cannot outlive it.
    FOREIGN KEY (org_id, device_id) REFERENCES device (org_id, id) ON DELETE CASCADE,

    CONSTRAINT device_marketplace_session_status
        CHECK (status IN ('connected', 'signed_out', 'wiped')),
    CONSTRAINT device_marketplace_session_label_bounded
        CHECK (account_label IS NULL OR char_length(account_label) BETWEEN 1 AND 200)
);

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form: a pooled connection reverts a transaction-local
-- app.current_org to the empty string rather than to missing, so NULLIF is
-- what keeps an unpinned statement matching nothing instead of raising 22P02.
ALTER TABLE device ENABLE ROW LEVEL SECURITY;
ALTER TABLE device FORCE ROW LEVEL SECURITY;
CREATE POLICY device_org_isolation ON device
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE device_marketplace_session ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_marketplace_session FORCE ROW LEVEL SECURITY;
CREATE POLICY device_marketplace_session_org_isolation ON device_marketplace_session
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No grant to any other role. The registry is written and read on the API
-- path alone, under `tam_app` and its forced row-level security: `tam_engine`
-- has no reason to read it, `tam_broker` holds no session for a no-API
-- marketplace to begin with, and `tam_auth` is confined to the `auth` schema
-- and must hold no privilege on any table in `public`.
