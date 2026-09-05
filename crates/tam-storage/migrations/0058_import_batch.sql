-- A spreadsheet the seller filled, parsed and held before anything is created.
--
-- The founder's ask is a template workbook with one tab per marketplace, filled
-- and uploaded, every row becoming a draft unless its status says live. What
-- makes it two tables rather than one stored document is the three things a
-- single document cannot do: cite the seller's own spreadsheet row number in a
-- refusal, let a resumed commit skip what it already created, and retry one row
-- rather than the batch. The last of those is migration 0025's own hazard aimed
-- at this path -- `import_one` mints a fresh product id on every pass, so a
-- re-run without a per-row breadcrumb inserts a second product no unique index
-- refuses -- and the breadcrumb here is `product_id` on the row, exactly as
-- `sync_request_resource` carries one.
--
-- Nothing here creates anything. A parsed batch holds rows and refusals; the
-- products, the labels and the jobs are a later, deliberate commit.

CREATE TABLE import_batch (
    org_id         uuid        NOT NULL REFERENCES organisation (id),
    -- The submit's own idempotency key, so a double-clicked upload is one
    -- batch rather than two, following `job` and `sync_request`.
    id             uuid        NOT NULL,
    -- The uploaded filename, so the listing says which sheet this was.
    source_name    text        NOT NULL,
    state          text        NOT NULL,
    row_count      int         NOT NULL,
    live_count     int         NOT NULL,
    -- How many rows the parse refused. Derivable by counting rows, and stored
    -- anyway: the listing renders one line per batch and the refused count is
    -- the primary fact of a parsed one, so deriving it would put a per-batch
    -- aggregate inside the list route's own page walk.
    failed_count   int         NOT NULL,
    created_at     timestamptz NOT NULL,
    -- When an unsettled batch is swept and its uploaded bytes deleted.
    --
    -- A column rather than `created_at` plus a constant read at render time.
    -- The console states this deadline to the seller, and a stored instant is
    -- the deadline they were told; a computed one silently moves every batch
    -- in the table the day the constant moves, including batches whose
    -- deadline a seller has already read.
    expires_at     timestamptz NOT NULL,
    settled_at     timestamptz,
    failure_detail text,

    PRIMARY KEY (org_id, id),

    CONSTRAINT import_batch_source_name_bounded CHECK (
        char_length(source_name) BETWEEN 1 AND 260
    ),
    CONSTRAINT import_batch_state CHECK (
        state IN ('parsed', 'attaching', 'importing', 'imported', 'failed', 'abandoned')
    ),
    CONSTRAINT import_batch_counts_non_negative CHECK (
        row_count >= 0 AND live_count >= 0 AND failed_count >= 0
    ),
    CONSTRAINT import_batch_counts_within_rows CHECK (
        live_count <= row_count AND failed_count <= row_count
    ),
    -- 0025's totality constraint: a settled batch states when, and an
    -- unsettled one states nothing.
    CONSTRAINT import_batch_settled_total CHECK (
        (state IN ('parsed', 'attaching', 'importing')) = (settled_at IS NULL)
    ),
    CONSTRAINT import_batch_failure_detail CHECK (
        (failure_detail IS NULL) OR state IN ('failed', 'abandoned')
    ),
    CONSTRAINT import_batch_expires_after_created CHECK (expires_at > created_at)
);

-- One open batch per organisation, as a partial unique index rather than as a
-- count the route takes before inserting.
--
-- The rule exists because two batches attaching files matched by filename
-- would let the same file bind to rows in both, with nothing on screen telling
-- the seller which is which. A route-side count is racy against a
-- double-submit under two idempotency keys; this index is the rule made true
-- of any writer, and the route reads its violation as the answer "you already
-- have one open" rather than as a fault.
CREATE UNIQUE INDEX import_batch_one_open_per_org
    ON import_batch (org_id)
    WHERE state IN ('parsed', 'attaching', 'importing');

