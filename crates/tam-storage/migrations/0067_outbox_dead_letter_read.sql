-- The operator's read of the outbox's dead letters.
--
-- A message that exhausts its attempt budget, or that the deliverer refuses
-- outright, is written `state = 'dead'`, and until now nothing read it: a
-- seller whose completion mail died learned nothing, and neither did anybody
-- operating the platform. The operator surface now counts dead letters by
-- topic and by organisation, which is the smallest reading that says whether
-- the problem is one tenant or the relay.
--
-- The grant is three columns and the policy is one state. The payload is a
-- run summary and the dedupe key names a job, neither of which a count needs,
-- and the pending rows are the live queue, which an operator has no reading
-- for. The policy, like migration 0060's, is where the narrowness lives: the
-- role sees a dead letter's organisation, topic and state and nothing else on
-- the table.
GRANT SELECT (org_id, topic, state) ON outbox_message TO tam_backoffice;

CREATE POLICY outbox_message_backoffice_dead_read ON outbox_message
    FOR SELECT TO tam_backoffice
    USING (state = 'dead');
