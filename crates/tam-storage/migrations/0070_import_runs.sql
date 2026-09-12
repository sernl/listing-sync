-- One import is one run, with a row per resource and a review before anything
-- is created.
--
-- Phase 2 of `docs/notes/design/2026-09-12-one-marketplace-per-site-and-the-seller-workflows.md`
-- section 2. Two sources -- a spreadsheet and a marketplace the seller's own
-- device reads -- reach one outcome: resources in the catalogue, with nothing
-- drafted anywhere. The two used to differ in where they paused. The
-- spreadsheet stopped between the parse and the commit (0058's own header:
-- "nothing here creates anything"); the marketplace import created a product
-- the moment a page landed, which left no instant at which a duplicate could
-- be put to the seller. This migration gives the marketplace source the same
-- pause, and gives both sources the same row shape, so one review and one
-- chunked commit serve them.
--
-- Nothing here creates a product either. An `import_run_item` holds what the
-- read said and what the matcher concluded; the products, the labels and the
-- fingerprints are a later, deliberate commit, exactly as 0058 states for a
-- spreadsheet row.

-- Trigram similarity, for the title layer of the matcher. Created here rather
-- than assumed: a deployment that has never needed it has no extension, and
-- the GIN index below names the operator class the extension defines, so an
-- absent extension fails this migration rather than the first duplicate read.
CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE import_run (
    org_id         uuid        NOT NULL REFERENCES organisation (id),
    -- The submit's own idempotency key, as `job`, `sync_request` and
    -- `import_batch` all are.
    id             uuid        NOT NULL,
    -- Which machinery produced it. Closed rather than derived from the
    -- presence of `source`: a spreadsheet run and a marketplace run with no
    -- readable source must not read the same.
    kind           text        NOT NULL,
    -- The shop this read, for a marketplace run. NULL on a spreadsheet run.
    -- Unconstrained by a foreign key for the reason `sync_request.source`
    -- gives: the repository's own round trip through InventoryId is what
    -- admits a value.
    source         text,
    -- The spreadsheet batch this run reviews, for a spreadsheet run.
    batch_id       uuid,
    -- Catalogue-only import names no target. A column rather than an absence
    -- because a future "import and draft" would name one, and reading NULL as
    -- "never drafted" is the same trap 0053 names.
    target         text,
    state          text        NOT NULL,
    -- The anchor job every event of this run hangs from, keyed on the run. A
    -- `job_event` row requires a job and a run is not one; the itemless job
    -- `ImportDrainMeasured` already uses is what carries the ledger.
    anchor_job     uuid        NOT NULL,
    -- Known after the enumeration, for the reason `ImportProgress.total` is
    -- zero before one: a total of nothing must not read as an empty shop.
    read_total     int,
    created_at     timestamptz NOT NULL,
    settled_at     timestamptz,
    failure_detail text,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, batch_id) REFERENCES import_batch (org_id, id),

    CONSTRAINT import_run_kind CHECK (kind IN ('marketplace', 'spreadsheet')),
    CONSTRAINT import_run_state CHECK (
        state IN ('reading', 'reviewing', 'committing', 'complete', 'failed', 'abandoned')
    ),
    -- A marketplace run names a shop; a spreadsheet run names a batch.
    CONSTRAINT import_run_source_follows_kind CHECK (
        (kind = 'marketplace') = (source IS NOT NULL)
    ),
    CONSTRAINT import_run_batch_follows_kind CHECK (
        (kind = 'spreadsheet') = (batch_id IS NOT NULL)
    ),
    -- 0025's totality constraint.
    CONSTRAINT import_run_settled_total CHECK (
        (state IN ('reading', 'reviewing', 'committing')) = (settled_at IS NULL)
    ),
    CONSTRAINT import_run_failure_detail CHECK (
        failure_detail IS NULL OR state IN ('failed', 'abandoned')
    ),
    CONSTRAINT import_run_read_total_non_negative CHECK (
        read_total IS NULL OR read_total >= 0
    ),
    CONSTRAINT import_run_failure_detail_bounded CHECK (
        failure_detail IS NULL OR char_length(failure_detail) <= 500
    )
);

-- One open run per org, as `import_batch_one_open_per_org` is.
CREATE UNIQUE INDEX import_run_one_open_per_org ON import_run (org_id)
    WHERE state IN ('reading', 'reviewing', 'committing');

