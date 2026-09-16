-- The provenance a deletion fence needs, for the two workflows whose work is
-- minted somewhere other than the row the seller deletes.
--
-- 0079 gave each of the three user-facing tables a fence and a tombstone. Two
-- holes were left in it, and both are the same shape: work created *on behalf
-- of* a deleted record, through a row that carries no link back to it.
--
--   * A scheduled import publishes what it imported by minting publishing
--     jobs. Those jobs named no import, so deleting the import fenced its
--     itemless anchor and left its publication work claimable -- new
--     marketplace writes after DELETE had answered.
--   * A legacy sync page's per-resource apply spans several commits. Between
--     them the request's own ledger row can be quiet, so a DELETE arriving
--     mid-apply saw nothing executing and reported `deleted` over a resource
--     that was still being written.
--
-- Both are fixed by writing down the fact the fence has to read, rather than
-- by inferring it.

ALTER TABLE job
    -- The import this job publishes the results of, for a job minted by a
    -- scheduled run's publication pass. NULL on every job a seller, a
    -- migration or an ordinary schedule send minted, which is nearly all of
    -- them.
    --
    -- Distinct from `job_origin.run`, which is the *sync request* a job is a
    -- leg of: one is "which request asked for this", the other "which import
    -- produced the resources this sends". A job can have either, both or
    -- neither, and collapsing them would fence the wrong workflow's work.
    --
    -- Tenant-qualified foreign key, as every cross-table reference in this
    -- schema is: `(org_id, id)` is `import_run`'s own key, so a job cannot
    -- name another organisation's import. `ON DELETE SET NULL` is a
    -- formality -- a deleted import is tombstoned, never DELETEd -- and is
    -- stated so that a future hard-erasure of a tenant cannot be blocked by
    -- provenance.
    ADD COLUMN import_run_id uuid,
    ADD CONSTRAINT job_import_run_fk
        FOREIGN KEY (org_id, import_run_id) REFERENCES import_run (org_id, id)
        ON DELETE SET NULL (import_run_id);

-- The enumeration an import's deletion performs: every job that import's
-- publication minted, fenced beside its anchor in the same transaction.
-- Partial, because the column is NULL on almost every job and the deletion
-- path is the only reader.
CREATE INDEX job_by_import_run ON job (org_id, import_run_id)
    WHERE import_run_id IS NOT NULL;

ALTER TABLE sync_request_resource
    -- When this resource's apply was admitted: the instant a legacy
    -- migration page began the multi-commit write for it, cleared when that
    -- resource reaches `canonicalised` or `failed`.
    --
    -- The outstanding-execution marker a request's deletion classifies
    -- against, and its own column rather than a reading of `state`: 0025
    -- already writes `pending` at request creation, so `pending` alone says
    -- "not canonicalised yet" and says nothing about whether anything is
    -- executing. An apply that spans commits is otherwise invisible between
    -- them -- no claimed item, no live lease -- and a fence that read only
    -- the ledger reported a resource still being written as quiescent.
    -- Nullable and unconstrained: it is evidence, and the states it feeds
    -- are decided in the repository beside the ledger's own.
    ADD COLUMN admitted_at timestamptz;
