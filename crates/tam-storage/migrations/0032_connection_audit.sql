-- The connection lifecycle as an append-only log, following field_audit:
-- the application role holds insert and select and neither update nor
-- delete, and the revocation binds the owner too because privileges are
-- checked against the ACL rather than ownership.
--
-- A connection's own row carries only its current state, so every question
-- about how it got there -- when it was linked, how often a refresh failed
-- before the gate closed, whether a seller or the engine revoked it -- is
-- unanswerable from it. This table is where those are answered.
CREATE TABLE connection_audit (
    id            bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    org_id        uuid        NOT NULL REFERENCES organisation (id),
    connection_id uuid        NOT NULL,
    event         text        NOT NULL,
    actor_kind    text        NOT NULL,
    actor_id      text,
    detail        text,
    at            timestamptz NOT NULL,

    -- Closed here rather than in the writer: a log admitting an unplanned
    -- verb reads as coverage it does not have.
    CONSTRAINT connection_audit_event CHECK (
        event IN (
            'linked', 'claimed', 'refreshed', 'refresh_failed',
            'needs_reauth', 'relinked', 'revoked', 'unlinked'
        )
    ),

    CONSTRAINT connection_audit_actor_kind CHECK (
        actor_kind IN ('person', 'system')
    ),

    -- The sum type as two columns: a person is always identified, a system
    -- component names itself in actor_id and is never anonymous either, but
    -- only the person half is enforced here because the system half's
    -- vocabulary is the writer's to extend.
    CONSTRAINT connection_audit_person_identified CHECK (
        actor_kind <> 'person' OR actor_id IS NOT NULL
    )
);

-- No foreign key to connection, deliberately and in step with field_audit,
-- which names a mapping_id under no constraint either: an audit row must
-- outlive whatever it records, and a log that a delete can block or cascade
-- is not append-only.

-- The read path a seller's connections page and any later export take.
CREATE INDEX connection_audit_by_connection
    ON connection_audit (org_id, connection_id, at DESC);

REVOKE UPDATE, DELETE ON connection_audit FROM tam_app;

-- The two roles that drive a connection's lifecycle: the broker owns link
-- and revoke, the engine owns the needs_reauth gate. Neither may rewrite
-- what it wrote.
GRANT SELECT, INSERT ON connection_audit TO tam_engine;
GRANT SELECT, INSERT ON connection_audit TO tam_broker;

ALTER TABLE connection_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE connection_audit FORCE ROW LEVEL SECURITY;
-- The null-safe form migration 0009 established for every policy after it:
-- a pooled connection reverts a transaction-local pin to the empty string
-- rather than to missing, and NULLIF keeps that failing closed.
CREATE POLICY connection_audit_org_isolation ON connection_audit
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);
