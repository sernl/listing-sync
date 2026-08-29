-- A sync's first leg, which is a marketplace read and not a write, so it does
-- not belong in the item ledger.
--
-- `prepare_item` is deliberately adapter-free and pool-only, so an item that
-- cannot run never costs a gateway session; folding a read into it deletes
-- that property. The stored operation set is isomorphic to `ItemOperation`'s
-- variants, and a fourth read operation reopens the CHECK, the digest, the
-- admission gate and the machine for a leg that writes nothing. And a job
-- carries one inventory, so a job holding both legs of a cross-platform sync
-- is not expressible at all.
--
-- So the read leg is a request row drained by a pass outside the item pump,
-- under the tenant's own role. On success the drain mints the write jobs and
-- records their ids here, which is what the seller polls between asking for a
-- sync and the ledger having something to show them.

CREATE TABLE sync_request (
    org_id         uuid        NOT NULL REFERENCES organisation (id),
    id             uuid        NOT NULL,
    -- Inventory codes, unconstrained by a foreign key for the same reason
    -- election_rule.inventory is: the pair (code, marketplace) is what
    -- marketplace_inventory is keyed on, and the repo's own round trip
    -- through InventoryId is what admits a value here.
    source         text        NOT NULL,
    target         text        NOT NULL,
    -- A sync leaves the source listing alone; a migrate removes it once the
    -- target is bound. The removal's job is the second one below.
    disposition    text        NOT NULL,
    intent         text        NOT NULL,
    state          text        NOT NULL,
    -- The two jobs a request produces. A migrate needs both, and they are
    -- separate rows in `job` because `job.inventory` is single-valued. Each
    -- is created under its own request key derived from this id, because one
    -- key per request would make the second creation a replay of the first
    -- and silently drop its items.
    create_job_id  uuid,
    remove_job_id  uuid,
    failure_detail text,
    requested_at   timestamptz NOT NULL,
    settled_at     timestamptz,

    PRIMARY KEY (org_id, id),

    CONSTRAINT sync_request_disposition CHECK (disposition IN ('sync', 'migrate')),
    CONSTRAINT sync_request_intent CHECK (intent IN ('draft', 'live')),
    CONSTRAINT sync_request_state CHECK (
        state IN ('pending', 'draining', 'enqueued', 'failed')
    ),
    -- A settled request states when, and an unsettled one states nothing.
    CONSTRAINT sync_request_settled_total CHECK (
        (state IN ('pending', 'draining')) = (settled_at IS NULL)
    ),
    CONSTRAINT sync_request_failure_detail CHECK (
        (failure_detail IS NULL) OR state = 'failed'
    )
);

CREATE TABLE sync_request_resource (
    org_id         uuid NOT NULL REFERENCES organisation (id),
    request_id     uuid NOT NULL,
    ordinal        int  NOT NULL,
    -- How the seller addressed the listing: a Tes resource id or a TPT
    -- product id, kept as the seller wrote it so the drain resolves it
    -- against the source adapter rather than against a guess made here.
    locator        text NOT NULL,
    state          text NOT NULL,
    -- What canonicalising this resource produced. The drain writes both in
    -- the same transaction that marks the row canonicalised and skips any
    -- row that already carries them, which is what makes a redrained request
    -- resumable per resource rather than only at its last step: `import_one`
    -- commits four times internally and mints a fresh product id on every
    -- pass, so without this breadcrumb a second pass is a legal duplicate.
    product_id     uuid,
    mapping_id     uuid,
    failure_detail text,

    PRIMARY KEY (org_id, request_id, ordinal),
    FOREIGN KEY (org_id, request_id) REFERENCES sync_request (org_id, id),

    CONSTRAINT sync_request_resource_state CHECK (
        state IN ('pending', 'canonicalised', 'failed')
    ),
    CONSTRAINT sync_request_resource_canonicalised_total CHECK (
        (state = 'canonicalised')
            = (product_id IS NOT NULL AND mapping_id IS NOT NULL)
    )
);

ALTER TABLE sync_request ENABLE ROW LEVEL SECURITY;
ALTER TABLE sync_request FORCE ROW LEVEL SECURITY;
CREATE POLICY sync_request_org_isolation ON sync_request
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE sync_request_resource ENABLE ROW LEVEL SECURITY;
ALTER TABLE sync_request_resource FORCE ROW LEVEL SECURITY;
CREATE POLICY sync_request_resource_org_isolation ON sync_request_resource
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No engine grant, deliberately. The drain runs as tam_app in its own
-- process, which has forced RLS and pins one tenant per request, so the
-- cross-tenant role never sees a sync request at all.
