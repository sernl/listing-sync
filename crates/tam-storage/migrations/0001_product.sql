-- The tenancy root. Every tenant table references it; its full shape is a
-- later change (docs/design/schema.md defines tenant tables against it but
-- carries no organisation DDL yet, so this is the minimum viable target).
CREATE TABLE organisation (
    id         uuid        NOT NULL,
    name       text        NOT NULL,
    created_at timestamptz NOT NULL,

    PRIMARY KEY (id)
);

CREATE TABLE product (
    org_id            uuid        NOT NULL REFERENCES organisation (id),
    id                uuid        NOT NULL,
    title             text        NOT NULL,
    body              text        NOT NULL,
    price_kind        text        NOT NULL,
    price_minor_units bigint,
    price_currency    text,
    created_at        timestamptz NOT NULL,
    updated_at        timestamptz NOT NULL,
    deleted_at        timestamptz,

    PRIMARY KEY (org_id, id),

    CONSTRAINT product_price_total CHECK (
        (price_kind = 'free' AND price_minor_units IS NULL AND price_currency IS NULL)
     OR (price_kind = 'paid' AND price_minor_units > 0 AND price_currency IS NOT NULL)
    )
);

-- Row-level security keyed on the tenant. FORCE applies the policy to the
-- table owner too, which is exactly the role the application and the tests
-- connect as; without it the policy would be theatre. current_setting's
-- missing_ok form makes an undeclared tenant read as NULL and match nothing,
-- so a connection that never pinned its organisation sees an empty table
-- rather than every tenant's rows.
ALTER TABLE product ENABLE ROW LEVEL SECURITY;
ALTER TABLE product FORCE ROW LEVEL SECURITY;

CREATE POLICY product_org_isolation ON product
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);
