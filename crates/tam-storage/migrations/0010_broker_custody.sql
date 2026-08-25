-- Custody: the application role can no longer read the credential vault at
-- all, and the broker role is the only one that can. The design's rule —
-- "only the session broker's database role may select from connection_secret"
-- — as a grant, not a convention. The engine role was never granted
-- connection_secret (migration 0007 lists it nowhere), so only tam_app's
-- inherited PUBLIC access needs revoking.
REVOKE ALL ON connection_secret FROM tam_app;

GRANT SELECT, INSERT, UPDATE ON connection_secret TO tam_broker;
GRANT SELECT, UPDATE ON connection TO tam_broker;
GRANT SELECT ON organisation, marketplace_inventory TO tam_broker;
