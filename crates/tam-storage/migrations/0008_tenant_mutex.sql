-- The durable per-tenant mutex, as structure rather than convention: at most
-- one live-leased item per organisation, enforced the same way the
-- write-attempt fence is. Two workers racing different items of one tenant
-- both pass any read-time check under READ COMMITTED; the second one fails
-- here instead, which is the difference between a mutex and a hope.
-- UNIQUE (org_id, idempotency_key) on job_item is the backstop to this, not
-- a duplicate of it.
CREATE UNIQUE INDEX job_item_one_live_lease_per_org
    ON job_item (org_id)
    WHERE state IN ('leased', 'running', 'verifying');

-- Executing RequeueBehindGate flips the connection to needs_reauth, and the
-- lease scan refuses to lease where no linked connection exists, so the gate
-- fails closed for unlinked tenants too.
GRANT UPDATE ON connection TO tam_engine;
