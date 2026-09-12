-- Publishing on a timetable, and pulling a shop back on one.
--
-- Phase 4 of `docs/notes/design/2026-09-12-one-marketplace-per-site-and-the-seller-workflows.md`
-- section 5. Two clocks, and they are not the same clock. A schedule is the
-- seller's outbound one -- "send these resources to TPT at nine on Friday" --
-- and a sync setting is the inbound one: "re-read my shop every six hours".
-- Both are read by one pass in the serving process, and both are stored here
-- because a cadence held in a browser is a cadence that stops when the tab
-- closes, which is precisely what the two preview pages said and what this
-- migration falsifies.
--
-- Nothing here is a `sync_request`. A schedule materialises into ordinary
-- jobs, through the same lowering `POST /{version}/jobs` runs, because a
-- schedule is a seller pressing Publish on a timer and not a second kind of
-- work. `sync_request` addresses listings on a shop we cannot read; a
-- schedule addresses resources the catalogue already holds, so there is
-- nothing to canonicalise and no drain to wait for.
--
-- No engine grants. The pass runs inside `tam-server` as `tam_app` with one
-- tenant pinned per organisation, exactly as 0025's closing comment says the
-- drain does, so the cross-tenant role never sees a schedule at all.

CREATE TABLE schedule (
    org_id              uuid        NOT NULL REFERENCES organisation (id),
    id                  uuid        NOT NULL,
    -- The seller's own name for it, which is what the activity log quotes
    -- back at them.
    name                text        NOT NULL,
    -- Which way the members are named. Closed rather than derived from which
    -- column is filled: a label schedule re-resolves its members at every
    -- tick and a product schedule is frozen at the list the seller ticked,
    -- and reading those two as one would silently change what a tick sends.
    -- Collections are phase 5, so this is a two-value check that grows.
    selection_kind      text        NOT NULL,
    -- The label, for a label selection. The tick list lives in
    -- `schedule_product`.
    selection_label     text,
    -- Where the listing ends up, in `sync_request.intent`'s own spelling.
    intent              text        NOT NULL,
    -- Minutes past local midnight, which is what a time control produces.
    -- Stored as the seller's own wall time plus the zone it is read in rather
    -- than as an instant, because "nine in the morning" is what they chose:
    -- an instant frozen at today's offset drifts an hour at every clock
    -- change, and the seller who picked nine did not pick eight.
    at_minute_of_day    int         NOT NULL,
    -- An IANA zone name, validated by the route against the compiled-in
    -- database rather than by a check constraint: the set is thirty years of
    -- politics and does not belong in a schema.
    timezone            text        NOT NULL,
    -- How often. `once` fires at its next due minute and never again, which
    -- `last_run_at` is what records.
    repeat              text        NOT NULL,
    -- Sunday is 0, as `chrono::Weekday::num_days_from_sunday` and every date
    -- control agree. Only a weekly schedule has one.
    weekday             smallint,
    -- Whether a member already listed on a target is revised when its
    -- resource has changed since the listing was last written. Off by
    -- default: a schedule the seller set up to publish new work must not
    -- start rewriting their live listings because they fixed a typo.
    republish_on_update boolean     NOT NULL,
    enabled             boolean     NOT NULL,
    created_at          timestamptz NOT NULL,
    -- The tick this schedule was last materialised at, and the whole of the
    -- "has it run yet" question: the pass computes the latest due tick and
    -- compares. A column rather than a derived MAX over `schedule_run`,
    -- because a tick that resolved no members writes no run row and would
    -- otherwise be recomputed as due for ever.
    last_run_at         timestamptz,

    PRIMARY KEY (org_id, id),

    CONSTRAINT schedule_name_bounded CHECK (char_length(name) BETWEEN 1 AND 120),
    CONSTRAINT schedule_selection_kind CHECK (selection_kind IN ('label', 'products')),
    -- A label selection names a label; a product selection names none.
    CONSTRAINT schedule_selection_label_follows_kind CHECK (
        (selection_kind = 'label') = (selection_label IS NOT NULL)
    ),
    CONSTRAINT schedule_selection_label_bounded CHECK (
        selection_label IS NULL OR char_length(selection_label) BETWEEN 1 AND 120
    ),
    CONSTRAINT schedule_intent CHECK (intent IN ('draft', 'live')),
    CONSTRAINT schedule_minute_of_day CHECK (at_minute_of_day BETWEEN 0 AND 1439),
    CONSTRAINT schedule_timezone_bounded CHECK (char_length(timezone) BETWEEN 1 AND 64),
    CONSTRAINT schedule_repeat CHECK (repeat IN ('once', 'daily', 'weekly')),
    -- A weekly schedule names a day and nothing else does, which is the
    -- totality rule 0025 states for its settle columns.
    CONSTRAINT schedule_weekday_follows_repeat CHECK (
        (repeat = 'weekly') = (weekday IS NOT NULL)
    ),
    CONSTRAINT schedule_weekday_range CHECK (weekday IS NULL OR weekday BETWEEN 0 AND 6)
);

