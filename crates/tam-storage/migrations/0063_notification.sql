-- What a seller is told when one of their runs finishes, and whether they
-- want it by mail as well as in the console.
--
-- The row is written in the transaction that settles the run's last item,
-- because that is the transaction that owns the fact. The outbox row beside it
-- is only the email channel: the inbox fills whether or not the seller takes
-- mail, and whether or not a relay is configured at all.
--
-- The completion unit is the run rather than the job. A migration owns two
-- jobs -- `sync_request.create_job_id` and `remove_job_id`, which are separate
-- rows in `job` because `job.inventory` is single-valued -- so keying on the
-- job would tell a seller twice about one migration. The unique index below is
-- therefore the arbiter of "one notification per run": a race between the two
-- legs writes one row whichever leg wins.

CREATE TABLE notification (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    id          uuid        NOT NULL,
    kind        text        NOT NULL,
    -- The sync_request or the import_batch, never the job: it is what the
    -- console's row and the mail's button open, and a job has no page of its
    -- own that states what the seller asked for.
    subject_id  uuid        NOT NULL,
    -- The inventory written to, which for a migration is the target rather
    -- than the source. Null for an import, which commits to the catalogue and
    -- writes to no marketplace.
    inventory   text,
    marketplace text,
    -- The settled outcomes as `JobEventPayload::JobSettled` counts them:
    -- succeeded, degraded, failed, ambiguous, skipped, blocked. Composed here
    -- rather than derived at read time, so the row states what was true when
    -- the run finished.
    counts      jsonb       NOT NULL,
    created_at  timestamptz NOT NULL,
    read_at     timestamptz,

    PRIMARY KEY (org_id, id),

    CONSTRAINT notification_kind CHECK (kind IN ('sync', 'migration', 'import')),
    CONSTRAINT notification_marketplace_total CHECK (
        (inventory IS NULL) = (marketplace IS NULL)
    ),
    CONSTRAINT notification_import_names_no_marketplace CHECK (
        (kind = 'import') = (inventory IS NULL)
    ),
    FOREIGN KEY (inventory, marketplace)
        REFERENCES marketplace_inventory (code, marketplace)
);

-- One row per run, as a database property rather than a code convention.
CREATE UNIQUE INDEX notification_one_per_run
    ON notification (org_id, kind, subject_id);

-- The list route's keyset, newest first, which is the order the cursor encodes.
CREATE INDEX notification_newest_first
    ON notification (org_id, created_at DESC, id DESC);

ALTER TABLE notification ENABLE ROW LEVEL SECURITY;
ALTER TABLE notification FORCE ROW LEVEL SECURITY;
CREATE POLICY notification_org_isolation ON notification
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- Both settle paths reach `settle_if_complete`, and they arrive as different
-- roles: the in-process worker as tam_engine, and a seller device's own settle
-- as tam_app under its org pin. So the engine needs INSERT.
--
-- INSERT alone, and the two verbs left out are left out deliberately. Nothing
-- on an engine path reads this table: `record` is an INSERT ... ON CONFLICT DO
-- NOTHING with no RETURNING, which Postgres serves on the insert privilege by
-- itself. Nothing on an engine path marks a row read either: the only caller
-- of `mark_read_through` is the API route, on the app pool. A cross-tenant role
-- holding UPDATE here could mark every organisation's inbox read and take from
-- a seller the record that they had not yet seen a run.
GRANT INSERT ON notification TO tam_engine;

-- Whether this seller wants the mail. Per user rather than per organisation,
-- because the address is the user's; with one user per organisation today the
-- two are indistinguishable in behaviour, and an invite is the moment the
-- question would otherwise be answered by accident.
ALTER TABLE app_user ADD COLUMN notify_email boolean NOT NULL DEFAULT true;

-- The narrowest grant that lets the outbox drainer find who to mail. Column
-- scoped rather than table wide, and it is the first privilege the engine role
-- holds on a user table at all: `email` is deliberately absent, because for a
-- self-serve signup it holds `{subject}@subject.invalid` and the real address
-- is the identity service's. `auth_subject` is what the drainer resolves
-- against that service for the length of one send.
GRANT SELECT (id, org_id, auth_subject, notify_email) ON app_user TO tam_engine;

-- The engine settles the last item of a run and must resolve which run that
-- was, so it reads the request the job belongs to. The role is BYPASSRLS, which
-- bypasses the policy and not the table privilege, so without this line every
-- settle reached from the maintenance loop fails with permission denied.
--
-- Column scoped to exactly the six columns `run_of` names, using the same
-- technique this migration applies to `app_user` eight lines above. Table-wide
-- would additionally hand a cross-tenant role every organisation's `source`,
-- `intent`, `state`, `requested_at`, `settled_at` and `failure_detail` -- the
-- last being free text a marketplace's own error message lands in. What the
-- engine learns under this form is the disposition, the target inventory and
-- the two job ids, and it already learns the target inventory from `job`.
GRANT SELECT (org_id, id, disposition, target, create_job_id, remove_job_id)
    ON sync_request TO tam_engine;

-- The request a job belongs to, written in the transaction that mints the job.
--
-- The other direction already exists -- `sync_request.create_job_id` and
-- `remove_job_id` -- and it is written by a later statement than the one that
-- mints the job, in the cron drain's own third transaction. A settle landing
-- between the two, or after that third transaction failed outright, finds no
-- request through those columns and cannot tell a run's completion from a bare
-- job's: for a plain sync, which has no second leg to correct it, that is a
-- finished run the seller is never told about.
--
-- So the job names its request as it is created, and the settle reads that
-- direction. One column, immutable once set, and the request keeps naming its
-- jobs for the reads that walk the other way.
ALTER TABLE job ADD COLUMN sync_request_id uuid;

CREATE INDEX job_by_sync_request
    ON job (org_id, sync_request_id)
    WHERE sync_request_id IS NOT NULL;

-- Whether an imported row was already in the catalogue when the commit reached
-- it.
--
-- The commit's resume creates nothing for a row whose reserved product already
-- exists, and records it `created` exactly as it records one it did create --
-- so the batch's own counts cannot tell the two apart, and a seller who
-- re-commits a sheet of forty rows that all already exist is told forty
-- succeeded. The console's vocabulary has a word for this and the mail has a
-- row for it; this is the column that lets either say it.
ALTER TABLE import_batch_row ADD COLUMN skipped boolean NOT NULL DEFAULT false;
