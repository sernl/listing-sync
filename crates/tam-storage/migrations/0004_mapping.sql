-- The mapping aggregate: the binding between a canonical product and a remote
-- listing, its verification, policies, price rule and remote lifecycle.
-- The DDL follows docs/design/schema.md; that document calls itself a sketch
-- rather than a migration, and sketches/domain.rs is the artefact of record
-- for the types, so columns the sketch omits for fields the types carry are
-- added here rather than the fields dropped: binding_marker
-- (Binding::Creating.marker), ambiguous_since (Binding::AmbiguousCreate.since)
-- with its binding_candidate child table, verify_stale_since
-- (Verification::Stale.since), lifecycle_reason (RemoteLifecycle::Rejected
-- .reason), the price_rule column group (Mapping.price_rule), and a position
-- on field_mismatch so Mismatched's first-then-rest order survives.

CREATE TABLE mapping (
    org_id             uuid        NOT NULL REFERENCES organisation (id),
    id                 uuid        NOT NULL,
    product_id         uuid        NOT NULL,
    inventory          text        NOT NULL,
    marketplace        text        NOT NULL,

    binding_state      text        NOT NULL,
    remote_id_kind     text,
    remote_url         text,
    remote_numeric_id  bigint,
    binding_attempt    uuid,
    binding_marker     text,
    first_seen_at      timestamptz,
    ambiguous_since    timestamptz,
    severed_at         timestamptz,
    sever_cause        text,

    verify_state       text        NOT NULL,
    verified_at        timestamptz,
    verify_stale_since timestamptz,
    normaliser_version int         NOT NULL,

    policy_title       text        NOT NULL,
    policy_description text        NOT NULL,
    policy_price       text        NOT NULL,
    policy_taxonomy    text        NOT NULL,
    policy_grades      text        NOT NULL,
    policy_files       text        NOT NULL,

    price_rule_kind            text   NOT NULL,
    price_rate_micros          bigint,
    price_rounding             text,
    price_explicit_kind        text,
    price_explicit_minor_units bigint,
    price_explicit_currency    text,

    publish_mode       text        NOT NULL,
    lifecycle_state    text        NOT NULL,
    lifecycle_since    timestamptz,
    lifecycle_reason   text,

    created_at         timestamptz NOT NULL,
    updated_at         timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),
    FOREIGN KEY (inventory, marketplace)
        REFERENCES marketplace_inventory (code, marketplace),

    CONSTRAINT mapping_one_per_inventory UNIQUE (org_id, product_id, inventory),

    CONSTRAINT mapping_binding_total CHECK (
        CASE binding_state
            WHEN 'unbound'          THEN remote_id_kind IS NULL AND binding_attempt IS NULL
            WHEN 'creating'         THEN remote_id_kind IS NULL AND binding_attempt IS NOT NULL
            WHEN 'bound'            THEN remote_id_kind IS NOT NULL AND first_seen_at IS NOT NULL
            WHEN 'ambiguous_create' THEN remote_id_kind IS NULL
                                     AND binding_attempt IS NOT NULL
                                     AND ambiguous_since IS NOT NULL
            WHEN 'severed'          THEN remote_id_kind IS NOT NULL
                                     AND severed_at IS NOT NULL
                                     AND sever_cause IS NOT NULL
            ELSE false
        END
    ),

    CONSTRAINT mapping_binding_marker CHECK (
        binding_marker IS NULL OR binding_state = 'creating'
    ),

    CONSTRAINT mapping_remote_id_shape CHECK (
        remote_id_kind IS NULL
     OR (remote_id_kind = 'tes'
         AND remote_url IS NOT NULL AND remote_numeric_id IS NULL)
     OR (remote_id_kind IN ('tpt', 'etsy')
         AND remote_numeric_id IS NOT NULL AND remote_url IS NULL)
    ),

    CONSTRAINT mapping_remote_id_marketplace CHECK (
        remote_id_kind IS NULL OR remote_id_kind = marketplace
    ),

    CONSTRAINT mapping_verify_total CHECK (
        (verify_state = 'stale' AND verified_at IS NULL)
     OR (verify_state IN ('clean', 'mismatched') AND verified_at IS NOT NULL)
    ),

    -- Verification exists only inside Binding::Bound; every other state
    -- stores the neutral 'stale' with no instant, and a bound stale row
    -- carries the instant Verification::Stale names.
    CONSTRAINT mapping_verify_bound CHECK (
        (binding_state = 'bound'
         AND (verify_state <> 'stale' OR verify_stale_since IS NOT NULL))
     OR (binding_state <> 'bound'
         AND verify_state = 'stale' AND verify_stale_since IS NULL)
    ),

    CONSTRAINT mapping_policies CHECK (
        policy_title       IN ('managed', 'frozen', 'propose')
    AND policy_description IN ('managed', 'frozen', 'propose')
    AND policy_price       IN ('managed', 'frozen', 'propose')
    AND policy_taxonomy    IN ('managed', 'frozen', 'propose')
    AND policy_grades      IN ('managed', 'frozen', 'propose')
    AND policy_files       IN ('managed', 'frozen', 'propose')
    ),

    CONSTRAINT mapping_price_rule_total CHECK (
        (price_rule_kind = 'converted'
         AND price_rate_micros IS NOT NULL
         AND price_rounding IN ('nearest', 'up_to_charm')
         AND price_explicit_kind IS NULL
         AND price_explicit_minor_units IS NULL
         AND price_explicit_currency IS NULL)
     OR (price_rule_kind = 'explicit'
         AND price_rate_micros IS NULL
         AND price_rounding IS NULL
         AND ((price_explicit_kind = 'free'
               AND price_explicit_minor_units IS NULL
               AND price_explicit_currency IS NULL)
           OR (price_explicit_kind = 'paid'
               AND price_explicit_minor_units > 0
               AND price_explicit_currency IS NOT NULL)))
    ),

    CONSTRAINT mapping_publish_mode CHECK (
        publish_mode IN ('dry_run', 'propose', 'publish')
    ),

    CONSTRAINT mapping_lifecycle_total CHECK (
        lifecycle_state IN ('absent', 'draft', 'submitted', 'in_review',
                            'live', 'rejected', 'withdrawn')
    AND ((lifecycle_state IN ('absent', 'draft')) = (lifecycle_since IS NULL))
    AND (lifecycle_reason IS NULL OR lifecycle_state = 'rejected')
    ),

    CONSTRAINT mapping_sever_cause CHECK (
        sever_cause IS NULL
     OR sever_cause IN ('removed_by_marketplace', 'removed_by_seller',
                        'not_found_on_verify')
    )
);

