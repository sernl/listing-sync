-- The seller's own mapping decisions, which override the global relation for
-- their organisation and no other.
--
-- projection_edge is global reference data because an edge is a fact about two
-- vocabularies and the canonical taxonomy is ours rather than a tenant's. An
-- override is neither of those things: it is one tenant's decision about their
-- own catalogue. So this is tenant data under the forced null-safe policy, and
-- the row-level precedent it copies is election_rule (0023) rather than
-- projection_edge (0013).
--
-- It carries no reverse-uniqueness index, and the absence is deliberate. The
-- global exact-reverse index enforces a property of the shared relation:
-- inbound resolution is the reverse of the Exact edges alone, so two canonical
-- terms may not both claim one target path, or that reverse stops being a
-- function. An override is consulted outbound only and never inverted, so a
-- seller pointing two of their own terms at one target path breaks nothing
-- that index exists to protect.

CREATE TABLE projection_override (
    org_id         uuid        NOT NULL REFERENCES organisation (id),
    inventory      text        NOT NULL,
    axis           text        NOT NULL,
    from_term      uuid        NOT NULL REFERENCES canonical_term (id),
    to_segments    text[]      NOT NULL,
    to_native_id   text,
    kind           text        NOT NULL,
    decided_by     text        NOT NULL,
    decided_source text,
    decided_user   uuid,
    decided_org    uuid,
    decided_at     timestamptz NOT NULL,

    -- One decision per axis per term per target inventory. A seller who
    -- changes their mind replaces the row rather than adding a second one,
    -- which is what keeps the precedence rule single-valued: an override wins
    -- over the relation, so two of them would reintroduce exactly the
    -- ambiguity the override exists to settle.
    PRIMARY KEY (org_id, inventory, axis, from_term),

    -- The spelling is pinned for the same reason election_rule pins its own:
    -- the licence CHECK below states a rule over a set of axis values, and a
    -- set is only trustworthy if the values in it are.
    CONSTRAINT projection_override_axis CHECK (
        axis IN ('subject', 'topic', 'resource_type', 'phase', 'licence')
    ),
    -- 'narrower' is absent rather than forgotten: a narrower edge never
    -- participates in an outbound projection, so an override that produced one
    -- would be a decision the seller could make and never observe.
    CONSTRAINT projection_override_kind CHECK (kind IN ('exact', 'broader')),
    -- Delegation::Never wins over any opt-in, and an override is an opt-in of
    -- the strongest kind. Issuing a rights grant on the seller's behalf is not
    -- a preference we can be given, so licence is refused here as it is
    -- refused in election_rule -- in both layers, because the domain check
    -- passes for anything that writes the row directly.
    CONSTRAINT projection_override_licence_never_overridden CHECK (axis <> 'licence'),
    -- A path with no segments names nothing, and the projection would resolve
    -- to a value the target cannot be told.
    CONSTRAINT projection_override_segments CHECK (cardinality(to_segments) > 0),
    CONSTRAINT projection_override_decider_total CHECK (
        (decided_by = 'imported' AND decided_source IS NOT NULL
            AND decided_user IS NULL AND decided_org IS NULL)
     OR (decided_by = 'human' AND decided_source IS NULL
            AND decided_user IS NOT NULL AND decided_org IS NOT NULL)
    )
);

ALTER TABLE projection_override ENABLE ROW LEVEL SECURITY;
ALTER TABLE projection_override FORCE ROW LEVEL SECURITY;
CREATE POLICY projection_override_org_isolation ON projection_override
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- The engine reads a seller's overrides while projecting their listings and
-- never authors one, which is the same asymmetry election_rule carries: the
-- decision is the seller's and the sync run only consults it.
GRANT SELECT ON projection_override TO tam_engine;