-- The listing's own order, newest first, and the sweep's scan.
CREATE INDEX import_batch_by_age ON import_batch (org_id, created_at DESC, id);

CREATE TABLE import_batch_row (
    org_id         uuid        NOT NULL REFERENCES organisation (id),
    batch_id       uuid        NOT NULL,
    -- The workbook tab this row came off, as the tab is titled.
    sheet          text        NOT NULL,
    -- The seller's own spreadsheet row number, so a refusal cites what they
    -- see rather than a zero-based index into a parse.
    ordinal        int         NOT NULL,
    -- Null on the Teachouse tab, which is a product that lives only here and
    -- names no inventory. Unconstrained by a foreign key for the reason
    -- `sync_request.source` gives: the repository's own round trip through
    -- InventoryId is what admits a value.
    inventory      text,
    intent         text        NOT NULL,
    -- The parsed row as the create form's own shape, following the discipline
    -- migration 0056 states for `resource_template.draft`: the field set is
    -- TPT's and moves when TPT's does, the document is never joined against
    -- and never read by the engine, and what keeps it honest is that the route
    -- validates it through the create form's own value rules before the column
    -- sees it. The ceiling and its arithmetic are 0056's, restated for the
    -- same reason and against the same route-side bound.
    draft          jsonb       NOT NULL,
    -- Every refusal the parse raised against this row, as
    -- `[{"column": ..., "problem": ...}]`. The report is the deliverable
    -- rather than a byproduct, and it has to survive a refresh: the seller
    -- reads it on the batch page after closing the tab the upload happened in.
    -- Warnings are not stored here and are derived when the batch is read,
    -- because both of them -- a label the sheet would newly create, and a live
    -- row on a marketplace with no connection -- are answers about the state of
    -- the world now rather than at parse time.
    problems       jsonb       NOT NULL,
    -- The filename the seller's `File` column named, before any file exists.
    file_name      text,
    -- The bytes bound to this row, as the upload's own handle: content hash,
    -- probed kind and length. Three columns rather than one identifier,
    -- because there is no file row to point at -- a `product_file` exists only
    -- after the commit creates the product -- and the create this row will
    -- make takes exactly the handle `POST /{version}/uploads` returned.
    --
    -- The foreign key is what makes "bytes this organisation uploaded" true of
    -- the column rather than only of the route: per-tenant dedup keys `blob` on
    -- (org_id, hash), so a row cannot name a blob another tenant holds.
    file_hash      bytea,
    file_kind      text,
    file_byte_len  bigint,
    state          text        NOT NULL,
    product_id     uuid,
    mapping_id     uuid,
    job_id         uuid,
    failure_detail text,

    PRIMARY KEY (org_id, batch_id, sheet, ordinal),
    FOREIGN KEY (org_id, batch_id) REFERENCES import_batch (org_id, id),
    FOREIGN KEY (org_id, file_hash) REFERENCES blob (org_id, hash),

    CONSTRAINT import_batch_row_sheet_bounded CHECK (
        char_length(sheet) BETWEEN 1 AND 64
    ),
    CONSTRAINT import_batch_row_ordinal_positive CHECK (ordinal > 0),
    CONSTRAINT import_batch_row_intent CHECK (intent IN ('draft', 'live')),
    CONSTRAINT import_batch_row_state CHECK (
        state IN ('parsed', 'attached', 'created', 'published', 'failed', 'skipped')
    ),
    CONSTRAINT import_batch_row_draft_is_object CHECK (
        jsonb_typeof(draft) = 'object'
    ),
    CONSTRAINT import_batch_row_draft_bounded CHECK (
        octet_length(draft::text) <= 131072
    ),
    CONSTRAINT import_batch_row_problems_is_array CHECK (
        jsonb_typeof(problems) = 'array'
    ),
    CONSTRAINT import_batch_row_problems_bounded CHECK (
        octet_length(problems::text) <= 131072
    ),
    CONSTRAINT import_batch_row_file_name_bounded CHECK (
        file_name IS NULL OR char_length(file_name) BETWEEN 1 AND 260
    ),
    -- A handle is three facts or none. A hash without its length would make
    -- the create guess at a number `POST /{version}/products` checks against
    -- the stored blob.
    CONSTRAINT import_batch_row_file_handle_total CHECK (
        (file_hash IS NULL) = (file_kind IS NULL)
        AND (file_hash IS NULL) = (file_byte_len IS NULL)
    ),
    CONSTRAINT import_batch_row_file_byte_len_non_negative CHECK (
        file_byte_len IS NULL OR file_byte_len >= 0
    ),
    -- A live row past creation names the bytes it published. Stated for rows
    -- that reached `created`, not for a parsed one: the sheet names a filename
    -- and the seller attaches the bytes afterwards, so a live row without a
    -- handle is the ordinary state of an attaching batch.
    CONSTRAINT import_batch_row_live_creation_names_a_file CHECK (
        intent <> 'live'
        OR state NOT IN ('created', 'published')
        OR file_hash IS NOT NULL
    ),
    -- 0025's totality constraint, split because a Teachouse row names no
    -- inventory and so gets no mapping: a created row names a product always,
    -- and names a mapping exactly when it named an inventory.
    CONSTRAINT import_batch_row_created_total CHECK (
        (state IN ('created', 'published')) = (product_id IS NOT NULL)
    ),
    CONSTRAINT import_batch_row_mapping_follows_inventory CHECK (
        (product_id IS NOT NULL AND inventory IS NOT NULL) = (mapping_id IS NOT NULL)
    ),
    -- A refused row is failed. The converse does not hold: a commit-time
    -- failure names a `failure_detail` and raised no parse refusal at all.
    CONSTRAINT import_batch_row_problems_imply_failed CHECK (
        jsonb_array_length(problems) = 0 OR state = 'failed'
    ),
    CONSTRAINT import_batch_row_failure_detail CHECK (
        (failure_detail IS NULL) OR state = 'failed'
    )
);