-- Newest first, which is the listing's own order.
CREATE INDEX import_run_by_age ON import_run (org_id, created_at DESC, id);

CREATE TABLE import_run_item (
    org_id         uuid        NOT NULL REFERENCES organisation (id),
    run_id         uuid        NOT NULL,
    -- How the source addresses it: a marketplace locator, or 'sheet#ordinal'.
    -- The run's own handle, and the replay key a re-posted page dedups by.
    locator        text        NOT NULL,
    -- The read order, so the console renders what the seller saw.
    ordinal        int         NOT NULL,
    state          text        NOT NULL,
    -- The reserved product identifier, unconstrained by a foreign key for the
    -- reason `import_batch_row.product_id` gives: it is minted at the read so
    -- the matcher has a stable name for a side of a pair, and the product it
    -- names exists only once the commit has run. A resumed commit skips what
    -- it already made by reading whether that product exists.
    product_id     uuid,
    -- What the read said, verbatim: the `ObservedResource` minus its cover
    -- bytes, which are stored as a blob and named by `cover_hash`. The
    -- document is never joined against; it is the commit's own input, written
    -- once by the route that validated it, which is the discipline 0056 and
    -- 0058 state for `draft`.
    observed       jsonb,
    -- The review card's own fields, lifted out of `observed` so the list read
    -- is one query rather than a document walk per row.
    title          text,
    price_minor    bigint,
    price_currency text,
    cover_hash     bytea,
    -- Which machine described this, and when it says it did, in 0052's voice
    -- and for its reason: the description is a claim a device made over bytes
    -- we never held, and the column name has to say so. NULL where no machine
    -- described it -- a spreadsheet row's draft is the seller's own typing.
    observed_by_device text,
    observed_at        timestamptz,
    failure_detail text,
    skip_reason    text,
    read_at        timestamptz NOT NULL,
    settled_at     timestamptz,

    PRIMARY KEY (org_id, run_id, locator),
    FOREIGN KEY (org_id, run_id) REFERENCES import_run (org_id, id),
    FOREIGN KEY (org_id, cover_hash) REFERENCES blob (org_id, hash),
    FOREIGN KEY (org_id, observed_by_device) REFERENCES device (org_id, id),

    CONSTRAINT import_run_item_ordinal_positive CHECK (ordinal > 0),
    -- `listed` and `selected` precede the read: the device posts the shop's
    -- own list first and the seller ticks it, so a run holds rows before it
    -- holds descriptions. `read` is the state that holds `observed`.
    CONSTRAINT import_run_item_state CHECK (
        state IN ('listed', 'selected', 'read', 'matched', 'review',
                  'imported', 'skipped', 'failed')
    ),
    CONSTRAINT import_run_item_locator_bounded CHECK (
        char_length(locator) BETWEEN 1 AND 400
    ),
    CONSTRAINT import_run_item_skip_reason_bounded CHECK (
        skip_reason IS NULL OR char_length(skip_reason) <= 500
    ),
    CONSTRAINT import_run_item_failure_detail_bounded CHECK (
        failure_detail IS NULL OR char_length(failure_detail) <= 500
    ),
    CONSTRAINT import_run_item_observed_is_object CHECK (
        observed IS NULL OR jsonb_typeof(observed) = 'object'
    ),
    -- The route's own ceilings, summed, with headroom. `COPY_MAX` is 64 KiB
    -- for a listing's title and 64 KiB again for its body, and the sketch, the
    -- digests and the locator add about a kilobyte; a smaller number here
    -- would refuse a listing the route had already accepted, which is the one
    -- failure a stored document must not have. The cover never enters it: its
    -- bytes are a blob named by `cover_hash`.
    CONSTRAINT import_run_item_observed_bounded CHECK (
        observed IS NULL OR octet_length(observed::text) <= 262144
    ),
    -- A described row holds the description. Before the read there is nothing
    -- to hold, and a row that failed its read holds a reason instead.
    CONSTRAINT import_run_item_observed_follows_state CHECK (
        observed IS NOT NULL
        OR state IN ('listed', 'selected', 'skipped', 'failed')
    ),
    -- A price is an amount and its denomination, or neither.
    CONSTRAINT import_run_item_price_total CHECK (
        (price_minor IS NULL) = (price_currency IS NULL)
    ),
    -- A device's assertion is the machine and the instant together, for
    -- 0045's reason: an instant nobody attributed is not a device-asserted
    -- fact.
    CONSTRAINT import_run_item_observation_total CHECK (
        (observed_by_device IS NULL) = (observed_at IS NULL)
    ),
    CONSTRAINT import_run_item_settled_total CHECK (
        (state IN ('listed', 'selected', 'read', 'matched', 'review')) = (settled_at IS NULL)
    ),
    CONSTRAINT import_run_item_failure_detail CHECK (
        (failure_detail IS NULL) = (state <> 'failed')
    ),
    CONSTRAINT import_run_item_skip_reason_total CHECK (
        (skip_reason IS NULL) = (state <> 'skipped')
    )
);

