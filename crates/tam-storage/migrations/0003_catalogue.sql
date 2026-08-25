-- The catalogue around product: content-addressed blobs, the files that
-- reference them, taxonomy assignments and the grade declaration. Every
-- tenant table here gets the same forced row-level-security policy as
-- product; canonical_term is global (the canonical taxonomy is ours, not a
-- tenant's) and M1g owns its full shape.

CREATE TABLE canonical_term (
    id     uuid NOT NULL,
    kind   text NOT NULL,
    parent uuid REFERENCES canonical_term (id),
    label  text NOT NULL,

    PRIMARY KEY (id),

    CONSTRAINT canonical_term_kind CHECK (
        kind IN ('subject', 'topic', 'resource_type', 'phase')
    )
);

-- Deduplication is per tenant rather than global, because a global blob table
-- keyed on hash alone is an existence oracle and is unachievable in any case
-- under per-tenant file encryption.
CREATE TABLE blob (
    org_id          uuid        NOT NULL REFERENCES organisation (id),
    hash            bytea       NOT NULL,
    byte_len        bigint      NOT NULL,
    object_key      text        NOT NULL,
    dek_key_version int         NOT NULL,
    first_seen_at   timestamptz NOT NULL,

    PRIMARY KEY (org_id, hash)
);

-- byte_len lives on blob alone; a file row reaches it through its hash, so
-- two rows for one blob cannot disagree about its size.
CREATE TABLE product_file (
    org_id            uuid        NOT NULL REFERENCES organisation (id),
    id                uuid        NOT NULL,
    product_id        uuid        NOT NULL,
    position          int         NOT NULL,
    role              text        NOT NULL,
    kind              text        NOT NULL,
    hash              bytea       NOT NULL,
    scan_state        text        NOT NULL,
    scan_signature    text,
    scanned_at        timestamptz,
    scan_failure_code text,
    created_at        timestamptz NOT NULL,
    deleted_at        timestamptz,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),
    FOREIGN KEY (org_id, hash)       REFERENCES blob (org_id, hash),

    CONSTRAINT product_file_position UNIQUE (org_id, product_id, position),

    CONSTRAINT product_file_role CHECK (role IN ('payload', 'preview', 'cover')),
    CONSTRAINT product_file_kind CHECK (kind IN ('pdf', 'pptx', 'docx', 'zip', 'image')),

    CONSTRAINT product_file_scan_total CHECK (
        (scan_state = 'pending'  AND scan_signature IS NULL     AND scanned_at IS NULL     AND scan_failure_code IS NULL)
     OR (scan_state = 'clean'    AND scan_signature IS NULL     AND scanned_at IS NOT NULL AND scan_failure_code IS NULL)
     OR (scan_state = 'infected' AND scan_signature IS NOT NULL AND scan_failure_code IS NULL)
     OR (scan_state = 'failed'   AND scan_failure_code IS NOT NULL AND scan_signature IS NULL)
    )
);

CREATE UNIQUE INDEX product_file_one_cover
    ON product_file (org_id, product_id)
    WHERE role = 'cover' AND deleted_at IS NULL;

CREATE TABLE product_term (
    org_id     uuid NOT NULL REFERENCES organisation (id),
    product_id uuid NOT NULL,
    term_id    uuid NOT NULL REFERENCES canonical_term (id),
    position   int  NOT NULL,

    PRIMARY KEY (org_id, product_id, term_id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),

    CONSTRAINT product_term_position UNIQUE (org_id, product_id, position)
);

CREATE TABLE grade_declaration (
    org_id             uuid NOT NULL REFERENCES organisation (id),
    product_id         uuid NOT NULL,
    source             text NOT NULL,
    source_inventory   text,
    source_term_kind   text,
    derived_low_years  smallint,
    derived_high_years smallint,

    PRIMARY KEY (org_id, product_id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),

    CONSTRAINT grade_declaration_source CHECK (
        (source = 'seller'   AND source_inventory IS NULL     AND source_term_kind IS NULL)
     OR (source = 'imported' AND source_inventory IS NOT NULL AND source_term_kind IS NOT NULL)
    ),

    CONSTRAINT grade_declaration_interval CHECK (
        (derived_low_years IS NULL) = (derived_high_years IS NULL)
        AND (derived_low_years IS NULL
             OR (derived_low_years BETWEEN 0 AND 255
                 AND derived_high_years BETWEEN 0 AND 255
                 AND derived_low_years <= derived_high_years))
    )
);

-- The seller's declaration verbatim as an ordered list of vocabulary terms,
-- which is what makes the grade round-trip law hold by construction.
CREATE TABLE grade_declaration_path (
    org_id     uuid   NOT NULL REFERENCES organisation (id),
    product_id uuid   NOT NULL,
    position   int    NOT NULL,
    inventory  text   NOT NULL,
    term_kind  text   NOT NULL,
    segments   text[] NOT NULL,
    native_id  text,

    PRIMARY KEY (org_id, product_id, position),
    FOREIGN KEY (org_id, product_id)
        REFERENCES grade_declaration (org_id, product_id) ON DELETE CASCADE
);

-- PayloadSet is non-empty by construction in Rust; this closes the same
-- invariant in SQL. PostgreSQL cannot defer a CHECK, which is why it is a
-- deferred constraint trigger. It runs at commit while the transaction's
-- app.current_org pin is still live, so the lookups see the tenant's rows.
CREATE FUNCTION assert_product_has_payload() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    checked_org     uuid;
    checked_product uuid;
BEGIN
    IF TG_TABLE_NAME = 'product' THEN
        checked_org     := NEW.org_id;
        checked_product := NEW.id;
    ELSE
        checked_org     := COALESCE(NEW.org_id, OLD.org_id);
        checked_product := COALESCE(NEW.product_id, OLD.product_id);
    END IF;

    PERFORM 1 FROM product p
        WHERE p.org_id = checked_org AND p.id = checked_product
          AND p.deleted_at IS NULL;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    PERFORM 1 FROM product_file f
        WHERE f.org_id = checked_org AND f.product_id = checked_product
          AND f.role = 'payload' AND f.deleted_at IS NULL;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'product % has no live payload file', checked_product
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER product_payload_nonempty
    AFTER INSERT OR UPDATE ON product
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_product_has_payload();

CREATE CONSTRAINT TRIGGER product_file_payload_nonempty
    AFTER UPDATE OR DELETE ON product_file
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_product_has_payload();

ALTER TABLE blob ENABLE ROW LEVEL SECURITY;
ALTER TABLE blob FORCE ROW LEVEL SECURITY;
CREATE POLICY blob_org_isolation ON blob
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE product_file ENABLE ROW LEVEL SECURITY;
ALTER TABLE product_file FORCE ROW LEVEL SECURITY;
CREATE POLICY product_file_org_isolation ON product_file
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE product_term ENABLE ROW LEVEL SECURITY;
ALTER TABLE product_term FORCE ROW LEVEL SECURITY;
CREATE POLICY product_term_org_isolation ON product_term
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE grade_declaration ENABLE ROW LEVEL SECURITY;
ALTER TABLE grade_declaration FORCE ROW LEVEL SECURITY;
CREATE POLICY grade_declaration_org_isolation ON grade_declaration
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE grade_declaration_path ENABLE ROW LEVEL SECURITY;
ALTER TABLE grade_declaration_path FORCE ROW LEVEL SECURITY;
CREATE POLICY grade_declaration_path_org_isolation ON grade_declaration_path
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);
