-- A removal that must not run before its counterpart exists, and a publish
-- that must not run before the create it publishes has bound.
--
-- Stated as a predicate on the world rather than as a dependency on another
-- item. `Binding::Bound` is written only by the settle path from a landing
-- whose lifecycle came from the driver's own verification read, so "the
-- counterpart exists and we saw it" is exactly what this column asks. A
-- dependency on a specific item would strand the removal the first time a
-- failed create was re-run as a different job.
--
-- The unsafe direction of a migrate -- source gone, target absent -- is
-- unreachable because of this column. The safe failure is a duplicate.

ALTER TABLE job_item ADD COLUMN requires_bound_on text;

-- marketplace_inventory is keyed on (code, marketplace), so a single-column
-- reference needs a single-column unique index to point at. The code alone is
-- already unique in the seeded data; this states it so the foreign key below
-- can exist without the item having to carry a marketplace it never uses.
CREATE UNIQUE INDEX marketplace_inventory_code ON marketplace_inventory (code);

ALTER TABLE job_item ADD CONSTRAINT job_item_counterpart_inventory
    FOREIGN KEY (requires_bound_on) REFERENCES marketplace_inventory (code);

-- How many times this mapping's listing has been severed.
--
-- A migrate reversed is an ordinary sync in the other direction, and because
-- mapping_one_per_inventory is unpredicated it lands on the severed row
-- rather than minting a new one. But a re-create of the same product with the
-- same files into the same inventory produces a byte-identical idempotency
-- key: a create's intent digest is the payload digest with no job mixed in,
-- job_item_idempotent is table-wide with no job scoping, and job_item rows are
-- never deleted. So the second round trip is refused forever with a message
-- about unchanged content that is wrong for a listing which no longer exists.
--
-- Mixing the job id into the create arm would fix it and would delete the
-- property the content-addressed key was built for: that re-uploading
-- unchanged content is a no-op. A sever counter separates keys across a
-- sever and nowhere else, which is exactly the boundary that matters.
ALTER TABLE mapping ADD COLUMN sever_generation int NOT NULL DEFAULT 0;
