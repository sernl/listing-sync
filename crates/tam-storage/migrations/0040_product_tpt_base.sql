-- The TPT-base fields of a product, beside the product rather than inside it.
--
-- TPT is the canonical base model by founder decision, and its create form
-- carries eight groups of which `product` already holds three: the name, the
-- description with its declared format, the price, the payload files and the
-- verbatim grade declaration. What it does not hold is everything below, and
-- a sidecar keyed on the product is how those arrive without moving a column
-- that every adapter, the engine and the import already read.
--
-- One row per product, so the primary key is the product's own key. A product
-- with no row here is one authored before this table existed or through a
-- path that does not carry these fields; every read is therefore an outer
-- join and the absence is a fact rather than a fault.
--
-- The three picker arrays are TPT facet slugs, `data[TaxonomyTags][]` values,
-- not canonical term ids. They are stored verbatim because that is what the
-- seller chose and what the wire takes; the projection onto another
-- marketplace goes through the crosswalk relation and never through this
-- table. No CHECK constrains their membership: the catalogue holds 358 slugs,
-- it is re-polled, and a slug retired between a poll and a write must land in
-- a row an operator can read rather than abort the seller's create.
--
-- The cardinality caps are deliberately not CHECK constraints either. They
-- are properties of a form measured on a particular day, they live in
-- docs/design/data/tpt-vocabulary.json, and one of them -- the subject-area
-- cap of three -- has already been contradicted by a capture TPT accepted.
-- Freezing a measured claim into DDL would make a re-poll a migration.
--
-- tax_code_id, teaching_duration_id, answer_key_id and copyright_declaration_id
-- are all nullable except where the model refuses the absence, and the
-- refusals live in crates/tam-domain/src/authoring.rs rather than here for the
-- same reason: they are form rules, and the form is where a seller can act on
-- them. The one exception is status_user, which is NOT NULL because draft
-- versus live is a field on every TPT listing and has no unset state.
--
-- copyright_declaration_id records which of TPT's two attestations the seller
-- selected. It is nullable, and a NULL is a product that cannot be submitted
-- rather than one attested by default: TPT's own form arrives with value 1
-- pre-selected, and a default here would make the attestation ours rather
-- than the seller's.
--
-- standards is jsonb rather than a table because no standard has been
-- ingested yet: the four frameworks' catalogues are a separate stream, the
-- code-to-node-id table TPT needs is its own crawl, and a normalised
-- alignment table with no catalogue to reference would be an empty foreign
-- key. Each element is {framework, code, tpt_node_id}; tpt_node_id is
-- optional because TPT's id is a search-index identifier that gets rebuilt,
-- and an alignment we can display but not yet post is a state worth holding.

CREATE TABLE product_tpt_base (
    org_id     uuid NOT NULL,
    product_id uuid NOT NULL,

    -- data[Item][generate_thumbnail]: 1 auto-generate, 2 upload now,
    -- 3 upload later. The four slots are the conditional body of 2.
    thumbnail_mode   smallint NOT NULL,
    -- data[ItemDigital][thumb1..thumb4], first slot first. Content hashes,
    -- the same handle the upload endpoint returns.
    thumbnail_hashes bytea[]  NOT NULL DEFAULT '{}',
    -- data[Upload][videopreview].
    video_preview_hash bytea,

    -- data[Item][license_price]. Carried explicitly and never derived: TPT's
    -- 90 percent is the form's pre-fill, and a projection that recomputed it
    -- would overwrite a seller's own figure on every sync.
    additional_licence_minor_units bigint,
    -- data[Item][discountprice].
    bundle_discount_minor_units    bigint,
    -- data[ItemTaxCode][tax_code_id], the row id and never the Avalara code.
    tax_code_id                    smallint,

    subject_area_slugs text[] NOT NULL DEFAULT '{}',
    tag_slugs          text[] NOT NULL DEFAULT '{}',
    format_slugs       text[] NOT NULL DEFAULT '{}',
    -- data[Category][Category][]: the seller's own shelves, not a platform
    -- vocabulary, so no member set exists to constrain against.
    custom_categories  text[] NOT NULL DEFAULT '{}',

    standards jsonb NOT NULL DEFAULT '[]',

    teaching_duration_id smallint,
    pages_or_slides      integer,
    answer_key_id        smallint,

    copyright_declaration_id smallint,
    -- data[Item][status_user]: 0 draft, 1 live.
    status_user              smallint NOT NULL,

    updated_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, product_id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),

    -- The closed vocabularies whose member sets are wire facts rather than
    -- measured claims: each is enumerated by a captured form control and has
    -- not moved across two captures four days apart.
    CONSTRAINT product_tpt_base_thumbnail_mode CHECK (thumbnail_mode IN (1, 2, 3)),
    CONSTRAINT product_tpt_base_status CHECK (status_user IN (0, 1)),
    CONSTRAINT product_tpt_base_copyright CHECK (
        copyright_declaration_id IS NULL OR copyright_declaration_id IN (1, 2)
    ),
    CONSTRAINT product_tpt_base_tax_code CHECK (
        tax_code_id IS NULL OR tax_code_id BETWEEN 1 AND 5
    ),
    CONSTRAINT product_tpt_base_answer_key CHECK (
        answer_key_id IS NULL OR answer_key_id BETWEEN 0 AND 5
    ),
    CONSTRAINT product_tpt_base_teaching_duration CHECK (
        teaching_duration_id IS NULL OR teaching_duration_id BETWEEN 0 AND 22
    ),
    CONSTRAINT product_tpt_base_pages CHECK (
        pages_or_slides IS NULL OR pages_or_slides > 0
    ),
    -- The four thumbnail slots are the form's own count, and unlike the
    -- picker caps this one is a slot count rather than a claim about a
    -- vocabulary: the form renders four boxes and there is no fifth to fill.
    CONSTRAINT product_tpt_base_thumbnail_slots CHECK (
        array_length(thumbnail_hashes, 1) IS NULL OR array_length(thumbnail_hashes, 1) <= 4
    ),
    -- Thumbnails supplied under a mode whose slots the form does not render
    -- were collected by a control TPT does not show.
    CONSTRAINT product_tpt_base_thumbnails_need_upload_now CHECK (
        array_length(thumbnail_hashes, 1) IS NULL OR thumbnail_mode = 2
    )
);

-- Row-level security keyed on the tenant, matching every other tenant table.
-- FORCE applies the policy to the owner, which is the role the application
-- and the tests connect as; without it the policy would be theatre.
ALTER TABLE product_tpt_base ENABLE ROW LEVEL SECURITY;
ALTER TABLE product_tpt_base FORCE ROW LEVEL SECURITY;

CREATE POLICY product_tpt_base_org_isolation ON product_tpt_base
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);
