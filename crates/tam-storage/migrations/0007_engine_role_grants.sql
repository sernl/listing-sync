-- Table privileges for the engine role (created cluster-wide by the init
-- script with BYPASSRLS; a migration cannot create it because migrations run
-- as the unprivileged application role). The engine crosses tenants by
-- design — the lease scan, the stealer and the breaker are inherently
-- cross-org — and everything it may do is enumerated here. No DELETE
-- anywhere: the engine stalls and settles, it never erases.
GRANT SELECT, INSERT, UPDATE ON job, job_item, job_event, org_event_counter,
    write_attempt, outbox_message, rate_budget TO tam_engine;
GRANT SELECT, INSERT ON org_halt, org_inventory_halt, inventory_halt,
    field_audit TO tam_engine;
GRANT SELECT ON organisation, marketplace_inventory, product, mapping,
    binding_candidate, field_mismatch, connection TO tam_engine;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA public TO tam_engine;