CREATE INDEX import_run_item_by_state ON import_run_item (org_id, run_id, state);
CREATE UNIQUE INDEX import_run_item_ordinal ON import_run_item (org_id, run_id, ordinal);

-- The device-asserted content sketch of one resource, in the shape and the
-- voice `0052_product_file_source.sql:168-175` states: every column here is a
-- claim a machine we do not operate made over bytes we never held, and none of
-- it is our verification.
CREATE TABLE product_fingerprint (
    org_id              uuid        NOT NULL REFERENCES organisation (id),
    product_id          uuid        NOT NULL,
    -- Freezes the sketch format. A bump re-computes on the device; sketches
    -- are never compared across versions.
    fingerprint_version smallint    NOT NULL,

    -- Text layer. NULL where the device extracted nothing, which is not zero.
    text_simhash        bigint,
    text_minhash        bytea,
    shingle_count       int,
    extracted_chars     int,
    -- Manku's banded blocking, as stored columns so the index is on values
    -- rather than on an expression over a nullable bigint.
    simhash_band_0      smallint,
    simhash_band_1      smallint,
    simhash_band_2      smallint,
    simhash_band_3      smallint,

    page_count          int,
    cover_phash         bigint,
    -- Normalised title, under pg_trgm. Not nullable: every listing has a
    -- title, and a title is the one layer every source carries.
    title_norm          text        NOT NULL,

    -- Which machine reported this. NULL where no machine did: a spreadsheet
    -- row's sketch is computed from the bytes the seller uploaded to us, and
    -- naming a device for it would be the conflation 0052 exists to prevent.
    observed_by_device  text,
    observed_at         timestamptz NOT NULL,
    recorded_at         timestamptz NOT NULL,

    PRIMARY KEY (org_id, product_id, fingerprint_version),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),
    FOREIGN KEY (org_id, observed_by_device) REFERENCES device (org_id, id),

    -- The text layer is one measurement: present together or absent together,
    -- for 0053's reason.
    CONSTRAINT product_fingerprint_text_total CHECK (
        (text_simhash IS NULL AND text_minhash IS NULL
            AND shingle_count IS NULL AND extracted_chars IS NULL
            AND simhash_band_0 IS NULL AND simhash_band_1 IS NULL
            AND simhash_band_2 IS NULL AND simhash_band_3 IS NULL)
     OR (text_simhash IS NOT NULL AND text_minhash IS NOT NULL
            AND shingle_count IS NOT NULL AND extracted_chars IS NOT NULL
            AND simhash_band_0 IS NOT NULL AND simhash_band_1 IS NOT NULL
            AND simhash_band_2 IS NOT NULL AND simhash_band_3 IS NOT NULL)
    ),
    CONSTRAINT product_fingerprint_minhash_width CHECK (
        text_minhash IS NULL OR octet_length(text_minhash) = 512
    ),
    CONSTRAINT product_fingerprint_counts CHECK (
        (shingle_count IS NULL OR shingle_count >= 0)
    AND (extracted_chars IS NULL OR extracted_chars >= 0)
    AND (page_count IS NULL OR page_count >= 0)
    ),
    -- Bounded above and not below. A title of nothing but punctuation
    -- normalises to nothing, and the empty string is the honest answer to
    -- "what does this title say once boilerplate is stripped": refusing the
    -- row would cost the sketch its other layers over a title the matcher was
    -- never going to earn anything from.
    CONSTRAINT product_fingerprint_title_norm_bounded CHECK (
        char_length(title_norm) <= 400
    )
);

