-- Tes is one marketplace with no regions (decisions.md, 2026-09-12).
--
-- `tes_gb` becomes `tes` and `tes_us` and `tes_nz` are retired. The three
-- were a modelling choice taken before probe 04 and kept for currency, the
-- age field, the taxonomy prefix and a GB-to-NZ duplication, none of which is
-- a catalogue.
--
-- Tenant rows are rewritten in place. Where an organisation holds one product
-- mapped, elected, overridden, halted or reconciled on two Tes inventories
-- the migration REFUSES: that is a seller with two live Tes listings for one
-- product, which is a person to talk to rather than a row to drop. The
-- derived projection tables are deleted and re-seeded instead, because they
-- are a function of the captures and merging them is impossible anyway --
-- `projection_edge_single_valued` forbids two edges from one term into one
-- (inventory, kind), and every paired GB/NZ node is exactly that.

-- 0. This migration reads and rewrites every tenant's rows, and the
--    application role owns these tables under FORCE ROW LEVEL SECURITY, so
--    without lifting it the collision scan would see nothing and every
--    UPDATE below would move nothing. The lift and its restore are in one
--    transaction with the rewrite, so no statement outside this migration
--    ever observes an unfenced table. The set is named once, in a temporary
--    table both halves read, so a table cannot be lifted and left lifted.
--    `product_file`, `write_attempt` and `binding_candidate` are read rather
--    than rewritten: the deferred trigger `mapping_payload_nonempty` looks
--    for a live payload when a mapping row is updated, and step 2a looks for
--    attempts and candidates, and a fenced read answers nothing here too.
CREATE TEMP TABLE one_tes_unfenced (name text PRIMARY KEY) ON COMMIT DROP;
INSERT INTO one_tes_unfenced (name) VALUES
    ('mapping'), ('job'), ('job_item'), ('notification'), ('election_rule'),
    ('election_item'), ('projection_override'), ('import_batch_row'), ('sync_request'),
    ('grade_declaration'), ('grade_declaration_path'), ('native_residue'), ('product'),
    ('product_file'), ('write_attempt'), ('binding_candidate'),
    ('org_inventory_halt'), ('reconciliation_item');

DO $$
DECLARE
    unfenced record;
BEGIN
    FOR unfenced IN SELECT name FROM one_tes_unfenced LOOP
        EXECUTE format('ALTER TABLE %I NO FORCE ROW LEVEL SECURITY', unfenced.name);
    END LOOP;
END $$;

-- 1. Mint the survivor. `tes_gb` keeps its identity under the new code rather
--    than being merged into a fresh one, so the idempotency ordinal stays 0
--    and every historical key still derives.
INSERT INTO marketplace_inventory (code, marketplace, transport_class)
VALUES ('tes', 'tes', 'seller_device');

-- 2a. An unbound Tes mapping with nothing ever attempted is an intent tick
--     from the retired Curriculum control, not a listing: no remote id, no
--     write attempt, no job item, no candidate. Where a product carries such
--     ticks beside another Tes row, the ticks go and one row survives: a
--     bound one, else the GB one, else the oldest, because that is the row
--     that names a listing. Two BOUND Tes rows on one product are a seller to
--     speak to, and 2b refuses them.
WITH ranked AS (
    SELECT org_id, id,
           row_number() OVER (
               PARTITION BY org_id, product_id
               ORDER BY (binding_state = 'bound') DESC,
                        (inventory = 'tes_gb') DESC,
                        created_at, id) AS place
      FROM mapping
     WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz')
)
DELETE FROM mapping tick
 USING ranked
 WHERE tick.org_id = ranked.org_id AND tick.id = ranked.id AND ranked.place > 1
   AND tick.binding_state = 'unbound'
   AND tick.remote_url IS NULL AND tick.remote_numeric_id IS NULL
   AND NOT EXISTS (SELECT 1 FROM job_item ji
                    WHERE ji.org_id = tick.org_id AND ji.mapping_id = tick.id)
   AND NOT EXISTS (SELECT 1 FROM write_attempt wa
                    WHERE wa.org_id = tick.org_id AND wa.mapping_id = tick.id)
   AND NOT EXISTS (SELECT 1 FROM binding_candidate bc
                    WHERE bc.org_id = tick.org_id AND bc.mapping_id = tick.id);

-- 2b. Refuse rather than merge silently.
DO $$
DECLARE
    offenders text;
