-- Halt scopes, the rate budget, the connection aggregate, its secret store
-- and the append-only field audit. schema.md describes these in prose; the
-- column shapes are derived here and recorded as such. All three halt scopes
-- fail closed: a worker that cannot read them refuses to automate.

-- Every tenant, one inventory: the fleet kill switch. Operator-scoped
-- reference of fleet state, deliberately NOT tenant data and NOT under RLS.
CREATE TABLE inventory_halt (
    inventory   text        NOT NULL,
    marketplace text        NOT NULL,
    raised_by   text        NOT NULL,
    reason      text        NOT NULL,
    raised_at   timestamptz NOT NULL,

    PRIMARY KEY (inventory, marketplace),
    FOREIGN KEY (inventory, marketplace)
        REFERENCES marketplace_inventory (code, marketplace)
);

-- One tenant, every inventory.
CREATE TABLE org_halt (
    org_id    uuid        NOT NULL REFERENCES organisation (id),
    raised_by text        NOT NULL,
    reason    text        NOT NULL,
    raised_at timestamptz NOT NULL,

    PRIMARY KEY (org_id)
);

-- One tenant, one inventory: what an ambiguous create raises.
CREATE TABLE org_inventory_halt (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    inventory   text        NOT NULL,
    marketplace text        NOT NULL,
    raised_by   text        NOT NULL,
    reason      text        NOT NULL,
    raised_at   timestamptz NOT NULL,

    PRIMARY KEY (org_id, inventory, marketplace),
    FOREIGN KEY (inventory, marketplace)
        REFERENCES marketplace_inventory (code, marketplace)
);

-- One row per tenant per marketplace, under the recorded assumption that one
-- author login reaches both Tes inventories; if the M-1 probe finds two
-- logins, the unique constraint is dropped by a forward migration and the
-- budget follows without a schema change.
CREATE TABLE connection (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    id          uuid        NOT NULL,
    marketplace text        NOT NULL,
    state       text        NOT NULL,
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT connection_one_per_marketplace UNIQUE (org_id, marketplace),

    CONSTRAINT connection_state CHECK (
        state IN ('unlinked', 'linking', 'linked', 'needs_reauth', 'revoked')
    ),

    CONSTRAINT connection_marketplace CHECK (
        marketplace IN ('tes', 'etsy', 'tpt')
    )
);

-- Keyed on the connection rather than the inventory, because the budget it
-- protects is the marketplace's own per-account fair-usage counter and a
-- connection is exactly one marketplace account.
CREATE TABLE rate_budget (
    org_id        uuid        NOT NULL REFERENCES organisation (id),
    connection_id uuid        NOT NULL,
    window_start  timestamptz NOT NULL,
    actions_used  int         NOT NULL DEFAULT 0,

    PRIMARY KEY (org_id, connection_id, window_start),
    FOREIGN KEY (org_id, connection_id) REFERENCES connection (org_id, id)
);

-- The wrapped data-encryption key, nonce, ciphertext and AAD context. Under
-- RLS today; the M1e credential broker adds its own database role and this
-- table becomes selectable by that role alone, which is why no repository in
-- this crate reads it.
CREATE TABLE connection_secret (
    org_id        uuid        NOT NULL REFERENCES organisation (id),
    connection_id uuid        NOT NULL,
    key_version   int         NOT NULL,
    wrapped_dek   bytea       NOT NULL,
    nonce         bytea       NOT NULL,
    ciphertext    bytea       NOT NULL,
    aad           bytea       NOT NULL,
    created_at    timestamptz NOT NULL,

    PRIMARY KEY (org_id, connection_id, key_version),
    FOREIGN KEY (org_id, connection_id) REFERENCES connection (org_id, id)
);

-- Append-only: the application role holds insert and select and neither
-- update nor delete, and rows ship off-box continuously.
CREATE TABLE field_audit (
    id                 bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    org_id             uuid        NOT NULL REFERENCES organisation (id),
    mapping_id         uuid        NOT NULL,
    field              text        NOT NULL,
    intended           text,
    observed_before    text,
    observed_after     text,
    class              text        NOT NULL,
    normaliser_version int         NOT NULL,
    created_at         timestamptz NOT NULL
);

REVOKE UPDATE, DELETE ON field_audit FROM tam_app;

ALTER TABLE org_halt ENABLE ROW LEVEL SECURITY;
ALTER TABLE org_halt FORCE ROW LEVEL SECURITY;
CREATE POLICY org_halt_org_isolation ON org_halt
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE org_inventory_halt ENABLE ROW LEVEL SECURITY;
ALTER TABLE org_inventory_halt FORCE ROW LEVEL SECURITY;
CREATE POLICY org_inventory_halt_org_isolation ON org_inventory_halt
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE rate_budget ENABLE ROW LEVEL SECURITY;
ALTER TABLE rate_budget FORCE ROW LEVEL SECURITY;
CREATE POLICY rate_budget_org_isolation ON rate_budget
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE connection ENABLE ROW LEVEL SECURITY;
ALTER TABLE connection FORCE ROW LEVEL SECURITY;
CREATE POLICY connection_org_isolation ON connection
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE connection_secret ENABLE ROW LEVEL SECURITY;
ALTER TABLE connection_secret FORCE ROW LEVEL SECURITY;
CREATE POLICY connection_secret_org_isolation ON connection_secret
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE field_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE field_audit FORCE ROW LEVEL SECURITY;
CREATE POLICY field_audit_org_isolation ON field_audit
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);
