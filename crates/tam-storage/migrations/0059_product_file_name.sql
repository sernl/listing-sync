-- What a seller called the file they uploaded.
--
-- A blob-backed `product_file` row carries a hash, a kind and a length, and no
-- name at all, because the upload route reads the request body as raw bytes and
-- a raw body has no filename. So the console renders three PDFs as three rows
-- differing only by their size, and a seller cannot tell which of them is the
-- worksheet. The name is the client's word for bytes it chose, carried back
-- alongside the handle rather than derived from anything the server probed.
--
-- Nullable and staying that way. Every row written before this migration has no
-- name and none can be invented for it: the bytes were sealed under a content
-- hash and the filename was never anywhere. The console says "unnamed file" for
-- those rather than leaving the row blank or guessing from the kind.
ALTER TABLE product_file ADD COLUMN name text;

-- The sourced arm already has its own name and must not acquire a second one.
--
-- `payload_file_name` on a marketplace-sourced row is the name the bytes are
-- handed onward under, and migration 0052 states the rule it follows. A `name`
-- beside it would be a second answer to the same question, and the two would
-- disagree the first time one of them was written and the other was not. So the
-- column is confined to the blob-backed arm by the same discipline
-- `product_file_blob_or_source` applies to every other column here: named in
-- both directions rather than left to a convention.
ALTER TABLE product_file
    ADD CONSTRAINT product_file_name_is_blob_backed CHECK (
        name IS NULL OR hash IS NOT NULL
    );

-- Bounded, because it is client-supplied text that reaches a marketplace
-- filename on the way out. 255 is the shortest limit any filesystem or
-- marketplace in this workspace's captures imposes, so a name that fits here
-- fits everywhere it is later used.
ALTER TABLE product_file
    ADD CONSTRAINT product_file_name_length CHECK (
        name IS NULL OR (char_length(name) > 0 AND char_length(name) <= 255)
    );