-- One name per organisation, compared case-insensitively as `label_one_per_name`
-- is: two schedules a seller cannot tell apart in a list are two they cannot
-- choose between.
CREATE UNIQUE INDEX schedule_one_per_name ON schedule (org_id, lower(name));

-- Which marketplaces one tick sends to. A table rather than an array for the
-- reason every other set here is one: the pass joins it, and a tick mints one
-- job per inventory.
CREATE TABLE schedule_marketplace (
    org_id      uuid NOT NULL REFERENCES organisation (id),
    schedule_id uuid NOT NULL,
    -- Unconstrained by a foreign key for `sync_request.source`'s reason: the
    -- repository's own round trip through InventoryId is what admits a value.
    inventory   text NOT NULL,

    PRIMARY KEY (org_id, schedule_id, inventory),
    FOREIGN KEY (org_id, schedule_id) REFERENCES schedule (org_id, id) ON DELETE CASCADE
);

-- The frozen tick list, for a product selection.
--
-- `ON DELETE CASCADE` on both parents: a schedule that is deleted takes its
-- list with it, and a resource the seller deleted from the catalogue leaves
-- the schedule rather than making every later tick refuse.
CREATE TABLE schedule_product (
    org_id      uuid NOT NULL REFERENCES organisation (id),
    schedule_id uuid NOT NULL,
    product_id  uuid NOT NULL,

    PRIMARY KEY (org_id, schedule_id, product_id),
    FOREIGN KEY (org_id, schedule_id) REFERENCES schedule (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id) ON DELETE CASCADE
);

-- What one tick did to one resource on one marketplace.
--
-- The primary key *is* the idempotency, which is the whole mechanism: the
-- pass inserts with `ON CONFLICT DO NOTHING`, so a second pass in the same
-- minute -- or a process that died between minting the job and marking the
-- schedule -- writes nothing and sends nothing twice. `schedule_run` is
-- therefore the record the runs list is drawn from and the fence that stops a
-- duplicate, in one row.
CREATE TABLE schedule_run (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    schedule_id uuid        NOT NULL,
    -- The scheduled instant this materialisation belongs to, not the instant
    -- the pass ran: a pass that runs forty seconds late must land on the same
    -- key as one that runs on time.
    tick        timestamptz NOT NULL,
    product_id  uuid        NOT NULL,
    inventory   text        NOT NULL,
    -- `sent` carries the job the member was minted into; `skipped` carries
    -- the sentence the lowering refused with. A refusal is recorded rather
    -- than raised because one uncapturable transition must not cost the
    -- seller the other nineteen resources of the tick.
    state       text        NOT NULL,
    job_id      uuid,
    reason      text,
    recorded_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, schedule_id, tick, product_id, inventory),
    FOREIGN KEY (org_id, schedule_id) REFERENCES schedule (org_id, id) ON DELETE CASCADE,

    CONSTRAINT schedule_run_state CHECK (state IN ('sent', 'skipped')),
    -- 0025's totality constraint, twice: a sent member names its job and a
    -- skipped one names its reason, and neither names the other's column.
    CONSTRAINT schedule_run_job_follows_state CHECK ((state = 'sent') = (job_id IS NOT NULL)),
    CONSTRAINT schedule_run_reason_follows_state CHECK (
        (state = 'skipped') = (reason IS NOT NULL)
    ),
    CONSTRAINT schedule_run_reason_bounded CHECK (
        reason IS NULL OR char_length(reason) <= 500
    )
);

-- Newest tick first, which is the runs list's own order.
CREATE INDEX schedule_run_by_tick ON schedule_run (org_id, schedule_id, tick DESC);

