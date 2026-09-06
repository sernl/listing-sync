-- The cover an imported row carries, and the state a claimed row sits in
-- between reserving its identifiers and holding a product.
--
-- Both are the commit's, and both are written here rather than in 0058 because
-- that migration is applied and frozen: a constraint that has to admit a new
-- value is dropped and restated, never edited in place.

-- `POST /{version}/uploads` generates a cover during the upload and answers it
-- beside the payload, and `POST /{version}/products` writes it onto the
-- product. A row that carried only its payload would therefore create a
-- resource with no thumbnail, which is a listing the seller has to go and fix
-- one at a time -- the whole of what a bulk import exists to avoid.
--
-- Three columns and the same foreign key the payload handle carries, for
-- 0058's own reason: there is no file row to point at until the commit creates
-- the product, and the key is what makes "bytes this organisation uploaded"
-- true of the column rather than only of the route.
ALTER TABLE import_batch_row ADD COLUMN cover_hash     bytea;
ALTER TABLE import_batch_row ADD COLUMN cover_kind     text;
ALTER TABLE import_batch_row ADD COLUMN cover_byte_len bigint;

ALTER TABLE import_batch_row
    ADD CONSTRAINT import_batch_row_cover_blob_held
    FOREIGN KEY (org_id, cover_hash) REFERENCES blob (org_id, hash);

ALTER TABLE import_batch_row
    ADD CONSTRAINT import_batch_row_cover_handle_total CHECK (
        (cover_hash IS NULL) = (cover_kind IS NULL)
        AND (cover_hash IS NULL) = (cover_byte_len IS NULL)
    );

ALTER TABLE import_batch_row
    ADD CONSTRAINT import_batch_row_cover_byte_len_non_negative CHECK (
        cover_byte_len IS NULL OR cover_byte_len >= 0
    );

-- The state a row holds while its product is being created.
--
-- The commit reserves `product_id` and `mapping_id` before it creates
-- anything, so that a pass resumed after a closed browser reads whether the
-- product it already reserved exists and either records the breadcrumb or
-- re-runs the create under the same identifier. Without the reservation the
-- only guard is a breadcrumb written after the create, which is exactly the
-- hazard 0058's own header names: a second charged product in the seller's
-- catalogue for one spreadsheet row. `creating` is what a reserved identifier
-- needs a state for, because `import_batch_row_created_total` ties the
-- identifier to the state and a reserved one is not yet created.
ALTER TABLE import_batch_row DROP CONSTRAINT import_batch_row_state;
ALTER TABLE import_batch_row
    ADD CONSTRAINT import_batch_row_state CHECK (
        state IN ('parsed', 'attached', 'creating', 'created', 'published', 'failed', 'skipped')
    );

ALTER TABLE import_batch_row DROP CONSTRAINT import_batch_row_created_total;
ALTER TABLE import_batch_row
    ADD CONSTRAINT import_batch_row_created_total CHECK (
        (state IN ('creating', 'created', 'published')) = (product_id IS NOT NULL)
    );

-- D32's rule reaches the claim as well as the create: a live row whose
-- identifiers are reserved is a row the next statement creates, and it names
-- the bytes it will publish by then or the claim itself was wrong.
ALTER TABLE import_batch_row DROP CONSTRAINT import_batch_row_live_creation_names_a_file;
ALTER TABLE import_batch_row
    ADD CONSTRAINT import_batch_row_live_creation_names_a_file CHECK (
        intent <> 'live'
        OR state NOT IN ('creating', 'created', 'published')
        OR file_hash IS NOT NULL
    );
