-- The bind is folded into the attempt settle, so the engine records where its
-- own landed write lives: one UPDATE on mapping and nothing else. The rest of
-- 0007's enumeration stands — no INSERT, no DELETE, and nothing on
-- binding_candidate or field_mismatch — because resolving an ambiguous create
-- deletes candidate rows and a mismatched verification inserts mismatch rows,
-- so both of those paths stay on the app path with reconciliation.
GRANT UPDATE ON mapping TO tam_engine;

-- mapping_one_per_inventory protects the product side of a binding; nothing
-- protected the remote side, so two mappings in one org and inventory could
-- both claim the same remote listing. Partial because only a bound row holds
-- a claim, and split in two because mapping_remote_id_shape makes the url and
-- numeric columns exclusive per kind.
CREATE UNIQUE INDEX mapping_one_bound_url
    ON mapping (org_id, inventory, remote_url)
    WHERE binding_state = 'bound' AND remote_url IS NOT NULL;

CREATE UNIQUE INDEX mapping_one_bound_numeric_id
    ON mapping (org_id, inventory, remote_id_kind, remote_numeric_id)
    WHERE binding_state = 'bound' AND remote_numeric_id IS NOT NULL;