BEGIN
    SELECT string_agg(DISTINCT org_id::text, ', ') INTO offenders
    FROM (
        SELECT org_id FROM mapping
         WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz')
         GROUP BY org_id, product_id HAVING count(DISTINCT inventory) > 1
        UNION ALL
        SELECT org_id FROM mapping
         WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz')
           AND binding_state = 'bound' AND remote_numeric_id IS NOT NULL
         GROUP BY org_id, remote_id_kind, remote_numeric_id
        HAVING count(DISTINCT inventory) > 1
        UNION ALL
        SELECT org_id FROM mapping
         WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz')
           AND binding_state = 'bound' AND remote_url IS NOT NULL
         GROUP BY org_id, remote_url HAVING count(DISTINCT inventory) > 1
        UNION ALL
        SELECT org_id FROM election_rule
         WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz')
         GROUP BY org_id, axis, trigger_kind, trigger_key
        HAVING count(DISTINCT inventory) > 1
        UNION ALL
        SELECT org_id FROM election_item
         WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz') AND state = 'open'
         GROUP BY org_id, product_id, axis, trigger_kind, trigger_key
        HAVING count(DISTINCT inventory) > 1
        UNION ALL
        SELECT org_id FROM projection_override
         WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz')
         GROUP BY org_id, axis, from_term HAVING count(DISTINCT inventory) > 1
        UNION ALL
        SELECT org_id FROM org_inventory_halt
         WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz')
         GROUP BY org_id, marketplace HAVING count(DISTINCT inventory) > 1
        UNION ALL
        SELECT org_id FROM reconciliation_item
         WHERE target_inventory IN ('tes_gb', 'tes_us', 'tes_nz') AND state = 'open'
         GROUP BY org_id, term, target_term_kind
        HAVING count(DISTINCT target_inventory) > 1
    ) AS collisions;

    IF offenders IS NOT NULL THEN
        RAISE EXCEPTION
            'one Tes: these organisations hold a product on two Tes inventories and must be '
            'reconciled by hand before this migration runs: %', offenders;
    END IF;
END $$;

-- The global halt table carries no organisation, so its collision is named by
-- the marketplace it halts rather than by a tenant.
DO $$
DECLARE
    halted text;
BEGIN
    SELECT string_agg(marketplace, ', ') INTO halted
    FROM (
        SELECT marketplace FROM inventory_halt
         WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz')
         GROUP BY marketplace HAVING count(*) > 1
    ) AS collisions;

    IF halted IS NOT NULL THEN
        RAISE EXCEPTION
            'one Tes: two Tes inventories are halted at once on %, so lifting the halt is a '
            'decision to take before this migration runs', halted;
    END IF;
END $$;

-- 3. Rewrite tenant rows. `job_item.requires_bound_on` carries a
--    single-column foreign key onto `marketplace_inventory (code)`, so it
--    moves before the old codes are deleted or the delete fails.
UPDATE mapping SET inventory = 'tes' WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE job SET inventory = 'tes' WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE job_item SET requires_bound_on = 'tes'
    WHERE requires_bound_on IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE notification SET inventory = 'tes' WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE election_rule SET inventory = 'tes' WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE election_item SET inventory = 'tes' WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE projection_override SET inventory = 'tes'
    WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE import_batch_row SET inventory = 'tes' WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE sync_request SET source = 'tes' WHERE source IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE sync_request SET target = 'tes' WHERE target IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE grade_declaration SET source_inventory = 'tes'
    WHERE source_inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE grade_declaration_path SET inventory = 'tes'
    WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE native_residue SET inventory = 'tes' WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE product SET rights_source_inventory = 'tes'
    WHERE rights_source_inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE inventory_halt SET inventory = 'tes' WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE org_inventory_halt SET inventory = 'tes'
    WHERE inventory IN ('tes_gb', 'tes_us', 'tes_nz');
UPDATE reconciliation_item SET target_inventory = 'tes'
    WHERE target_inventory IN ('tes_gb', 'tes_us', 'tes_nz');

-- 4. The projection tables are derived from the crawled captures, so the
--    `tes_us` and `tes_nz` rows are deleted rather than merged and the
--    `tes_gb` rows are renamed. `tam-taxonomy-seed` re-derives the rest.
DELETE FROM projection_edge WHERE to_inventory IN ('tes_us', 'tes_nz');
DELETE FROM projection_no_counterpart WHERE to_inventory IN ('tes_us', 'tes_nz');
UPDATE projection_edge SET to_inventory = 'tes' WHERE to_inventory = 'tes_gb';
UPDATE projection_no_counterpart SET to_inventory = 'tes' WHERE to_inventory = 'tes_gb';

-- 5. Retire the three codes.
DELETE FROM marketplace_inventory WHERE code IN ('tes_gb', 'tes_us', 'tes_nz');

-- 6. Re-fence every table step 0 lifted, from the same list. The rewrites
--    above queue this transaction's deferred constraint triggers, and
--    `ALTER TABLE` refuses a table with pending trigger events, so they are
--    fired first -- which is also where a rewrite that broke an invariant
--    would surface.
SET CONSTRAINTS ALL IMMEDIATE;

DO $$
DECLARE
    unfenced record;
BEGIN
    FOR unfenced IN SELECT name FROM one_tes_unfenced LOOP
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', unfenced.name);
    END LOOP;
END $$;

-- The Tes market as a datum on the connection rather than as a variant of the
-- inventory enum, so a seller on a second Tes market is a column value and
-- never a re-forked dimension. Surfaced nowhere yet; the adapter reads the GB
-- tree and the GB age bands.
ALTER TABLE connection ADD COLUMN country text NOT NULL DEFAULT 'GB';

COMMENT ON COLUMN connection.country IS
    'The Tes market this connection authors into, ISO 3166-1 alpha-2, defaulted to GB. '
    'Read for Tes and ignored elsewhere.';
