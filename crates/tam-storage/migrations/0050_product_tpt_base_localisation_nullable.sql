-- The localisation flag gains a third state: not stated.
--
-- 0049 added appropriate_for_country as boolean NOT NULL DEFAULT false, on the
-- reading that false is what an unticked checkbox posts. That reading holds for
-- a seller who answered and fails for a row that never was asked: every row
-- written before 0049 took the default, so a backfilled row and a seller's
-- deliberate "no" are the same value and nothing downstream can tell them
-- apart.
--
-- The engine is what needs them apart. TPT's edit is a full replace, so the
-- adapter reads the product's current flag back and reposts it whenever the
-- projection states nothing; it suppresses that read on any stated value. Fed
-- a backfilled false, it would post 0 over a box the seller ticked on TPT,
-- which is the silent clear the read-back exists to prevent. So the column
-- becomes nullable, NULL means the seller has not answered, and the sidecar
-- can be fed to the projection.
--
-- Every existing row is set to NULL, and nothing is lost by it: no value in
-- this column was ever a seller's answer. The column was added by 0049 and the
-- form has never written anything but the checkbox's own state through a path
-- that would have overwritten the default anyway, so what is being discarded
-- is the backfill and only the backfill.
--
-- The tenant fence has to be lifted for the UPDATE and is restored in the same
-- statement group. product_tpt_base carries FORCE ROW LEVEL SECURITY, which
-- applies the org policy to the table's owner, and a migration connection sets
-- no app.current_org; the UPDATE would therefore match no row and leave every
-- backfilled false in place without reporting anything. sqlx runs a migration
-- inside a transaction, so a failure between the two ALTERs rolls the fence
-- back on with the rest of the file.

ALTER TABLE product_tpt_base
    ALTER COLUMN appropriate_for_country DROP DEFAULT,
    ALTER COLUMN appropriate_for_country DROP NOT NULL;

ALTER TABLE product_tpt_base NO FORCE ROW LEVEL SECURITY;
UPDATE product_tpt_base SET appropriate_for_country = NULL;
ALTER TABLE product_tpt_base FORCE ROW LEVEL SECURITY;
