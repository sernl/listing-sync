-- What one item does to the listing its mapping names, and which listing it
-- said it was doing it to.
--
-- The transition is stored rather than derived because no column records
-- which side of the draft line a listing sits on: mapping.lifecycle_state is
-- written at insert and never again, and mapping.publish_mode is a sync
-- policy. The subject is stored as the caller's assertion; the mapping's
-- binding stays the authority, and the engine refuses an item whose stored
-- subject diverges from it.
--
-- No grant: 0007 grants SELECT, INSERT, UPDATE ON job_item to tam_engine
-- with no column list, and a table grant covers columns added later.

ALTER TABLE job_item ADD COLUMN operation          text NOT NULL DEFAULT 'create';
ALTER TABLE job_item ADD COLUMN subject_kind       text;
ALTER TABLE job_item ADD COLUMN subject_url        text;
ALTER TABLE job_item ADD COLUMN subject_numeric_id bigint;
ALTER TABLE job_item ADD COLUMN state_from         text;
ALTER TABLE job_item ADD COLUMN state_to           text;

-- The default backfills every existing row to 'create', which is what every
-- item in the ledger is: Effect::Submit is the only write effect today. It is
-- then dropped, so an insert site that forgets the column raises a NOT NULL
-- violation rather than silently storing 'create'.
ALTER TABLE job_item ALTER COLUMN operation DROP DEFAULT;

ALTER TABLE job_item ADD CONSTRAINT job_item_operation CHECK (
    operation IN ('create', 'revise', 'remove')
);

-- The subject's shape is the mapping's, so one codec serves both.
ALTER TABLE job_item ADD CONSTRAINT job_item_subject_shape CHECK (
    subject_kind IS NULL
 OR (subject_kind = 'tes'
     AND subject_url IS NOT NULL AND subject_numeric_id IS NULL)
 OR (subject_kind IN ('tpt', 'etsy')
     AND subject_numeric_id IS NOT NULL AND subject_url IS NULL)
);

ALTER TABLE job_item ADD CONSTRAINT job_item_lifecycle_states CHECK (
    (state_from IS NULL OR state_from IN ('draft', 'live'))
AND (state_to   IS NULL OR state_to   IN ('draft', 'live'))
);

-- A create names no listing and no transition; a revise names both ends; a
-- removal names the state whose route it will post the delete to and has no
-- landing state at all.
ALTER TABLE job_item ADD CONSTRAINT job_item_operation_total CHECK (
    CASE operation
        WHEN 'create' THEN subject_kind IS NULL
                       AND state_from IS NULL AND state_to IS NULL
        WHEN 'revise' THEN subject_kind IS NOT NULL
                       AND state_from IS NOT NULL AND state_to IS NOT NULL
        WHEN 'remove' THEN subject_kind IS NOT NULL
                       AND state_from IS NOT NULL AND state_to IS NULL
        ELSE false
    END
);
