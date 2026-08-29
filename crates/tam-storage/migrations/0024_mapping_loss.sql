-- What a projection could not carry, recorded against the mapping that
-- carried it. A loss never blocks -- that is the whole difference between
-- disclosing and deciding -- so without a durable record it is a log line,
-- which the direction rules out.

CREATE TABLE mapping_loss (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    -- No foreign key, and deliberately, following field_audit: the record
    -- outlives the mapping. An ON DELETE CASCADE here would erase the history
    -- that is this table's stated reason to exist, so severing a mapping must
    -- not take its losses with it.
    mapping_id  uuid        NOT NULL,
    attempt     uuid        NOT NULL,
    position    int         NOT NULL,
    kind        text        NOT NULL,
    -- Only a no-target-field loss names an axis: the others are about values
    -- within an axis the target does bind.
    axis        text,
    -- The variant's own payload, whole. One representation rather than a
    -- scalar column per variant, because the four losses carry four different
    -- shapes -- dropped term ids, a source path list, an elected subset and
    -- its decider -- and flattening them into shared columns would record
    -- three of the four approximately.
    detail      jsonb       NOT NULL DEFAULT '{}',
    recorded_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, mapping_id, attempt, position),

    CONSTRAINT mapping_loss_kind CHECK (
        kind IN ('broadened', 'no_target_field', 'collapsed', 'elected')
    ),
    CONSTRAINT mapping_loss_axis CHECK ((kind = 'no_target_field') = (axis IS NOT NULL)),
    CONSTRAINT mapping_loss_axis_value CHECK (
        axis IS NULL
     OR axis IN ('subject', 'topic', 'resource_type', 'phase', 'licence')
    )
);

ALTER TABLE mapping_loss ENABLE ROW LEVEL SECURITY;
ALTER TABLE mapping_loss FORCE ROW LEVEL SECURITY;
CREATE POLICY mapping_loss_org_isolation ON mapping_loss
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- Append-only as a fence rather than as a convention. tam_app owns every
-- table, so an unrevoked table is fully mutable by the API role and
-- "append-only" would be a word in a comment; this mirrors 0006's revoke on
-- field_audit.
REVOKE UPDATE, DELETE ON mapping_loss FROM tam_app;

-- The engine records what its own projection could not carry. INSERT and
-- SELECT only, which keeps 0007's stated rule intact: a re-projection writes
-- a new attempt's rows rather than replacing the old ones, so no delete grant
-- is ever needed.
--
-- Retention is deferred, and said so rather than implied: pruning.rs deletes
-- job_event and nothing else, so an append-per-attempt table on the sync hot
-- path has no pruner yet.
GRANT SELECT, INSERT ON mapping_loss TO tam_engine;
