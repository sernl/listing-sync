-- What an old import already did, written down where 0074 could not write it.
--
-- 0074 introduced `enumeration_complete`, `discovered`, `processed` and
-- `selected_total`, and ended with an `UPDATE import_run` meant to derive the
-- first three for the runs that already existed. That statement moved nothing,
-- anywhere. Migrations are applied as `tam_app` -- in production too, by
-- `nix/module.nix` -- and `tam_app` owns these tables under FORCE ROW LEVEL
-- SECURITY, while a migration runs with no tenant pinned. The
-- organisation-scoped policy therefore hid every row from the backfill, which
-- reported success over zero rows. `selected_total` had no backfill at all.
--
-- So an old run carries the column defaults rather than its own history: the
-- enumeration reads as open even though `read_total` says the list closed, the
-- counters read as zero even though the run holds described rows, and the
-- denominator is absent even where the seller ticked the list days ago.
--
-- The last of those is what strands the import. A closed list with no frozen
-- selection is, to every reader, the seller still choosing: the device's
-- `/import/open` answer says `listed, not selected`, `RunPhase::owed`
-- concludes nothing is owed and walks away, the console draws `selecting`, and
-- the maintenance sweep exempts the run from the lease rule for the same
-- reason. Meanwhile the run stays open and `import_run_one_open_per_source`
-- fences that shop against every import the seller starts next. The resources
-- they chose are never described, and the shop cannot be imported again.
--
-- This migration recovers the two layers in order: the facts 0074 meant to
-- derive, and then the selection those facts make readable.
--
-- The guard on both is `attempt = 0`, which is "no device has ever claimed
-- this run". It is what separates a row nothing has spoken for from live work.
-- `discovered` and `processed` are reports from a fenced owner and are not a
-- count of rows -- a device that has walked seven listings and posted two of
-- them is telling the truth about a shop the server cannot see -- and
-- `enumeration_complete` is set by the device and by nothing else. Those
-- writes all require a claim, so an unclaimed run cannot hold one, and a
-- claimed run is never touched here.
--
-- The restoration is monotone as well as guarded: a count is raised to what
-- the run's own rows show and never lowered, and the enumeration is closed but
-- never reopened. That is 0074's own rule -- an accepted count does not
-- regress -- applied to the pass that fills it in.
--
-- What counts as evidence that a selection was taken.
--
-- The tick list is not stored as a list. What it leaves behind is the state of
-- each row, and only some of those states can be reached by a selection:
--
--   `selected`             the selection's own write, still owed to a device;
--   `read`, `matched`,
--   `review`, `imported`   reachable only from `selected`, through a read;
--   `failed`               a failure recorded against work this run was doing,
--                          which for a marketplace run means a resource that
--                          had been chosen to be read or created;
--   `skipped` with a
--   stored `observed`      described first and then merged away at review, so
--                          it was read, so it was selected.
--
-- Everything else is either outside the selection or unrecoverable:
--
--   `listed`               never chosen -- and a taken selection leaves none,
--                          because it skips whatever it did not take;
--   `skipped` with no
--   `observed`             three different things wearing one shape. A listing
--                          the catalogue already held, skipped as the page
--                          landed and before the seller could tick it; a
--                          listing the seller declined, skipped as `not
--                          chosen`; and a resource a device could not read and
--                          skipped while describing. The first two were never
--                          selected. The third was. Nothing on the row
--                          distinguishes them: the reason is the device's own
--                          sentence, `observed_by_device` is not set by a
--                          skip, and the instants are the same. They are
--                          matched on neither, because a prose match would
--                          decide a seller's catalogue on a string a device
--                          composed.
--
-- The reconstructed total is therefore the recoverable floor of what was
-- chosen, not a reconstruction of the original number, and a run that lost a
-- resource to a device-side read skip carries a denominator one short of the
-- one the seller saw. That is the honest direction to be wrong in. Counting an
-- ambiguous skip would hand the reading stage resources nobody selected; being
-- short only makes a progress bar reach its end early on a run whose work is
-- already done. No old row is rewritten to make the number tidy, and the
-- original denominator is not guessed at from `read_total`, which counts the
-- shop rather than the choice.
--
-- What this migration deliberately does not do.
--
-- It never writes a denominator where there is no evidence of a choice: a run
-- whose rows are all `listed` is a seller mid-decision, and freezing a total
-- there would tell their device to go and describe a shop nobody asked for.
-- It touches open marketplace runs only -- a settled run is a record rather
-- than work, and is neither reopened nor re-narrated, and a spreadsheet run
-- has no tick list at all, so a parse still in flight must not have its
-- half-written row count frozen into a column that means "what the seller
-- chose". It authorises nothing: `commit_authorised_at` and
-- `commit_authorised_by` stay exactly as they are, so a recovered run still
-- has to be confirmed before anything is created. It claims no owner, raises
-- no fence, moves no run between states and rewrites no item.
--
-- It is idempotent by its guards rather than by luck: the counters are
-- monotone, the enumeration is one-way, and `selected_total IS NULL` keeps the
-- first number written, because once a number is frozen it is the run's and a
-- second pass over a run that has described more of its selection must not
-- shrink it.

