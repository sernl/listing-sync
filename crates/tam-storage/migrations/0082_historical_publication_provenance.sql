-- The publication provenance an upgrade has to supply, which is the half of
-- 0080 a new column could not reach.
--
-- 0080 added `job.import_run_id` and made every publishing job a scheduled
-- run mints from then on name the import whose resources it sends. The column
-- is NULL on every job minted before it, and the deletion fence enumerates
-- derived work through that column alone (`fence_owned_jobs`). So a database
-- upgraded with a queued auto-publication in it answers a seller's Delete by
-- fencing the itemless anchor, reporting `deleted`, and leaving the
-- publication claimable -- a marketplace write for an import they were told
-- is gone.
--
-- The ownership is not lost, only unread. `auto_publish_run` has recorded
-- `(org_id, run_id, product_id, target_inventory, job_id)` since 0071:225,
-- written by `SyncSettingRepo::record_auto_publish` after each job is minted,
-- and retained through a deletion like every other receipt. This migration
-- copies what those receipts already say into the column the fence reads, so
-- a historical publication is fenced by exactly the same enumeration as a new
-- one. Nothing is inferred: a job with no receipt keeps its NULL.
--
-- One receipt per resource per target, many per run, so several receipts name
-- one job. They are deduplicated by grouping on the job and reducing the run
-- ids they carry. Where that reduction is not a single run, or where the job
-- already names a different import, the migration refuses: two receipts
-- disagreeing about which import produced a job is a fact about the data this
-- pass cannot repair, and picking a row would fence one seller's import
-- against another's publication. An upgrade that stops with the offending
-- jobs named is recoverable; a wrong fence silently written is not.
--
-- Run per organisation rather than with the fence lifted. Migrations apply as
-- `tam_app`, which owns these tables under FORCE ROW LEVEL SECURITY with no
-- tenant pinned -- the reason 0074's backfill moved nothing anywhere and
-- 0076 had to lift the fence to repair it. Here the scan is tenant-local by
-- nature, so pinning each tenant in turn is both sufficient and stronger:
-- every statement below runs under the organisation's own policy, no table is
-- ever unfenced, and no statement reads two tenants' rows. The `org_id`
-- predicates are stated anyway, so the pass is scoped by the query and not
-- only by the policy, and the caller's `app.current_org` -- empty for a
-- migration, a tenant for a test harness that applies migrations on a pinned
-- connection -- is put back before the block returns.

DO $$
DECLARE
    -- What the connection was pinned to on the way in. `set_config` below is
    -- transaction-local and this whole block is one statement, so the
    -- restore is belt beside braces: a caller that applies migrations on an
    -- already-pinned connection must not find its tenant swapped for the
    -- last organisation in the roster.
    caller text := current_setting('app.current_org', true);
    tenant uuid;
    contradicted text;
BEGIN
    FOR tenant IN SELECT id FROM organisation ORDER BY id LOOP
        PERFORM set_config('app.current_org', tenant::text, true);

        -- 1. Refuse rather than choose. Two shapes, and both mean the same
        --    thing: this pass cannot say which import owns the job.
        --
        --      * receipts for one job naming more than one run;
        --      * a job already naming an import other than the one its
        --        receipts do, which no code path writes and a hand-repaired
        --        database might.
        --
        --    The reduction is `array_agg(DISTINCT ...)` rather than a `min`:
        --    PostgreSQL 17, which this deployment pins, carries no min/max
        --    aggregate for `uuid`. The array is both the deduplicated answer
        --    and the evidence of how many runs the receipts name.
        SELECT string_agg(owned.job_id::text, ', ' ORDER BY owned.job_id)
          INTO contradicted
          FROM (
              SELECT receipt.job_id,
                     array_agg(DISTINCT receipt.run_id) AS runs
                FROM auto_publish_run receipt
               WHERE receipt.org_id = tenant
                 -- A receipt that names no job names no work to fence. The
                 -- column is NOT NULL today; the predicate keeps this pass
                 -- honest rather than merely lucky if that is ever relaxed.
                 AND receipt.job_id IS NOT NULL
               GROUP BY receipt.job_id
          ) AS owned
          JOIN job ON job.org_id = tenant AND job.id = owned.job_id
         WHERE array_length(owned.runs, 1) > 1
            OR (job.import_run_id IS NOT NULL AND job.import_run_id <> owned.runs[1]);

        IF contradicted IS NOT NULL THEN
            RAISE EXCEPTION
                'auto_publish_run disagrees about which import produced job(s) % in organisation %',
                contradicted, tenant
                USING HINT = 'reconcile the receipts, or the job''s existing import_run_id, '
                             || 'before upgrading: a publication fenced against the wrong '
                             || 'import is a marketplace write nobody can account for';
        END IF;

        -- 2. The backfill, from the reduced receipts. `import_run_id IS NULL`
        --    is what makes a second run of this migration -- or a run over a
        --    database where 0080's own path already wrote the column -- move
        --    nothing: a provenance already written is the job's own, and the
        --    step above has already proved it agrees with the receipts.
        --
        --    A receipt naming a job that no longer exists joins to nothing
        --    and moves nothing, which is the right answer: there is no work
        --    left to fence.
        UPDATE job
           SET import_run_id = owned.runs[1]
          FROM (
              SELECT receipt.job_id,
                     array_agg(DISTINCT receipt.run_id) AS runs
                FROM auto_publish_run receipt
               WHERE receipt.org_id = tenant
                 AND receipt.job_id IS NOT NULL
               GROUP BY receipt.job_id
              HAVING count(DISTINCT receipt.run_id) = 1
          ) AS owned
         WHERE job.org_id = tenant
           AND job.id = owned.job_id
           AND job.import_run_id IS NULL;
    END LOOP;

    PERFORM set_config('app.current_org', coalesce(caller, ''), true);
END $$;
