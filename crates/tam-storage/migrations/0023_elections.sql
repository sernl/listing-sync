-- The seller's decision surface, in two tables because the two questions have
-- two different keys.
--
-- reconciliation_item deduplicates on (org, term, target vocabulary) while
-- open, which is what makes it a drain: the first product asks, the answer
-- becomes a projection edge, and every later product finds the edge waiting.
-- An election's answer is not a property of a vocabulary pair. "Which licence
-- do I grant" and "which of the year groups this band covers does this listing
-- mean" are properties of a product against a target, so they cannot
-- deduplicate across products and an index designed to collapse them would
-- collapse the wrong thing. An election is also structurally unrepresentable
-- in reconciliation_item, whose leading dedup column is
-- `term uuid NOT NULL REFERENCES canonical_term` — a supply or over-cap
-- question names no single term.
--
-- The reuse mechanism is a standing rule rather than deduplication: the seller
-- answers once as a policy and the raise path consults the rules before it
-- enqueues anything, which is what reconciles "the seller decides" with "do
-- not ask again".

CREATE TABLE election_rule (
    org_id         uuid        NOT NULL REFERENCES organisation (id),
    inventory      text        NOT NULL,
    axis           text        NOT NULL,
    trigger_kind   text        NOT NULL,
    -- 'free' | 'paid' for supply, the source value's native id for narrow,
    -- and the '' sentinel for the two triggers that generalise to nothing.
    trigger_key    text        NOT NULL,
    answer_kind    text        NOT NULL,
    -- The answered paths, as [{"segments": [..], "native_id": ".."}, ..].
    -- The vocabulary is the row's own (inventory, axis) and is not repeated.
    answer         jsonb       NOT NULL DEFAULT '[]',
    decided_by     text        NOT NULL,
    decided_source text,
    decided_user   uuid,
    decided_org    uuid,
    decided_at     timestamptz NOT NULL,

    PRIMARY KEY (org_id, inventory, axis, trigger_kind, trigger_key),

    -- The column's spelling is pinned for the same reason projection_edge
    -- pins its own kind column: the delegate CHECK below states a rule over a
    -- set of axis values, and a set is only trustworthy if the values are.
    CONSTRAINT election_rule_axis CHECK (
        axis IN ('subject', 'topic', 'resource_type', 'phase', 'licence')
    ),
    CONSTRAINT election_rule_trigger CHECK (
        trigger_kind IN ('supply', 'elect_one', 'over_cap', 'narrow')
    ),
    CONSTRAINT election_rule_answer CHECK (
        answer_kind IN ('value', 'ordering', 'delegate')
    ),
    -- The domain's ElectionRule::new refuses this too. Both layers, because
    -- the domain check passes for anything that writes the row directly.
    -- Stated over a set rather than a value: NonDelegable exists so a second
    -- non-delegable axis must state which bar it clears, and adding one must
    -- extend this list.
    CONSTRAINT election_rule_licence_never_delegated CHECK (
        NOT (answer_kind = 'delegate' AND axis IN ('licence'))
    ),
    CONSTRAINT election_rule_answer_shape CHECK (
        (answer_kind = 'value'    AND jsonb_array_length(answer) = 1)
     OR (answer_kind = 'ordering' AND jsonb_array_length(answer) >= 1)
     OR (answer_kind = 'delegate' AND jsonb_array_length(answer) = 0)
    ),
    -- The sentinel is tied to the trigger kinds that carry no key, so a
    -- narrow rule cannot be stored keyless and answer every band.
    CONSTRAINT election_rule_trigger_key CHECK (
        (trigger_kind IN ('supply', 'narrow')) = (trigger_key <> '')
    ),
    CONSTRAINT election_rule_decider_total CHECK (
        (decided_by = 'imported' AND decided_source IS NOT NULL
            AND decided_user IS NULL AND decided_org IS NULL)
     OR (decided_by = 'human' AND decided_source IS NULL
            AND decided_user IS NOT NULL AND decided_org IS NOT NULL)
    )
);

CREATE TABLE election_item (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    id           uuid        NOT NULL,
    product_id   uuid        NOT NULL,
    inventory    text        NOT NULL,
    axis         text        NOT NULL,
    trigger_kind text        NOT NULL,
    trigger_key  text        NOT NULL,
    -- Nullable: a sync run always names the mapping that raised the question,
    -- and an answer authored on the create form precedes every mapping and
    -- arrives already answered. Both are representable and nothing else is.
    raised_by    uuid,
    raised_at    timestamptz NOT NULL,
    state        text        NOT NULL,
    answer       jsonb       NOT NULL DEFAULT '[]',
    resolved_at  timestamptz,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),
    FOREIGN KEY (org_id, raised_by)  REFERENCES mapping (org_id, id),

    CONSTRAINT election_item_axis CHECK (
        axis IN ('subject', 'topic', 'resource_type', 'phase', 'licence')
    ),
    CONSTRAINT election_item_trigger CHECK (
        trigger_kind IN ('supply', 'elect_one', 'over_cap', 'narrow')
    ),
    CONSTRAINT election_item_trigger_key CHECK (
        (trigger_kind IN ('supply', 'narrow')) = (trigger_key <> '')
    ),
    CONSTRAINT election_item_state CHECK (state IN ('open', 'answered', 'withdrawn')),
    CONSTRAINT election_item_settled CHECK ((state = 'open') = (resolved_at IS NULL)),
    CONSTRAINT election_item_answer_shape CHECK (
        (state <> 'answered' AND jsonb_array_length(answer) = 0)
     OR (state =  'answered' AND jsonb_array_length(answer) >= 1)
    ),
    CONSTRAINT election_item_provenance CHECK (
        raised_by IS NOT NULL OR state = 'answered'
    )
);

-- One open item per question per product. trigger_key is in the key: without
-- it a product whose price flips free to paid keeps the stale free-branch
-- question under ON CONFLICT DO NOTHING, and the seller answers the free
-- question — a Creative Commons value — for a paid listing, which is the one
-- combination Tes refuses.
CREATE UNIQUE INDEX election_item_open_dedup
    ON election_item (org_id, product_id, inventory, axis, trigger_kind, trigger_key)
    WHERE state = 'open';

ALTER TABLE election_rule ENABLE ROW LEVEL SECURITY;
ALTER TABLE election_rule FORCE ROW LEVEL SECURITY;
CREATE POLICY election_rule_org_isolation ON election_rule
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE election_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE election_item FORCE ROW LEVEL SECURITY;
CREATE POLICY election_item_org_isolation ON election_item
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- The engine raises and reads; it never authors a seller's decision. This
-- mirrors 0013's reconciliation_item grant exactly, and the asymmetry is the
-- point: the sync run that discovers the question is the engine's, and making
-- it ask the seller to poll for it would make the decision surface lag the
-- discovery that produced it.
GRANT SELECT, INSERT ON election_item TO tam_engine;
GRANT SELECT             ON election_rule TO tam_engine;
