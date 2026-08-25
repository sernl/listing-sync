-- A pooled connection that ever ran a transaction-local
-- set_config('app.current_org', ...) reverts the setting to the EMPTY STRING
-- at commit, not to missing, so current_setting(..., true)::uuid raises
-- 22P02 on the next unpinned statement instead of matching nothing. NULLIF
-- restores the intended fail-closed behaviour: unset OR reset both read as
-- NULL, match nothing, refuse cleanly. Every future policy uses this form.
DO $$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY[
        'product', 'blob', 'product_file', 'product_term', 'grade_declaration',
        'grade_declaration_path', 'mapping', 'binding_candidate',
        'field_mismatch', 'job', 'job_item', 'job_event', 'org_event_counter',
        'write_attempt', 'outbox_message', 'org_halt', 'org_inventory_halt',
        'rate_budget', 'connection', 'connection_secret', 'field_audit'
    ] LOOP
        EXECUTE format(
            'ALTER POLICY %I ON %I '
            'USING (org_id = NULLIF(current_setting(''app.current_org'', true), '''')::uuid) '
            'WITH CHECK (org_id = NULLIF(current_setting(''app.current_org'', true), '''')::uuid)',
            t || '_org_isolation', t
        );
    END LOOP;
END
$$;
