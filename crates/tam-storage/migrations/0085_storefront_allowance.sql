-- Which storefront has already been given its free moves, and to whom.
--
-- The free tier grants five moves once, for life. "Once" has to mean once per
-- shop rather than once per organisation, because an organisation costs
-- nothing to create and a seller who wanted fifty free moves would make ten
-- of them. The shop is identified the way `connection.platform_account_digest`
-- identifies it -- a keyed digest of the marketplace's own account id, so the
-- server holds no seller identifier it does not need -- and that digest is
-- this table's key.
--
-- The primary key is (marketplace, platform_account_digest) and not the
-- organisation, which is the whole point: the row says "this shop has had its
-- five", and the organisation on it is provenance rather than scope. Row-level
-- security still fences reads to the owning tenant, and the guarantee survives
-- it, because a unique violation is raised on rows the pin cannot see. A
-- second organisation's INSERT ... ON CONFLICT DO NOTHING therefore inserts
-- nothing and grants nothing, without ever being told whose shop it was.
--
-- Written in the same transaction as the bind it records. A row here with no
-- credit in `move_ledger`, or a credit with no row here, would each be a way
-- to lose or repeat the grant, and neither is reachable if the two are one
-- write.
CREATE TABLE storefront_allowance (
    marketplace              text        NOT NULL,
    platform_account_digest  bytea       NOT NULL,
    org_id                   uuid        NOT NULL REFERENCES organisation (id),
    granted_at               timestamptz NOT NULL,

    PRIMARY KEY (marketplace, platform_account_digest),

    CONSTRAINT storefront_allowance_marketplace_said CHECK (marketplace <> ''),
    CONSTRAINT storefront_allowance_digest_sized CHECK (
        octet_length(platform_account_digest) = 32
    )
);

-- "Which shops has this organisation claimed" -- the read the Account page
-- makes and the only one keyed the other way round from the primary key.
CREATE INDEX storefront_allowance_by_org
    ON storefront_allowance (org_id, granted_at DESC);

-- The tenant fence, in 0009's null-safe form. It fences reads; the primary
-- key above is what fences the grant itself, across every tenant.
ALTER TABLE storefront_allowance ENABLE ROW LEVEL SECURITY;
ALTER TABLE storefront_allowance FORCE ROW LEVEL SECURITY;
CREATE POLICY storefront_allowance_org_isolation ON storefront_allowance
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);
