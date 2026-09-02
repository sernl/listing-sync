-- Two instants on every device-originated row: the seller's assertion and our
-- receipt.
--
-- After the two-branch split the interpreter runs on hardware we do not
-- operate, so `created_at` and `opened_at` stop being a clock we can vouch for
-- if a device supplies them. They stay ours — the server's receipt, written
-- from `now()` — and the device's own instant is recorded beside them as what
-- it is: an assertion, evidence rather than authority.
--
-- Null for every row a server process wrote, which is the honest state: there
-- was no second clock to record. `job_event.org_seq` continues to decide
-- order, so nothing downstream reads either column to sequence anything.
ALTER TABLE job_event    ADD COLUMN asserted_at timestamptz;
ALTER TABLE write_attempt ADD COLUMN asserted_at timestamptz;

COMMENT ON COLUMN job_event.asserted_at IS
    'The device''s own clock reading, where a device wrote this row. Evidence of what the seller''s machine believed, never our record of when it happened.';
COMMENT ON COLUMN write_attempt.asserted_at IS
    'The device''s own clock reading, where a device opened or settled this attempt.';
