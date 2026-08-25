-- The job ledger, the write-attempt fencing token and the outbox.
-- job_item, job_event, write_attempt and outbox_message follow
-- docs/design/schema.md verbatim. job is derived minimal (status is a total
-- function of the item states, computed and never stored), and
-- org_event_counter is the per-organisation row job_event.org_seq is
-- allocated from by locking it in the same transaction as the state change.

CREATE TABLE job (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    id          uuid        NOT NULL,
    inventory   text        NOT NULL,
    marketplace text        NOT NULL,
    created_at  timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (inventory, marketplace)
        REFERENCES marketplace_inventory (code, marketplace)
);

CREATE TABLE job_item (
    org_id           uuid        NOT NULL REFERENCES organisation (id),
    id               uuid        NOT NULL,
    job_id           uuid        NOT NULL,
    mapping_id       uuid        NOT NULL,
    idempotency_key  uuid        NOT NULL,

    state            text        NOT NULL,
    outcome          text,
    failure_code     text,
    failure_detail   text,
    blocked_on       text,
    evidence_ref     text,
    attempt_count    int         NOT NULL DEFAULT 0,

    lease_owner      text,
    lease_epoch      bigint      NOT NULL DEFAULT 0,
    lease_expires_at timestamptz,
    park_expires_at  timestamptz,

    created_at       timestamptz NOT NULL,
    settled_at       timestamptz,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, job_id)     REFERENCES job (org_id, id),
    FOREIGN KEY (org_id, mapping_id) REFERENCES mapping (org_id, id),

    CONSTRAINT job_item_idempotent UNIQUE (org_id, idempotency_key),

    CONSTRAINT job_item_lease_total CHECK (
        (state IN ('leased', 'running', 'verifying'))
      = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
    ),

    CONSTRAINT job_item_settled_total CHECK (
        (state = 'settled') = (outcome IS NOT NULL AND settled_at IS NOT NULL)
    )
);

CREATE TABLE org_event_counter (
    org_id   uuid   NOT NULL REFERENCES organisation (id),
    next_seq bigint NOT NULL DEFAULT 1,

    PRIMARY KEY (org_id)
);

CREATE TABLE job_event (
    id          bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    org_seq     bigint      NOT NULL,
    job_id      uuid        NOT NULL,
    job_item_id uuid,
    kind        text        NOT NULL,
    payload     jsonb       NOT NULL,
    created_at  timestamptz NOT NULL,

    FOREIGN KEY (org_id, job_id) REFERENCES job (org_id, id),
    CONSTRAINT job_event_org_seq UNIQUE (org_id, org_seq)
);

-- The row is written before the click rather than after the response, because
-- the commit boundary is intent recorded rather than response received.
CREATE TABLE write_attempt (
    org_id             uuid        NOT NULL REFERENCES organisation (id),
    id                 uuid        NOT NULL,
    job_item_id        uuid        NOT NULL,
    mapping_id         uuid        NOT NULL,
    lease_epoch        bigint      NOT NULL,
    intent             jsonb       NOT NULL,
    intent_hash        bytea       NOT NULL,
    correlation_marker text,
    state              text        NOT NULL,
    opened_at          timestamptz NOT NULL,
    settled_at         timestamptz,
    remote_id_kind     text,
    remote_url         text,
    remote_numeric_id  bigint,
    failure_code       text,
    ambiguity_cause    text,
    evidence_ref       text,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, job_item_id) REFERENCES job_item (org_id, id),
    FOREIGN KEY (org_id, mapping_id)  REFERENCES mapping  (org_id, id)
);

-- Makes a duplicate-upload storm structurally impossible rather than merely
-- unlikely: a retry racing a hung worker fails at the database instead of at
-- the marketplace.
CREATE UNIQUE INDEX write_attempt_one_in_flight
    ON write_attempt (org_id, mapping_id)
    WHERE state = 'in_flight';

-- At-least-once delivery to parties that offer idempotency; the row's own id
-- is the value sent as the downstream idempotency key. Marketplace writes
-- never travel through here.
CREATE TABLE outbox_message (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    id           uuid        NOT NULL,
    topic        text        NOT NULL,
    dedupe_key   text        NOT NULL,
    payload      jsonb       NOT NULL,
    state        text        NOT NULL DEFAULT 'pending',
    created_at   timestamptz NOT NULL,
    available_at timestamptz NOT NULL,
    attempts     int         NOT NULL DEFAULT 0,
    delivered_at timestamptz,
    last_error   text,

    PRIMARY KEY (org_id, id),
    CONSTRAINT outbox_dedupe UNIQUE (org_id, topic, dedupe_key),
    CONSTRAINT outbox_state_total CHECK (
        (state = 'delivered') = (delivered_at IS NOT NULL)
    )
);

ALTER TABLE job ENABLE ROW LEVEL SECURITY;
ALTER TABLE job FORCE ROW LEVEL SECURITY;
CREATE POLICY job_org_isolation ON job
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE job_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_item FORCE ROW LEVEL SECURITY;
CREATE POLICY job_item_org_isolation ON job_item
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE org_event_counter ENABLE ROW LEVEL SECURITY;
ALTER TABLE org_event_counter FORCE ROW LEVEL SECURITY;
CREATE POLICY org_event_counter_org_isolation ON org_event_counter
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE job_event ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_event FORCE ROW LEVEL SECURITY;
CREATE POLICY job_event_org_isolation ON job_event
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE write_attempt ENABLE ROW LEVEL SECURITY;
ALTER TABLE write_attempt FORCE ROW LEVEL SECURITY;
CREATE POLICY write_attempt_org_isolation ON write_attempt
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE outbox_message ENABLE ROW LEVEL SECURITY;
ALTER TABLE outbox_message FORCE ROW LEVEL SECURITY;
CREATE POLICY outbox_message_org_isolation ON outbox_message
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);
