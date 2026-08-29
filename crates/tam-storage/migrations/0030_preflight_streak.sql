-- How many indeterminate preflights this item has answered in a row.
--
-- A preflight that fails on anything but schema drift abandons the run and
-- leaves the lease to expire. That is the stall bias, and it is right for a
-- transient. It is wrong for a condition that holds every cycle: the Tes
-- preflight is write-bearing, so a session broker holding a stale secret
-- fails it identically forever, and `job_item_one_live_lease_per_org` keeps
-- every other item of that tenant waiting behind the abandoned lease for the
-- whole of its TTL. Recovery took a human re-linking the connection.
--
-- `attempt_count` cannot carry this. It counts leases rather than preflights,
-- only the two reapers advance it, and nothing resets it -- so it cannot
-- state "the last N preflights in a row failed", which is the predicate that
-- separates a sick connection from an unlucky moment. This column advances on
-- each indeterminate preflight and returns to zero on the first healthy one.
-- The driver's bound sits strictly below `job::ATTEMPTS_MAX` so the item
-- reaches the connection-health outcome before `expire_and_steal` settles it
-- `failed`/`Other`, which names nothing a seller can act on.

ALTER TABLE job_item ADD COLUMN preflight_failures int NOT NULL DEFAULT 0;
