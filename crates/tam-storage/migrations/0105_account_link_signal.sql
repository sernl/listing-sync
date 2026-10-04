-- Abuse prevention, part one: the linkage ledger, and the hole in the free
-- moves that account deletion opened.
--
-- docs/notes/design/abuse-prevention.md is the long form. The loop this
-- answers is free-moves farming: make a free account, connect a shop, spend
-- the five free moves, unlink, make the next account. `storefront_allowance`
-- (0085) already keys the five to the shop rather than to the account, so the
-- second account connecting the same shop is given nothing. Two things were
-- missing around it.
--
-- 1. storefront_grant_record. erase_organisation (0092) deletes every row
--    carrying the departing org_id, storefront_allowance included, so a seller
--    who deleted their account freed their shop's five for the next account.
--    This table remembers that a shop has had its five with no org_id column
--    at all, which is what keeps the erasure walk from finding it: the row
--    names a keyed digest of the shop (0031) and a date, and nothing that
--    identifies a person. Written in the same transaction as the credit, and
--    only when moves were actually credited. Global: there is no tenant to
--    fence a fact about a shop by, and the application role writes it from
--    inside a pinned heartbeat transaction.
--
-- 2. account_link_signal. What two accounts can be seen to share: a shop, an
--    install of the desktop app, a sign-in address, a browser, a card, an
--    email domain. Every value is a 32-byte keyed digest -- the shop's is the
--    0031 digest as it stands, the rest are HMAC-SHA-256 under a pepper the
--    server derives from its key-encryption key (tam_secrets::abuse_digest) --
--    so the table holds no address, id or card the operator could read back.
--    One row per (organisation, kind, value), with when it was first and last
--    seen; the scorer clusters organisations that share a value.
--
--    Global rather than tenant-fenced, because its whole purpose is the
--    comparison across tenants, and a fence would hide exactly the rows the
--    scorer needs. org_id is a real foreign key and a real column, so an
--    erasure deletes an organisation's signals with it: a deleted account
--    leaves nothing here. The address and browser rows go after ninety days
--    regardless (tam-server's abuse loop prunes them); the rest last as long
--    as the account.
--
--    Append-only in the sense that matters: a row's identity and first
--    sighting never change. last_seen moves forward on every new sighting;
--    the trigger below refuses any other rewrite.

CREATE TABLE storefront_grant_record (
    marketplace              text        NOT NULL,
    platform_account_digest  bytea       NOT NULL,
    granted_at               timestamptz NOT NULL,

    PRIMARY KEY (marketplace, platform_account_digest),

    CONSTRAINT storefront_grant_record_marketplace_said CHECK (marketplace <> ''),
    CONSTRAINT storefront_grant_record_digest_sized CHECK (
        octet_length(platform_account_digest) = 32
    )
);

-- Every shop already claimed, so the record starts complete rather than only
-- from this release. Every claim rather than only the credited ones: the
-- claims that credited nothing (a second shop of one organisation) are the
-- conservative side to err on, and the difference is a handful of rows.
INSERT INTO storefront_grant_record (marketplace, platform_account_digest, granted_at)
SELECT marketplace, platform_account_digest, granted_at
  FROM storefront_allowance;

CREATE TABLE account_link_signal (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    kind        text        NOT NULL,
    value_hash  bytea       NOT NULL,
    first_seen  timestamptz NOT NULL,
    last_seen   timestamptz NOT NULL,

    PRIMARY KEY (org_id, kind, value_hash),

    CONSTRAINT account_link_signal_kind_known CHECK (
        kind IN ('shop_digest', 'device_fingerprint', 'ip', 'email_domain',
                 'payment_fingerprint', 'user_agent_hash')
    ),
    CONSTRAINT account_link_signal_value_sized CHECK (octet_length(value_hash) = 32),
    CONSTRAINT account_link_signal_seen_ordered CHECK (last_seen >= first_seen)
);

-- The scorer's join: every organisation holding one value.
CREATE INDEX account_link_signal_by_value ON account_link_signal (kind, value_hash);
-- The ninety-day prune of addresses and browsers.
CREATE INDEX account_link_signal_by_last_seen ON account_link_signal (kind, last_seen);

CREATE FUNCTION refuse_signal_rewrite() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.org_id IS DISTINCT FROM OLD.org_id
       OR NEW.kind IS DISTINCT FROM OLD.kind
       OR NEW.value_hash IS DISTINCT FROM OLD.value_hash
       OR NEW.first_seen IS DISTINCT FROM OLD.first_seen
       OR NEW.last_seen < OLD.last_seen THEN
        RAISE EXCEPTION 'account_link_signal is append-only; only last_seen moves, and only forward'
            USING ERRCODE = 'insufficient_privilege';
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER account_link_signal_append_only
    BEFORE UPDATE ON account_link_signal
    FOR EACH ROW EXECUTE FUNCTION refuse_signal_rewrite();

-- The operators read the ledger from the Abuse page on the backoffice pool.
-- A global table, so the grant alone opens it; no policy is needed.
GRANT SELECT ON account_link_signal TO tam_backoffice;