-- How often one shop is re-read, and whether it is re-read at all.
CREATE TABLE marketplace_sync_setting (
    org_id        uuid        NOT NULL REFERENCES organisation (id),
    inventory     text        NOT NULL,
    enabled       boolean     NOT NULL,
    -- The seller's choice, floored by the route at the plan's own minimum.
    -- Stored as their choice rather than as the floored value so a later
    -- upgrade widens what they already asked for instead of leaving them at
    -- the old plan's number.
    interval_secs int         NOT NULL,
    -- When the pass last minted a run for this shop. NULL is a setting that
    -- has never been due, which is not the epoch: the first enabled tick
    -- pulls immediately, and reading NULL as zero would have said the same
    -- thing by accident rather than by decision.
    last_pull_at  timestamptz,

    PRIMARY KEY (org_id, inventory),

    -- An hour is the shortest any plan offers and a fortnight the longest any
    -- control produces. Bounded in the schema because an interval of zero is
    -- a pass that mints a run every minute for ever.
    CONSTRAINT marketplace_sync_setting_interval CHECK (
        interval_secs BETWEEN 3600 AND 1209600
    )
);

-- Where a pulled resource is published once it lands.
--
-- One row per pair rather than a column on the setting above, because a
-- seller pulling from TPT may well publish to both of the others and the
-- answer is a set. The template a phase-5 rule will name is deliberately
-- absent: `resource_template.draft` is a partial catalogue form with no
-- marketplace scope, so a rule landing now publishes the catalogue projection
-- and the console says so.
CREATE TABLE auto_publish_rule (
    org_id           uuid NOT NULL REFERENCES organisation (id),
    source_inventory text NOT NULL,
    target_inventory text NOT NULL,

    PRIMARY KEY (org_id, source_inventory, target_inventory),

    CONSTRAINT auto_publish_rule_two_marketplaces CHECK (
        source_inventory <> target_inventory
    )
);

-- What one rule did to one pulled resource.
--
-- `schedule_run`'s shape and `schedule_run`'s reason: the primary key is the
-- idempotency the design states -- (run, product, target) -- so a pass that
-- commits a chunk, dies, and commits the rest does not publish the first
-- chunk twice. It is also the only thing that can say a job was minted by a
-- rule rather than by a seller, which is what the activity log's "published
-- to Tes" line is read from.
CREATE TABLE auto_publish_run (
    org_id           uuid        NOT NULL REFERENCES organisation (id),
    run_id           uuid        NOT NULL,
    product_id       uuid        NOT NULL,
    target_inventory text        NOT NULL,
    job_id           uuid        NOT NULL,
    recorded_at      timestamptz NOT NULL,

    PRIMARY KEY (org_id, run_id, product_id, target_inventory),
    FOREIGN KEY (org_id, run_id) REFERENCES import_run (org_id, id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id) ON DELETE CASCADE
);

CREATE INDEX auto_publish_run_by_age ON auto_publish_run (org_id, recorded_at DESC);

-- Whether the pass minted this run, rather than a seller pressing Import.
--
-- A column rather than a second `kind`, because everything else about the run
-- is identical: the same device reads the same shop into the same rows. What
-- differs is who finishes it -- a scheduled run selects every listed row
-- itself and is committed by the pass, and a seller's run waits for the
-- seller -- and that is one bit.
ALTER TABLE import_run ADD COLUMN scheduled boolean NOT NULL DEFAULT false;

ALTER TABLE schedule ENABLE ROW LEVEL SECURITY;
ALTER TABLE schedule FORCE ROW LEVEL SECURITY;
CREATE POLICY schedule_org_isolation ON schedule
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE schedule_marketplace ENABLE ROW LEVEL SECURITY;
ALTER TABLE schedule_marketplace FORCE ROW LEVEL SECURITY;
CREATE POLICY schedule_marketplace_org_isolation ON schedule_marketplace
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE schedule_product ENABLE ROW LEVEL SECURITY;
ALTER TABLE schedule_product FORCE ROW LEVEL SECURITY;
CREATE POLICY schedule_product_org_isolation ON schedule_product
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE schedule_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE schedule_run FORCE ROW LEVEL SECURITY;
CREATE POLICY schedule_run_org_isolation ON schedule_run
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE marketplace_sync_setting ENABLE ROW LEVEL SECURITY;
ALTER TABLE marketplace_sync_setting FORCE ROW LEVEL SECURITY;
CREATE POLICY marketplace_sync_setting_org_isolation ON marketplace_sync_setting
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE auto_publish_rule ENABLE ROW LEVEL SECURITY;
ALTER TABLE auto_publish_rule FORCE ROW LEVEL SECURITY;
CREATE POLICY auto_publish_rule_org_isolation ON auto_publish_rule
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE auto_publish_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE auto_publish_run FORCE ROW LEVEL SECURITY;
CREATE POLICY auto_publish_run_org_isolation ON auto_publish_run
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No backoffice grants. "Did their import finish" is a support question and
-- 0070 opens `import_run` for it; what a seller publishes, when, and to which
-- shop is their business, and a timetable is not a fault anyone is paged for.
