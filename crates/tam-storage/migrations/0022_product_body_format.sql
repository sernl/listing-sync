-- How a listing body is written, declared rather than sniffed. Guessing a
-- body's format from its bytes is how a listing acquires escaped markup nobody
-- asked for, and the TPT write model already ruled that out; carrying the
-- declaration means a projection whose source and target disagree refuses
-- instead of corrupting.

-- Expand, backfill, switch, contract, as 0019 and 0020 did. 'markdown' is the
-- correct backfill because every product on file was imported from Tes, whose
-- draft body is posted with descriptionRawType "md"; the first HTML body
-- arrives with the TPT import and declares itself.
ALTER TABLE product ADD COLUMN body_format text NOT NULL DEFAULT 'markdown';
ALTER TABLE product ALTER COLUMN body_format DROP DEFAULT;

ALTER TABLE product ADD CONSTRAINT product_body_format CHECK (
    body_format IN ('markdown', 'html')
);
