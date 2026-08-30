-- The backoffice role's reach, extended by one table: what Paddle has told us
-- about a tenant's subscription.
--
-- Both halves again, for the reason migration 0037 states: tam_backoffice is
-- not BYPASSRLS, so a grant alone shows it nothing on a fenced table and a
-- policy alone gives it nothing to read. A table needs its name in both lists
-- before one row of it is visible, and tests/backoffice_grants.rs probes both
-- rather than trusting this file.
GRANT SELECT ON billing_subscription TO tam_backoffice;

-- Permissive, so it is OR-ed with the tenant fence migration 0038 established
-- rather than replacing it: tam_app still sees only its pinned organisation.
--
-- FOR SELECT is load-bearing here for the reason it is there. A policy written
-- FOR ALL would extend to INSERT, UPDATE and DELETE the moment anybody granted
-- one, and the SELECT-only property would then rest on the grant list alone.
CREATE POLICY billing_subscription_backoffice_read ON billing_subscription
    FOR SELECT TO tam_backoffice USING (true);
