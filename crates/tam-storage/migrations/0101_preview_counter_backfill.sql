-- Watermarked previews made before migration 0098 started counting them.
--
-- 0098 began every organisation's preview counter at zero, so a seller who
-- made previews earlier in the month the counter shipped in, or in any month
-- before it, read "0 watermarked previews" beside previews they could see on
-- their resources. The Billing page's figure has to be the record of what
-- was made, and from this release Look's allowance is five previews for the
-- account's whole life, summed over every month's row, so the months before
-- 0098 are part of that figure too.
--
-- The record is `product_file`: every preview a resource gained is a row
-- there, and a removed one keeps its row with `deleted_at` set, which is
-- exactly the "made, even if deleted" the counter counts. Only files whose
-- bytes we hold (`hash IS NOT NULL`) are counted: a preview a marketplace
-- import points at was made on the marketplace, never through us, and no gate
-- ever spent one for it.
--
-- GREATEST rather than a sum or an overwrite, so this never lowers a month
-- the gate has already counted (a swapped preview spends one and leaves one
-- row, so the counter can be ahead of the rows) and a second run changes
-- nothing.
--
-- Migrations run as `tam_app`, which owns both tables under FORCE ROW LEVEL
-- SECURITY with no tenant pinned, so the fence is lifted for the length of
-- this transaction and restored below: `0099_device_streams.sql`'s pattern,
-- for its reason. Without it the SELECT sees no rows and the backfill is a
-- silent no-op.
ALTER TABLE product_file NO FORCE ROW LEVEL SECURITY;
ALTER TABLE usage_counter NO FORCE ROW LEVEL SECURITY;

INSERT INTO usage_counter (org_id, kind, month, used, updated_at)
SELECT org_id,
       'preview',
       date_trunc('month', created_at AT TIME ZONE 'UTC')::date,
       count(*)::int,
       now()
  FROM product_file
 WHERE role = 'preview'
   AND hash IS NOT NULL
 GROUP BY org_id, date_trunc('month', created_at AT TIME ZONE 'UTC')::date
ON CONFLICT (org_id, kind, month) DO UPDATE
   SET used = EXCLUDED.used,
       updated_at = EXCLUDED.updated_at
 WHERE usage_counter.used < EXCLUDED.used;

ALTER TABLE product_file FORCE ROW LEVEL SECURITY;
ALTER TABLE usage_counter FORCE ROW LEVEL SECURITY;
