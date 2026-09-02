-- The live-lease mutex, re-scoped from the organisation to the connection by
-- founder ruling on 2026-09-03 (engine-driver-split.md, open question 2).
--
-- The invariant it protects is one live session per marketplace account, which
-- is a per-connection property. The per-tenant scope was right only while there
-- was exactly one claimant process; D14 grants a seller several devices, and
-- under the old index the second one lost at the index and could not tell an
-- empty queue from a queue held by its sibling.
--
-- A connection is keyed (org_id, marketplace), so this index is too. That is
-- deliberate and it is the invariant, not an approximation: tes_gb and tes_us
-- share one Tes login, so they share one slot.

-- `job_item` carries no marketplace of its own and a partial unique index
-- cannot join, so the column is denormalised from the mapping that owns it.
ALTER TABLE job_item ADD COLUMN marketplace text;

UPDATE job_item ji
   SET marketplace = m.marketplace
  FROM mapping m
 WHERE m.org_id = ji.org_id AND m.id = ji.mapping_id;

ALTER TABLE job_item ALTER COLUMN marketplace SET NOT NULL;

-- The enqueue writes the column, and this holds it to the mapping it was
-- copied from: a drifted value would put the mutex on the wrong account, which
-- is a correctness hazard rather than an untidiness, so it is enforced by
-- structure. The unique constraint below exists only because a foreign key
-- must reference one; (org_id, id) is already the primary key, so it adds no
-- uniqueness, only the index the reference needs.
ALTER TABLE mapping ADD CONSTRAINT mapping_identity_with_marketplace
    UNIQUE (org_id, id, marketplace);

ALTER TABLE job_item ADD CONSTRAINT job_item_marketplace_matches_mapping
    FOREIGN KEY (org_id, mapping_id, marketplace)
    REFERENCES mapping (org_id, id, marketplace);

DROP INDEX job_item_one_live_lease_per_org;

CREATE UNIQUE INDEX job_item_one_live_lease_per_connection
    ON job_item (org_id, marketplace)
    WHERE state IN ('leased', 'running', 'verifying');