ALTER TABLE import_batch ENABLE ROW LEVEL SECURITY;
ALTER TABLE import_batch FORCE ROW LEVEL SECURITY;
CREATE POLICY import_batch_org_isolation ON import_batch
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE import_batch_row ENABLE ROW LEVEL SECURITY;
ALTER TABLE import_batch_row FORCE ROW LEVEL SECURITY;
CREATE POLICY import_batch_row_org_isolation ON import_batch_row
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- The expiry sweep is cross-tenant by construction, like the job-event pruner
-- and the snapshot retention pass beside it: one pass covers every
-- organisation, so it runs on the BYPASSRLS engine role and sees nothing at all
-- under tam_app's forced row-level security.
--
-- SELECT travels with UPDATE because the pass picks its batch before settling
-- it, and the grant stops there. No INSERT, because the engine never parses a
-- sheet, and no DELETE, because the sweep settles a batch to `abandoned` and
-- tells the seller rather than erasing the record of what they uploaded. The
-- row grant is the same pair for the same pass: the sweep clears the file
-- handle it releases and leaves the parsed row standing.
--
-- The design that specified these tables said neither would be granted to
-- tam_engine, on the reasoning that the engine never reads a batch. That was
-- written before the founder approved the expiry sweep, which is the one
-- cross-tenant pass over this data and cannot run under a role that sees one
-- tenant at a time. This is the narrowest grant that sweep performs.
GRANT SELECT, UPDATE ON import_batch     TO tam_engine;
GRANT SELECT, UPDATE ON import_batch_row TO tam_engine;

-- No backoffice grant and no backoffice policy, for the reason migration 0056
-- gives for `resource_template`: this is listing copy the seller has not
-- published anywhere, no operator route reads it, and cross-tenant reach over
-- every tenant's private drafting with no reader is privilege granted against
-- a need nobody has stated. The day an operator surface needs it is the day to
-- add it, in migration 0037's two halves, with the route that reads it.