CREATE INDEX product_fingerprint_band_0 ON product_fingerprint (org_id, simhash_band_0);
CREATE INDEX product_fingerprint_band_1 ON product_fingerprint (org_id, simhash_band_1);
CREATE INDEX product_fingerprint_band_2 ON product_fingerprint (org_id, simhash_band_2);
CREATE INDEX product_fingerprint_band_3 ON product_fingerprint (org_id, simhash_band_3);
CREATE INDEX product_fingerprint_title_trgm
    ON product_fingerprint USING gin (title_norm gin_trgm_ops);

-- What was asked about one pair, and what was answered.
--
-- The pattern is `binding_candidate`'s and `field_mismatch`'s, which
-- 0004_mapping.sql already establishes: a decision the machinery could not
-- take alone, with positioned, classified evidence beside it.
CREATE TABLE duplicate_verdict (
    org_id              uuid        NOT NULL REFERENCES organisation (id),
    -- The unordered pair, made an order by construction rather than by
    -- convention: the lesser uuid is always `product_lo`, so one pair is one
    -- row and "we asked already" is a primary-key lookup.
    --
    -- Neither side carries a foreign key, for `import_batch_row.product_id`'s
    -- reason: a side may be an `import_run_item`'s reserved identifier, whose
    -- product exists only after the commit. A `different` answer about a pair
    -- one side of which was never created is still the answer, and is the
    -- whole point of storing it.
    product_lo          uuid        NOT NULL,
    product_hi          uuid        NOT NULL,
    -- `different` is stored because without it the next import re-asks a
    -- question the seller answered. `parked` is "decide later" and never
    -- blocks an import.
    verdict             text        NOT NULL,
    -- Whose claim this is, in the voice `product_file_observation` uses.
    decided_by          text        NOT NULL,
    -- The layer that carried the decision, so the console's one sentence is
    -- generated from a stored fact rather than re-scored on read.
    winning_layer       text        NOT NULL,
    -- The fused score at decision time, kept as evidence and never shown.
    log_odds            real        NOT NULL,
    fingerprint_version smallint    NOT NULL,
    -- Which run raised it, so a run's review queue is one index read.
    run_id              uuid,
    -- The merge survivor, on a `same` verdict. The loser is tombstoned by
    -- `product.deleted_at` rather than deleted, which is what makes the
    -- thirty-day reversal possible.
    kept_product        uuid,
    raised_at           timestamptz NOT NULL,
    decided_at          timestamptz,
    -- When a `same` verdict stops being reversible. Stored rather than
    -- computed, for 0058's reason: it is the deadline the seller was told.
    reversible_until    timestamptz,

    PRIMARY KEY (org_id, product_lo, product_hi),
    FOREIGN KEY (org_id, run_id) REFERENCES import_run (org_id, id),

    CONSTRAINT duplicate_verdict_ordered CHECK (product_lo < product_hi),
    CONSTRAINT duplicate_verdict_verdict CHECK (verdict IN ('same', 'different', 'parked')),
    CONSTRAINT duplicate_verdict_decided_by CHECK (decided_by IN ('seller', 'system')),
    CONSTRAINT duplicate_verdict_layer CHECK (
        winning_layer IN ('l1', 'l1b', 'l2', 'l3', 'l4', 'l5')
    ),
    -- A parked pair is undecided; anything else states when.
    CONSTRAINT duplicate_verdict_decided_total CHECK (
        (verdict = 'parked') = (decided_at IS NULL)
    ),
    -- Only a merge names a survivor and a reversal window.
    CONSTRAINT duplicate_verdict_merge_total CHECK (
        (verdict = 'same') = (kept_product IS NOT NULL)
    AND (verdict = 'same') = (reversible_until IS NOT NULL)
    ),
    CONSTRAINT duplicate_verdict_kept_is_a_side CHECK (
        kept_product IS NULL OR kept_product IN (product_lo, product_hi)
    ),
    -- The asymmetry the research demands, as a database fact: the system may
    -- only ever decide on a decisive layer, and only `same`. Everything else
    -- is the seller's.
    CONSTRAINT duplicate_verdict_system_is_decisive CHECK (
        decided_by = 'seller'
     OR (verdict = 'same' AND winning_layer IN ('l1', 'l2'))
    )
);