-- The candidate identifiers an ambiguous create could not decide between.
CREATE TABLE binding_candidate (
    org_id            uuid   NOT NULL REFERENCES organisation (id),
    mapping_id        uuid   NOT NULL,
    position          int    NOT NULL,
    remote_id_kind    text   NOT NULL,
    remote_url        text,
    remote_numeric_id bigint,

    PRIMARY KEY (org_id, mapping_id, position),
    FOREIGN KEY (org_id, mapping_id) REFERENCES mapping (org_id, id) ON DELETE CASCADE,

    CONSTRAINT binding_candidate_shape CHECK (
        (remote_id_kind = 'tes'
         AND remote_url IS NOT NULL AND remote_numeric_id IS NULL)
     OR (remote_id_kind IN ('tpt', 'etsy')
         AND remote_numeric_id IS NOT NULL AND remote_url IS NULL)
    )
);

CREATE TABLE field_mismatch (
    org_id         uuid NOT NULL REFERENCES organisation (id),
    mapping_id     uuid NOT NULL,
    position       int  NOT NULL,
    field          text NOT NULL,
    class          text NOT NULL,
    observed_in    text,
    limit_observed int,

    PRIMARY KEY (org_id, mapping_id, field),
    FOREIGN KEY (org_id, mapping_id) REFERENCES mapping (org_id, id) ON DELETE CASCADE,

    CONSTRAINT field_mismatch_position UNIQUE (org_id, mapping_id, position),

    CONSTRAINT field_mismatch_field CHECK (
        field IN ('title', 'description', 'price', 'taxonomy', 'grades', 'files')
    ),

    CONSTRAINT field_mismatch_class_total CHECK (
        (class = 'normalised' AND observed_in IS NULL AND limit_observed IS NULL)
     OR (class = 'truncated'  AND observed_in IS NULL AND limit_observed IS NOT NULL)
     OR (class = 'missing'    AND observed_in IS NULL AND limit_observed IS NULL)
     OR (class = 'wrong_field' AND observed_in IS NOT NULL AND limit_observed IS NULL)
     OR (class = 'unexpected' AND observed_in IS NULL AND limit_observed IS NULL)
    )
);

-- Verification::Mismatched is non-empty by construction in Rust; the deferred
-- trigger closes the same invariant in SQL, so the state cannot claim a
-- mismatch it cannot name.
CREATE FUNCTION assert_mismatch_nonempty() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.verify_state = 'mismatched' THEN
        PERFORM 1 FROM field_mismatch fm
            WHERE fm.org_id = NEW.org_id AND fm.mapping_id = NEW.id;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'mapping % claims a mismatch it cannot name', NEW.id
                USING ERRCODE = 'check_violation';
        END IF;
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER mapping_mismatch_nonempty
    AFTER INSERT OR UPDATE ON mapping
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_mismatch_nonempty();

ALTER TABLE mapping ENABLE ROW LEVEL SECURITY;
ALTER TABLE mapping FORCE ROW LEVEL SECURITY;
CREATE POLICY mapping_org_isolation ON mapping
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE binding_candidate ENABLE ROW LEVEL SECURITY;
ALTER TABLE binding_candidate FORCE ROW LEVEL SECURITY;
CREATE POLICY binding_candidate_org_isolation ON binding_candidate
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);

ALTER TABLE field_mismatch ENABLE ROW LEVEL SECURITY;
ALTER TABLE field_mismatch FORCE ROW LEVEL SECURITY;
CREATE POLICY field_mismatch_org_isolation ON field_mismatch
    FOR ALL
    USING (org_id = current_setting('app.current_org', true)::uuid)
    WITH CHECK (org_id = current_setting('app.current_org', true)::uuid);
