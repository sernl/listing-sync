-- Per-listing marketplace analytics, appended to and never updated in place.
--
-- Keyed on the mapping rather than the product because a metric belongs to one
-- listing on one marketplace, and one product may carry several mappings. The
-- mapping already names its inventory, so no inventory column is needed.
--
-- A snapshot row rather than a latest-value row: the all-time read gives a
-- running total, and a rate over any period is the difference between two of
-- them, so a table that overwrote would destroy the only way to compute one.
--
-- total_value is the untyped double the adapter read. The wire number means a
-- count for sales, an amount for earnings and a ratio for the Easel rates, so
-- reinterpreting it here would decide per metric what the read deliberately
-- does not. Storing earnings as a bare float rather than as Money is the
-- deliberate exception that follows from it.
--
-- metric carries no CHECK: the three captured today are a shortlist off the
-- twelve the adapter names, and widening the shortlist must not need a
-- migration.

CREATE TABLE listing_metric_snapshot (
    org_id      uuid             NOT NULL REFERENCES organisation (id),
    mapping_id  uuid             NOT NULL,
    metric      text             NOT NULL,
    observed_at timestamptz      NOT NULL,
    total_value double precision NOT NULL,

    PRIMARY KEY (org_id, mapping_id, metric, observed_at),

    -- A snapshot describes a listing, so it cannot outlive the mapping that
    -- names one. CASCADE rather than RESTRICT because the mapping is the
    -- subject and these rows are what was observed about it: a delete that
    -- had to erase the observations first would make removing a mapping
    -- depend on how long it had been measured.
    FOREIGN KEY (org_id, mapping_id) REFERENCES mapping (org_id, id) ON DELETE CASCADE
);

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form: a pooled connection reverts a transaction-local
-- app.current_org to the empty string rather than to missing, so NULLIF is
-- what keeps an unpinned statement matching nothing instead of raising 22P02.
ALTER TABLE listing_metric_snapshot ENABLE ROW LEVEL SECURITY;
ALTER TABLE listing_metric_snapshot FORCE ROW LEVEL SECURITY;
CREATE POLICY listing_metric_snapshot_org_isolation ON listing_metric_snapshot
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- Retention is the one erasure here, and it is cross-tenant by construction
-- like the job-event pruner beside it: one pass covers every organisation, so
-- it runs on the BYPASSRLS engine role and sees nothing at all under tam_app's
-- forced row-level security. SELECT travels with DELETE because the pass picks
-- its batch before erasing it. No INSERT and no UPDATE: the engine never
-- captures a snapshot and never rewrites one.
GRANT SELECT, DELETE ON listing_metric_snapshot TO tam_engine;