CREATE INDEX duplicate_verdict_open ON duplicate_verdict (org_id, run_id)
    WHERE verdict = 'parked';

-- Positioned, classified evidence: `field_mismatch`'s own shape, one layer per
-- row.
CREATE TABLE duplicate_evidence (
    org_id      uuid NOT NULL REFERENCES organisation (id),
    product_lo  uuid NOT NULL,
    product_hi  uuid NOT NULL,
    position    int  NOT NULL,
    layer       text NOT NULL,
    -- So the page-count rule is storable as the strong negative it is rather
    -- than as an absent positive.
    polarity    text NOT NULL,
    -- The measured quantity, in the layer's own units, with the unit named
    -- beside it rather than six nullable columns.
    measure     real NOT NULL,
    unit        text NOT NULL,
    -- What the digest or value was, where naming it is what makes the seller's
    -- sentence concrete ("worksheet-pack.pdf, 2.4 MB").
    observed_in text,

    PRIMARY KEY (org_id, product_lo, product_hi, layer),
    FOREIGN KEY (org_id, product_lo, product_hi)
        REFERENCES duplicate_verdict (org_id, product_lo, product_hi) ON DELETE CASCADE,

    CONSTRAINT duplicate_evidence_position UNIQUE (org_id, product_lo, product_hi, position),
    CONSTRAINT duplicate_evidence_layer CHECK (
        layer IN ('l1', 'l1b', 'l2', 'l3', 'l4', 'l5')
    ),
    CONSTRAINT duplicate_evidence_polarity CHECK (polarity IN ('positive', 'negative')),
    CONSTRAINT duplicate_evidence_unit CHECK (
        unit IN ('jaccard', 'hamming', 'bytes', 'ratio', 'count')
    ),
    CONSTRAINT duplicate_evidence_observed_in_bounded CHECK (
        observed_in IS NULL OR char_length(observed_in) BETWEEN 1 AND 400
    )
);

-- The marketplace's own label, which the seller did not type and cannot
-- remove.
--
-- A column rather than a naming convention, and it does three things no
-- convention could. It keeps the label outside the per-product twenty and the
-- plan's vocabulary allowance, because the seller did not spend it. It
-- survives `set_for_product`, which is a replace: without this column the
-- first manual label edit on an imported resource would detach the
-- marketplace label and the sweep would delete it, so the auto-label would
-- last exactly until the seller used the feature it was there to help. And it
-- exempts the row from `Colour::of_name`, so `TPT` is one colour everywhere
-- rather than a hash of four characters.
ALTER TABLE label ADD COLUMN system boolean NOT NULL DEFAULT false;

ALTER TABLE import_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE import_run FORCE ROW LEVEL SECURITY;
CREATE POLICY import_run_org_isolation ON import_run
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE import_run_item ENABLE ROW LEVEL SECURITY;
ALTER TABLE import_run_item FORCE ROW LEVEL SECURITY;
CREATE POLICY import_run_item_org_isolation ON import_run_item
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE product_fingerprint ENABLE ROW LEVEL SECURITY;
ALTER TABLE product_fingerprint FORCE ROW LEVEL SECURITY;
CREATE POLICY product_fingerprint_org_isolation ON product_fingerprint
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE duplicate_verdict ENABLE ROW LEVEL SECURITY;
ALTER TABLE duplicate_verdict FORCE ROW LEVEL SECURITY;
CREATE POLICY duplicate_verdict_org_isolation ON duplicate_verdict
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE duplicate_evidence ENABLE ROW LEVEL SECURITY;
ALTER TABLE duplicate_evidence FORCE ROW LEVEL SECURITY;
CREATE POLICY duplicate_evidence_org_isolation ON duplicate_evidence
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);


-- The operator reads the run and nothing under it. An import's standing is a
-- support question -- "did their import finish" -- and the resources it
-- described, the sketches of their files and the duplicate questions their
-- seller was asked are not. That asymmetry is why the backoffice grant is
-- enumerated per table rather than given as BYPASSRLS, per 0037.
GRANT SELECT ON import_run TO tam_backoffice;
CREATE POLICY import_run_backoffice_read ON import_run
    FOR SELECT
    TO tam_backoffice
    USING (true);
