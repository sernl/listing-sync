-- Per-tenant encryption for object-store blobs. The design reinstates
-- per-tenant data-encryption keys for the resource-file blobs specifically,
-- because the files are the asset and, unlike a credential, cannot be rotated
-- after disclosure. dek_key_version already exists; these columns carry the
-- wrapped DEK, the payload nonce and the AAD context so a blob object lifted
-- into another tenant fails authentication rather than decrypting. Nullable
-- for the not-yet-encrypted rows M1b's ProductRepo wrote with version 0; the
-- pipeline writes them non-null.
ALTER TABLE blob ADD COLUMN wrapped_dek bytea;
ALTER TABLE blob ADD COLUMN nonce       bytea;
ALTER TABLE blob ADD COLUMN aad         bytea;

-- The pipeline worker writes and reads blob rows and their bytes; it runs on
-- the engine role today (the pipeline and job engine share a worker host in
-- M1). No DELETE — a superseded blob is left for the retention pass, never
-- erased under a live product.
GRANT SELECT, INSERT, UPDATE ON blob TO tam_engine;
