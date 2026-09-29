-- Monthly usage counters: the allowances a plan renews each month rather
-- than holds as a standing count (docs/notes/design/entitlement-enforcement.md).
--
-- A standing count (resources, templates, collections, labels, devices) is a
-- count(*) over the rows themselves, because deleting one gives the room
-- back. A monthly allowance is not: a watermarked preview made and then
-- deleted was still made, so deleting it must not hand the seller another.
-- That needs a counter that only goes up, keyed by the month it counts in.
--
-- The month is the UTC calendar month, stored as its first day. A new month
-- is a new row, so the reset is the absence of a row rather than a job that
-- has to run at midnight; old rows stay as the record of what was used.
--
-- The gate increments with a single conditional upsert,
--   INSERT ... ON CONFLICT DO UPDATE SET used = used + 1 WHERE used < $cap,
-- so two concurrent requests at the last free slot cannot both be granted
-- it: the row lock serialises them, and the loser's WHERE is false.

CREATE TABLE usage_counter (
    org_id     uuid        NOT NULL REFERENCES organisation (id),
    kind       text        NOT NULL,
    month      date        NOT NULL,
    used       integer     NOT NULL,
    updated_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, kind, month),

    CONSTRAINT usage_counter_kind_known CHECK (kind IN ('preview', 'ai_fill')),
    CONSTRAINT usage_counter_month_is_first_day CHECK (
        month = date_trunc('month', month)::date
    ),
    CONSTRAINT usage_counter_used_counts CHECK (used >= 0)
);

ALTER TABLE usage_counter ENABLE ROW LEVEL SECURITY;
ALTER TABLE usage_counter FORCE ROW LEVEL SECURITY;
CREATE POLICY usage_counter_org_isolation ON usage_counter
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No grant to tam_backoffice, for migration 0084's reason: the operator
-- surface reads usage through the application pool with the organisation
-- pinned, so this table adds no unfenced path across tenants.
