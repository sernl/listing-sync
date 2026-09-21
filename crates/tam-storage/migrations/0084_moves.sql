-- Moves: the one metered currency, as an append-only ledger rather than a
-- counter on the tenant root.
--
-- A move is one resource committed to the other marketplace, counted at
-- commit after duplicate merge, whether it lands as a draft or live. Until
-- now the meter was a monthly count derived on the fly from `sync_request`,
-- and the thing a seller bought was a plan called `migration_only` whose
-- `rung` was simultaneously a resource ceiling and a one-off allowance. The
-- 2026-09-20 pricing model separates those: a pack is a balance of moves on
-- whatever plan the buyer already holds, so the balance needs somewhere to
-- live that a plan row cannot express.
--
-- A column would have been wrong for the reason 0069 gives about plans. A
-- balance carries more than a number: where each part of it came from, when
-- that part expires, and which commit spent it. All four are needed to answer
-- "why do I have eleven moves left", which is the only question this table
-- exists to answer, and a single integer answers none of them.
--
-- So the balance is a query: SUM(delta) over the rows that have not expired.
--
-- The sign convention is the source's. 'free', 'pack', 'subscription' and
-- 'founding' credit; 'commit' and 'refund' debit; 'operator' may do either,
-- because an operator correcting a mistake has to be able to correct it in
-- both directions. The CHECK states exactly that, so a writer that gets a
-- sign backwards fails at the statement rather than quietly doubling
-- somebody's balance.
--
-- Expiry is the subtle part. A pack's moves expire twelve months after
-- purchase, and a debit recorded with no expiry would still be subtracted
-- from the balance long after the credit it drew against had gone --
-- eventually taking the balance negative for a seller who did nothing wrong.
-- So a debit carries the expiry of the credit it consumed: `debit_move` reads
-- the soonest-expiring unexpired credit and stamps the debit with that same
-- instant. Consumption is therefore first-expiring-first, stated in the data
-- rather than reconstructed by a job, and the whole balance stays one SUM.
--
-- source_ref is what makes every writer idempotent, and the unique index
-- below is what enforces it: a commit names its job item, a monthly accrual
-- names its period, a purchase names the processor's session, and a
-- storefront's free grant names the storefront. A webhook delivered twice
-- collides rather than crediting twice.
CREATE TABLE move_ledger (
    org_id     uuid        NOT NULL REFERENCES organisation (id),
    id         uuid        NOT NULL,
    delta      int         NOT NULL,
    source     text        NOT NULL,
    source_ref text,
    expires_at timestamptz,
    created_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT move_ledger_source_known CHECK (
        source IN ('free', 'pack', 'subscription', 'founding', 'operator', 'commit', 'refund')
    ),

    CONSTRAINT move_ledger_delta_moves CHECK (delta <> 0),

    -- Each source's direction, in the one place a writer can be wrong about
    -- it. 'operator' is deliberately absent from both lists.
    CONSTRAINT move_ledger_delta_matches_its_source CHECK (
        (source IN ('free', 'pack', 'subscription', 'founding') AND delta > 0)
        OR (source IN ('commit', 'refund') AND delta < 0)
        OR source = 'operator'
    ),

    CONSTRAINT move_ledger_source_ref_said CHECK (
        source_ref IS NULL OR source_ref <> ''
    )
);

-- The balance read, which the Account page, the migration preview and the
-- commit gate all make. Partial on nothing, because an expired row is still
-- part of the scan's predicate; the ordering column is the expiry so the
-- "expiring soonest" half of the same answer comes off the same index.
CREATE INDEX move_ledger_balance
    ON move_ledger (org_id, expires_at);

-- One entry per thing that happened. A replayed webhook, a retried commit
-- and a second accrual for the same period all land here and collide.
-- Scoped to the organisation as well as the source, because two tenants'
-- monthly accruals name the same period string and neither is the other's.
CREATE UNIQUE INDEX move_ledger_source_ref_unique
    ON move_ledger (org_id, source, source_ref)
    WHERE source_ref IS NOT NULL;

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form: a pooled connection reverts a transaction-local
-- app.current_org to the empty string rather than to missing, so NULLIF is
-- what keeps an unpinned statement matching nothing instead of raising 22P02.
ALTER TABLE move_ledger ENABLE ROW LEVEL SECURITY;
ALTER TABLE move_ledger FORCE ROW LEVEL SECURITY;
CREATE POLICY move_ledger_org_isolation ON move_ledger
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No grant to tam_backoffice. The operator surface reads and writes a
-- balance through the application pool with the target organisation pinned,
-- exactly as it already does for a plan grant, so this table adds no second
-- unfenced path across tenants.

-- Every live Catalogue Import becomes the credit it always meant.
--
-- The grant's own id is the ledger row's id and its source_ref, so the two
-- records point at each other and a second run of this migration collides on
-- the unique index rather than crediting twice. Twelve months from the
-- purchase, which is the pack validity the new model publishes; a purchase
-- older than that converts to a credit that has already expired, which is the
-- honest reading of an allowance the seller has had for over a year.
INSERT INTO move_ledger (org_id, id, delta, source, source_ref, expires_at, created_at)
SELECT org_id,
       id,
       rung,
       'pack',
       id::text,
       granted_at + interval '12 months',
       now()
  FROM entitlement_grant
 WHERE plan = 'migration_only'
   AND revoked_at IS NULL
   AND (expires_at IS NULL OR expires_at > now())
   AND rung IS NOT NULL
   AND rung > 0;

-- And the grant itself stops being a plan. Rewritten rather than deleted:
-- the row is the audit trail of a purchase, and what it was is written into
-- the reason so the backoffice history still says so after the vocabulary
-- closes over it. Revoked, because the entitlement it carried now lives in
-- the ledger and holding both would grant the volume twice.
UPDATE entitlement_grant
   SET plan = 'free',
       revoked_at = COALESCE(revoked_at, now()),
       reason = COALESCE(reason || ' ', '')
                || '(was a migration_only grant at rung '
                || COALESCE(rung::text, 'none')
                || '; converted to a move_ledger credit by migration 0084)'
 WHERE plan = 'migration_only';

-- The closed set, now that nothing holds the fourth spelling. `Plan` in
-- tam-limits is the other copy of this list, and the two are meant to be
-- read together.
ALTER TABLE entitlement_grant
    DROP CONSTRAINT entitlement_grant_plan_known;

ALTER TABLE entitlement_grant
    ADD CONSTRAINT entitlement_grant_plan_known CHECK (
        plan IN ('free', 'subscriber', 'studio')
    );
