-- One JobSettled per job, structurally.
--
-- `settle_if_complete` decided completeness with an unlocked count under READ
-- COMMITTED, so two transactions settling a job's last two items each took a
-- snapshot in which the other's item was still unsettled: both returned early,
-- neither emitted the event, and the job had every item settled with no
-- JobSettled in the ledger and no `email.job_settled` for the seller. The
-- conditional insert guarded the duplicate direction alone. Two worker
-- processes per tenant is the designed configuration, and `revive_expired`
-- settles parked items from the maintenance loop while the pump settles a
-- leased one, which `job_item_one_live_lease_per_org` does not serialise --
-- it covers the live states a parked row does not have.
--
-- The fix is the per-organisation event lock, taken before the count rather
-- than after it. This index is the structural backstop the code comment there
-- said was missing: with it, a miss is still possible only if the lock is
-- removed, and a duplicate is impossible however the caller races.
CREATE UNIQUE INDEX job_event_one_settled_per_job
    ON job_event (org_id, job_id)
    WHERE kind = 'JobSettled';