-- 0. The fence has to come off for the length of this migration, for the same
--    reason 0074's backfill silently did nothing: the evidence scan would see
--    no rows and every UPDATE below would move none. The lift and its restore
--    are in one transaction with the writes, so no statement outside this
--    migration ever observes an unfenced table. The set is named once, in a
--    temporary table both halves read, so a table cannot be lifted and left
--    lifted. This is `0068_one_tes.sql`'s own pattern, for its own reason.
CREATE TEMP TABLE import_selection_unfenced (name text PRIMARY KEY) ON COMMIT DROP;
INSERT INTO import_selection_unfenced (name) VALUES
    ('import_run'), ('import_run_item');

DO $$
DECLARE
    unfenced record;
BEGIN
    FOR unfenced IN SELECT name FROM import_selection_unfenced LOOP
        EXECUTE format('ALTER TABLE %I NO FORCE ROW LEVEL SECURITY', unfenced.name);
    END LOOP;
END $$;

-- 1. The facts 0074 meant to derive, derived at last.
--
--    `read_total` is written by the list and by nothing else, so its presence
--    is exactly "the enumeration closed" under the old protocol -- 0074's own
--    sentence, and the reading is still true because a partial list only
--    became expressible with the bit this restores. The two counters are a
--    count of the run's rows.
--
--    Monotone in every column: `OR` only closes an enumeration, `GREATEST`
--    only raises a count. A run that somehow holds a number larger than its
--    rows keeps it.
UPDATE import_run run
   SET enumeration_complete = run.enumeration_complete OR (run.read_total IS NOT NULL),
       discovered = GREATEST(run.discovered, (
           SELECT count(*)::int FROM import_run_item item
            WHERE item.org_id = run.org_id AND item.run_id = run.id
       )),
       processed = GREATEST(run.processed, (
           SELECT count(*)::int FROM import_run_item item
            WHERE item.org_id = run.org_id AND item.run_id = run.id
              AND item.state IN ('read', 'matched', 'review', 'imported', 'skipped', 'failed')
       ))
 WHERE run.state = 'reading'
   AND run.kind = 'marketplace'
   AND run.attempt = 0;

-- 2. The selection, from the rows it left behind. One statement, because the
--    evidence and the count are the same set: a run reached by the join holds
--    at least one row a selection produced, and the number is how many of them
--    there are. A run with none is not joined and keeps its NULL, which is the
--    seller still choosing.
UPDATE import_run run
   SET selected_total = taken.chosen
  FROM (
      SELECT item.org_id, item.run_id, count(*)::int AS chosen
        FROM import_run_item item
       WHERE item.state IN ('selected', 'read', 'matched', 'review', 'imported', 'failed')
          OR (item.state = 'skipped' AND item.observed IS NOT NULL)
       GROUP BY item.org_id, item.run_id
  ) AS taken
 WHERE taken.org_id = run.org_id
   AND taken.run_id = run.id
   -- Never overwritten: a frozen denominator is the run's own, and this
   -- migration exists for the rows that have none.
   AND run.selected_total IS NULL
   -- Open and still reading, which is where the defect bites and the only
   -- state whose work a device can still resume.
   AND run.state = 'reading'
   AND run.kind = 'marketplace'
   -- Unclaimed, as step 1 is. A run under a live fence is its owner's, and a
   -- denominator arriving underneath a working device is exactly the moving
   -- number `selected_total` exists to prevent.
   AND run.attempt = 0
   -- The selection cannot have been taken before the shop was walked, so a
   -- run still enumerating has nothing to reconstruct and a partial list to
   -- be measured against if one were invented. Step 1 is what makes this
   -- readable on an old run.
   AND run.enumeration_complete;

-- 3. Re-fence, from the same list. `ALTER TABLE` refuses a table with pending
--    trigger events, so this transaction's deferred constraints are fired
--    first -- which is also where a write that broke an invariant would
--    surface.
SET CONSTRAINTS ALL IMMEDIATE;

DO $$
DECLARE
    unfenced record;
BEGIN
    FOR unfenced IN SELECT name FROM import_selection_unfenced LOOP
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', unfenced.name);
    END LOOP;
END $$;
