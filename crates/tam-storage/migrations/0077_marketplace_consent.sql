-- The seller's explicit permission for a marketplace that publishes no
-- official API.
--
-- D1 puts every no-API marketplace request on the seller's own device under
-- the seller's own sign-in, which is something the seller has to agree to
-- knowingly: the console shows a notice naming what the connection reads,
-- what it does and what stays on the device, and the seller ticks a box and
-- presses "I agree". This table is that agreement, recorded once per
-- organisation and per marketplace rather than per device, so every machine
-- the seller runs reads one answer and none of them can hold a private one.
--
-- The record is append-only: a withdrawal fills `withdrawn_*` on the standing
-- row rather than deleting it, and a fresh grant is a new row. The notice
-- itself is versioned by date, and a grant carries the version the seller
-- read, so a notice that changes materially makes every older grant stop
-- standing until the seller reads and agrees to the new one.
CREATE TABLE marketplace_consent (
    org_id          uuid        NOT NULL REFERENCES organisation (id),
    marketplace     text        NOT NULL,
    notice_version  text        NOT NULL,
    granted_by      uuid        NOT NULL,
    granted_at      timestamptz NOT NULL,
    withdrawn_by    uuid,
    withdrawn_at    timestamptz,

    PRIMARY KEY (org_id, marketplace, granted_at),

    CONSTRAINT marketplace_consent_withdrawal_pair
        CHECK ((withdrawn_by IS NULL) = (withdrawn_at IS NULL)),
    CONSTRAINT marketplace_consent_version_bounded
        CHECK (char_length(notice_version) BETWEEN 1 AND 32)
);

-- At most one grant stands per marketplace, whatever version it carries: a
-- grant on a newer notice withdraws the older one in the same transaction.
CREATE UNIQUE INDEX marketplace_consent_standing
    ON marketplace_consent (org_id, marketplace)
    WHERE withdrawn_at IS NULL;

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form: a pooled connection reverts a transaction-local
-- app.current_org to the empty string rather than to missing, so NULLIF is
-- what keeps an unpinned statement matching nothing instead of raising 22P02.
ALTER TABLE marketplace_consent ENABLE ROW LEVEL SECURITY;
ALTER TABLE marketplace_consent FORCE ROW LEVEL SECURITY;
CREATE POLICY marketplace_consent_org_isolation ON marketplace_consent
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No grant to any other role. The record is written and read on the API path
-- alone, under `tam_app` and its forced row-level security; the worker that
-- mints jobs reads it through the same repository.
