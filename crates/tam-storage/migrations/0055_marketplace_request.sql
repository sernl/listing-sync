-- Where else a seller sells: the marketplaces they ask us to support.
--
-- A request form on the console's Marketplaces page is the only writer, and
-- this is where what a seller types there lands. It is a message rather than a
-- record of anything the platform does: nothing reads these rows to decide a
-- transport, a schedule or an entitlement, and the closed `Marketplace` enum in
-- `tam-types` stays the only thing that says which marketplaces exist. Adding
-- one is a founder decision informed by this table, never an effect of it.
--
-- Tenant data rather than global, because the request names what one seller
-- sells and where: `url` is a shop of theirs and `reason` describes their own
-- business. An operator reads across tenants through the enumerated policy at
-- the foot of this file, which is the same crossing migration 0037 built for
-- every other table the backoffice reads.
--
-- The three text columns are bounded here as well as at the API boundary. The
-- API bound is what gives a seller a sentence to act on; this one is what makes
-- the bound true of the column no matter which writer reaches it.
CREATE TABLE marketplace_request (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    id           uuid        NOT NULL,
    -- Who asked, kept because an operator answering a request writes back to a
    -- person rather than to an organisation. A plain reference to the global
    -- app_user table, as platform_operator makes: app_user is keyed on id
    -- alone, and the row's own org_id is what fences it.
    requested_by uuid        NOT NULL REFERENCES app_user (id),
    name         text        NOT NULL,
    url          text        NOT NULL,
    reason       text        NOT NULL,
    created_at   timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT marketplace_request_name_bounded CHECK (
        char_length(name) BETWEEN 1 AND 120
    ),
    CONSTRAINT marketplace_request_url_bounded CHECK (
        char_length(url) BETWEEN 1 AND 2048
    ),
    CONSTRAINT marketplace_request_reason_bounded CHECK (
        char_length(reason) BETWEEN 1 AND 2000
    )
);

-- The operator listing is newest first across every tenant, and the primary
-- key leads with the organisation, so it cannot serve that order.
CREATE INDEX marketplace_request_newest
    ON marketplace_request (created_at DESC, id DESC);

ALTER TABLE marketplace_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE marketplace_request FORCE ROW LEVEL SECURITY;
CREATE POLICY marketplace_request_org_isolation ON marketplace_request
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- The operator's cross-tenant read, in the two halves migration 0037
-- established: tam_backoffice is not BYPASSRLS, so the grant alone shows it
-- nothing and the permissive SELECT policy beside the tenant one is what lets
-- it read past the pin. FOR SELECT rather than FOR ALL, so the read-only
-- property does not rest on the grant list alone.
GRANT SELECT ON marketplace_request TO tam_backoffice;
CREATE POLICY marketplace_request_backoffice_read ON marketplace_request
    FOR SELECT TO tam_backoffice USING (true);
