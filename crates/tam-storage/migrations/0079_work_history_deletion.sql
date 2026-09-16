-- Deleting work history without destroying the receipts that decide whether a
-- write landed.
--
-- A seller wants an import, a migration or a publishing job out of their
-- history. Three things they never asked for and must not get: a marketplace
-- listing removed, a catalogue resource removed, or a job that keeps running
-- after the row it was reached through is gone. So Delete is not a DELETE.
-- It is a fence plus a tombstone:
--
--   * the fence stops future work -- no new lease is handed out, no requeue
--     revives a parked item, no late drain mints a leg, no page commits;
--   * the tombstone hides the row from the seller's own lists and details
--     once the fence is proved, at the SQL paging and counting boundary
--     rather than in a browser;
--   * every receipt stays exactly where it was. `write_attempt` rows, the
--     `request_idempotency_key` that makes a replay a replay, the
--     `import_run_start_key` binding and `sync_request_resource`
--     breadcrumbs are all evidence about writes that may have reached a
--     marketplace, and a row destroyed is a duplicate upload nobody can
--     rule out.
--
-- Three states, one vocabulary across the three tables, because the console
-- renders one sentence for all three and a second spelling is a state nobody
-- handles:
--
--   'stopping'     -- the fence is up and something is still executing. The
--                     row stays visible, because a job that can still write
--                     must not read as gone.
--   'needs_review' -- the fence is up and the ledger cannot say what the
--                     marketplace did with a write already issued. Visible
--                     for the same reason, and it is never promoted by a
--                     clock: only evidence closes it.
--   'deleted'      -- quiescent. Hidden from the seller; still readable by
--                     reconciliation, which is the whole point of not having
--                     issued a DELETE.
--
-- `deletion_requested_at` rather than `deleted_at`, and the pair is total:
-- the instant is what every fence predicate reads, so a row carrying a state
-- without one would be hidden while still claimable.

ALTER TABLE job
    -- When the seller asked. The one column every fence in the ledger reads,
    -- which is why it is stamped before any item is settled and never
    -- cleared.
    ADD COLUMN deletion_requested_at timestamptz,
    -- Who asked, in `actor_kind`'s own vocabulary and beside its id, so the
    -- receipt says which seller or which process stopped the work.
    ADD COLUMN deletion_actor_kind   text,
    ADD COLUMN deletion_actor_id     text,
    ADD COLUMN deletion_state        text,
    ADD CONSTRAINT job_deletion_state CHECK (
        deletion_state IS NULL
            OR deletion_state IN ('stopping', 'needs_review', 'deleted')
    ),
    ADD CONSTRAINT job_deletion_total CHECK (
        (deletion_requested_at IS NULL) = (deletion_state IS NULL)
    ),
    ADD CONSTRAINT job_deletion_actor_total CHECK (
        (deletion_requested_at IS NULL)
            = (deletion_actor_kind IS NULL AND deletion_actor_id IS NULL)
    );

ALTER TABLE sync_request
    ADD COLUMN deletion_requested_at timestamptz,
    ADD COLUMN deletion_actor_kind   text,
    ADD COLUMN deletion_actor_id     text,
    ADD COLUMN deletion_state        text,
    ADD CONSTRAINT sync_request_deletion_state CHECK (
        deletion_state IS NULL
            OR deletion_state IN ('stopping', 'needs_review', 'deleted')
    ),
    ADD CONSTRAINT sync_request_deletion_total CHECK (
        (deletion_requested_at IS NULL) = (deletion_state IS NULL)
    ),
    ADD CONSTRAINT sync_request_deletion_actor_total CHECK (
        (deletion_requested_at IS NULL)
            = (deletion_actor_kind IS NULL AND deletion_actor_id IS NULL)
    );

ALTER TABLE import_run
    ADD COLUMN deletion_requested_at timestamptz,
    ADD COLUMN deletion_actor_kind   text,
    ADD COLUMN deletion_actor_id     text,
    ADD COLUMN deletion_state        text,
    ADD CONSTRAINT import_run_deletion_state CHECK (
        deletion_state IS NULL
            OR deletion_state IN ('stopping', 'needs_review', 'deleted')
    ),
    ADD CONSTRAINT import_run_deletion_total CHECK (
        (deletion_requested_at IS NULL) = (deletion_state IS NULL)
    ),
    ADD CONSTRAINT import_run_deletion_actor_total CHECK (
        (deletion_requested_at IS NULL)
            = (deletion_actor_kind IS NULL AND deletion_actor_id IS NULL)
    );

-- The seller's own lists, which are every page and every count they read.
-- Partial rather than a plain column index: the predicate is
-- `deletion_state IS DISTINCT FROM 'deleted'` on every one of those reads, so
-- the index that serves them is the one that holds only the rows they can
-- return. The newest-first key matches `job`'s existing keyset order and
-- `import_run_by_age`.
CREATE INDEX job_visible_by_age ON job (org_id, created_at DESC, id)
    WHERE deletion_state IS DISTINCT FROM 'deleted';

CREATE INDEX import_run_visible_by_age ON import_run (org_id, created_at DESC, id)
    WHERE deletion_state IS DISTINCT FROM 'deleted';

CREATE INDEX sync_request_visible_by_age ON sync_request (org_id, requested_at DESC, id)
    WHERE deletion_state IS DISTINCT FROM 'deleted';

-- The reconciliation queue: the rows a deletion left open. Small by
-- construction and swept per tenant, so the index exists to make the sweep
-- free rather than to serve a seller.
CREATE INDEX job_deletion_unquiesced ON job (org_id, id)
    WHERE deletion_state IN ('stopping', 'needs_review');

CREATE INDEX import_run_deletion_unquiesced ON import_run (org_id, id)
    WHERE deletion_state IN ('stopping', 'needs_review');

CREATE INDEX sync_request_deletion_unquiesced ON sync_request (org_id, id)
    WHERE deletion_state IN ('stopping', 'needs_review');
