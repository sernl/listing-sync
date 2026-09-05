-- The operator's read of the import-drain measurement.
--
-- The readout this feeds used to sit on the seller's reconciliation page. It
-- measures whether a tenant's crosswalk is converging, which is an engineering
-- question a teacher has no action to take on, so by founder decision it moves
-- to the operator pages -- and an operator surface reads across tenants, which
-- this role could not do here at all until now.
--
-- The grant below is the ninth entry in migration 0037's list and the only one
-- whose policy is not `USING (true)`. The eight there open a whole table to the
-- operator role. This one opens exactly the rows recording a drain measurement
-- and leaves the rest of the ledger -- every job, item, settlement and failure
-- event a tenant has written -- unreadable to it. A reviewer scanning eight
-- identical policies should stop at this one, which is why it is spelled out
-- here rather than appended to that file's list.
--
-- The restriction lives in the policy rather than in the view, and that is
-- forced rather than chosen. `job_event` carries FORCE ROW LEVEL SECURITY
-- (migration 0005), so a view executing as its owner is filtered by the tenant
-- policy against an unpinned `app.current_org` and returns zero rows -- not an
-- error, which is the dangerous half, since the page would render its empty
-- state forever and say nothing was wrong. `security_invoker` puts the reading
-- role's own policy in play instead, and the policy below is that policy.
GRANT SELECT ON job_event TO tam_backoffice;

CREATE POLICY job_event_backoffice_drain_read ON job_event
    FOR SELECT TO tam_backoffice
    USING (kind = 'ImportDrainMeasured');

-- The projection the operator route reads. The payload travels verbatim
-- because it is `jsonb` at rest and nothing constrains its shape there; the
-- client narrows it and drops a row that is not a measurement, so one
-- malformed historical row cannot empty the page.
CREATE VIEW import_drain_measurement WITH (security_invoker = true) AS
    SELECT org_id, org_seq, payload
    FROM job_event
    WHERE kind = 'ImportDrainMeasured';

GRANT SELECT ON import_drain_measurement TO tam_backoffice;
