-- The durable half of the taxonomy hub; the pure projection over this
-- relation lives in tam-taxonomy. projection_edge and
-- projection_no_counterpart are global reference data like canonical_term:
-- the canonical taxonomy is ours, not a tenant's. reconciliation_item is
-- tenant data under the forced null-safe policy.

CREATE TABLE projection_edge (
    from_term      uuid        NOT NULL REFERENCES canonical_term (id),
    to_inventory   text        NOT NULL,
    to_term_kind   text        NOT NULL,
    to_segments    text[]      NOT NULL,
    to_native_id   text,
    kind           text        NOT NULL,
    decided_by     text        NOT NULL,
    decided_source text,
    decided_user   uuid,
    decided_org    uuid,
    decided_at     timestamptz NOT NULL,

    PRIMARY KEY (from_term, to_inventory, to_term_kind, to_segments, kind),

    CONSTRAINT projection_edge_kind CHECK (
        kind IN ('exact', 'broader', 'narrower')
    ),
    CONSTRAINT projection_edge_term_kind CHECK (
        to_term_kind IN ('subject', 'topic', 'resource_type', 'phase')
    ),
    CONSTRAINT projection_edge_decider_total CHECK (
        (decided_by = 'imported' AND decided_source IS NOT NULL
            AND decided_user IS NULL AND decided_org IS NULL)
     OR (decided_by = 'human' AND decided_source IS NULL
            AND decided_user IS NOT NULL AND decided_org IS NOT NULL)
    )
);

-- Inbound projection is the reverse of Exact edges only, so a target path may
-- be claimed as Exact by at most one canonical term per vocabulary:
-- many-to-one is legal through Broader and illegal through Exact, rejected at
-- insert rather than resolved at read time.
CREATE UNIQUE INDEX projection_edge_exact_reverse
    ON projection_edge (to_inventory, to_term_kind, to_segments)
    WHERE kind = 'exact';

-- The durable record that a term genuinely has no counterpart in a
-- vocabulary, which turns Absent from a publish blocker into an omission.
CREATE TABLE projection_no_counterpart (
    term           uuid        NOT NULL REFERENCES canonical_term (id),
    to_inventory   text        NOT NULL,
    to_term_kind   text        NOT NULL,
    decided_by     text        NOT NULL,
    decided_source text,
    decided_user   uuid,
    decided_org    uuid,
    decided_at     timestamptz NOT NULL,

    PRIMARY KEY (term, to_inventory, to_term_kind),

    CONSTRAINT projection_no_counterpart_decider_total CHECK (
        (decided_by = 'imported' AND decided_source IS NOT NULL
            AND decided_user IS NULL AND decided_org IS NULL)
     OR (decided_by = 'human' AND decided_source IS NULL
            AND decided_user IS NOT NULL AND decided_org IS NOT NULL)
    )
);

CREATE TABLE reconciliation_item (
    org_id           uuid        NOT NULL REFERENCES organisation (id),
    id               uuid        NOT NULL,
    term             uuid        NOT NULL REFERENCES canonical_term (id),
    target_inventory text        NOT NULL,
    target_term_kind text        NOT NULL,
    raised_by        uuid        NOT NULL,
    raised_at        timestamptz NOT NULL,
    state            text        NOT NULL,
    resolved_at      timestamptz,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, raised_by) REFERENCES mapping (org_id, id),

    CONSTRAINT reconciliation_item_state CHECK (
        state IN ('open', 'resolved', 'no_counterpart')
    ),
    CONSTRAINT reconciliation_item_settled CHECK (
        (state = 'open') = (resolved_at IS NULL)
    )
);

-- A batch raises one item per gap, not one per product.
CREATE UNIQUE INDEX reconciliation_item_open_dedup
    ON reconciliation_item (org_id, term, target_inventory, target_term_kind)
    WHERE state = 'open';

ALTER TABLE reconciliation_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE reconciliation_item FORCE ROW LEVEL SECURITY;
CREATE POLICY reconciliation_item_org_isolation ON reconciliation_item
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- The worker projects terms while executing a job, so the engine reads the
-- relation and raises items; resolution stays on the API path, where tam_app
-- already holds owner privileges.
GRANT SELECT ON canonical_term, projection_edge, projection_no_counterpart
    TO tam_engine;
GRANT SELECT, INSERT ON reconciliation_item TO tam_engine;
