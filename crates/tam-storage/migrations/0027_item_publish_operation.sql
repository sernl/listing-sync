-- A publish whose subject is not known when the seller asks for it.
--
-- Both adapters create a draft, so reaching live from nothing is genuinely two
-- writes: a create, then a publish of whatever the create bound. `Revise`
-- cannot serve the second. It requires a stated subject in both the type and
-- the CHECK below, and an unbound mapping has none -- so the pair failed at
-- enqueue with a constraint violation, which is a 500 for an ordinary seller
-- action. On a severed mapping the only available id is the dead listing's,
-- and once the create rebinds, the admission gate refuses it as diverged, so
-- a re-publish after a removal silently never happened.
--
-- The alternative was refusing publish-to-live-from-nothing at the API and
-- making the seller's one action two round trips for a new listing. The
-- variant costs this CHECK rewrite and four arms; the refusal costs the
-- headline flow.
--
-- The CHECK is rewritten rather than extended because its ELSE arm is false:
-- a new operation value is inadmissible until the CASE names it.

-- Two CHECKs govern the operation column, not one: this names the admissible
-- values and the one below states what each requires of the other columns.
-- Widening only the second admits a value the first still refuses, which
-- fails at enqueue as a constraint violation -- a 500 for an ordinary seller
-- action, which is the shape this whole variant exists to remove.
ALTER TABLE job_item DROP CONSTRAINT job_item_operation;
ALTER TABLE job_item ADD CONSTRAINT job_item_operation CHECK (
    operation IN ('create', 'publish', 'revise', 'remove')
);

ALTER TABLE job_item DROP CONSTRAINT job_item_operation_total;
ALTER TABLE job_item ADD CONSTRAINT job_item_operation_total CHECK (
    CASE operation
        WHEN 'create'  THEN subject_kind IS NULL
                        AND state_from IS NULL AND state_to IS NULL
        -- No subject and no from: both are resolved from the binding at lease
        -- time, which is the whole point of the variant. The `to` is the
        -- seller's stated intent and is the one thing they can state in
        -- advance.
        WHEN 'publish' THEN subject_kind IS NULL
                        AND state_from IS NULL AND state_to IS NOT NULL
        WHEN 'revise'  THEN subject_kind IS NOT NULL
                        AND state_from IS NOT NULL AND state_to IS NOT NULL
        WHEN 'remove'  THEN subject_kind IS NOT NULL
                        AND state_from IS NOT NULL AND state_to IS NULL
        ELSE false
    END
);
