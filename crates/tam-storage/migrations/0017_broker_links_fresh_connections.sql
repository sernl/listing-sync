-- The broker's link upserts the connection row before sealing the secret
-- against it. 0010 granted UPDATE only, written while the insert branch sat
-- unreachably after the secret insert, behind its foreign key.
GRANT INSERT ON connection TO tam_broker;
