-- What the source stated about the rights it grants, and the source values
-- this model does not yet type.
--
-- A licence is a legal instrument and a translation of one is a different
-- instrument, so the grant is kept as the source's own value: which
-- inventory's vocabulary it came from, that vocabulary's own path, and its
-- wire token. Nothing derives one from another.
--
-- No grant on the product columns: 0007 grants table-level SELECT ON product
-- to tam_engine and a table grant covers columns added later, which 0019
-- confirmed for job_item.

-- Expand, backfill, switch, contract, exactly as 0019 did for job_item.
-- 'unstated' is the correct backfill because no existing product ever
-- captured a grant, which is the defect this column exists to fix.
ALTER TABLE product ADD COLUMN rights_state            text NOT NULL DEFAULT 'unstated';
ALTER TABLE product ADD COLUMN rights_source_inventory text;
ALTER TABLE product ADD COLUMN rights_segments         text[];
ALTER TABLE product ADD COLUMN rights_native_id        text;
ALTER TABLE product ALTER COLUMN rights_state DROP DEFAULT;

ALTER TABLE product ADD CONSTRAINT product_rights_total CHECK (
    CASE rights_state
        WHEN 'unstated' THEN rights_source_inventory IS NULL
                         AND rights_segments IS NULL AND rights_native_id IS NULL
        WHEN 'declared' THEN rights_source_inventory IS NOT NULL
                         AND rights_segments IS NOT NULL
        ELSE false
    END
);

-- Source values in axes this model does not yet type. term_kind is NULLABLE
-- deliberately: eight of TPT's twelve facet categories arrive in one flat
-- namespace with no axis of their own, and a residue that cannot hold a
-- kind-less term is not a residue. Born nullable rather than widened later,
-- because relaxing a NOT NULL column afterwards is the whole
-- expand-backfill-switch-contract dance for nothing.
--
-- Forcing an unclassified slug to some existing kind would be worse than
-- holding it kind-less: a value mislabelled 'topic' is projected as a topic
-- rather than named as a loss, which is exactly what this table exists to
-- prevent.
CREATE TABLE native_residue (
    org_id     uuid NOT NULL REFERENCES organisation (id),
    product_id uuid NOT NULL,
    position   int  NOT NULL,
    inventory  text NOT NULL,
    term_kind  text,
    segments   text[] NOT NULL,
    native_id  text,

    PRIMARY KEY (org_id, product_id, position),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id) ON DELETE CASCADE,

    CONSTRAINT native_residue_kind CHECK (
        term_kind IS NULL
     OR term_kind IN ('subject', 'topic', 'resource_type', 'phase', 'licence')
    )
);

ALTER TABLE native_residue ENABLE ROW LEVEL SECURITY;
ALTER TABLE native_residue FORCE ROW LEVEL SECURITY;
CREATE POLICY native_residue_org_isolation ON native_residue
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- A new child of product inherits nothing. 0016's grant block exists because
-- the worker seeds the machine from the whole product aggregate, so without
-- this the engine's product read raises 42501 at runtime rather than at
-- migration time.
GRANT SELECT ON native_residue TO tam_engine;

-- A licence becomes a kind a canonical term may have. These are the only two
-- CHECKs in the tree that constrain a term kind: projection_no_counterpart
-- and reconciliation_item carry none, and widening a constraint that does not
-- exist aborts the whole migration with 42704.
ALTER TABLE canonical_term DROP CONSTRAINT canonical_term_kind;
ALTER TABLE canonical_term ADD CONSTRAINT canonical_term_kind
    CHECK (kind IN ('subject', 'topic', 'resource_type', 'phase', 'licence'));

ALTER TABLE projection_edge DROP CONSTRAINT projection_edge_term_kind;
ALTER TABLE projection_edge ADD CONSTRAINT projection_edge_term_kind
    CHECK (to_term_kind IN ('subject', 'topic', 'resource_type', 'phase', 'licence'));
