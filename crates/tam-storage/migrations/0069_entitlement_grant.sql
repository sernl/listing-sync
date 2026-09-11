-- What an organisation is entitled to, as an append-only record of grants
-- rather than a column on the tenant root.
--
-- A column would carry a plan and nothing else. Every grant this product
-- makes carries more than a plan: a Paddle purchase carries the rung bought
-- and the subscription or transaction it came from, a backoffice grant
-- carries the operator who made it, their reason and an optional expiry, and
-- both carry a revocation that has to stay visible after it happens. A
-- column would have needed an audit table beside it within the week, and the
-- plan would then have been stored twice.
--
-- So the plan an organisation holds is a query rather than a value: the
-- strongest grant among the rows that are neither revoked nor expired, and
-- `free` where there is none. `crates/tam-storage/src/entitlement.rs` is the
-- only reader, and `Plan::strength` in tam-limits is the ordering.
--
-- plan carries a CHECK restating the closed set, unlike
-- billing_subscription.status, which deliberately does not. The difference is
-- whose vocabulary it is: Paddle owns its status words and versions them
-- without telling us, so an unseen value must land rather than abort a
-- webhook; this column's vocabulary is ours, enumerated in one Rust enum, and
-- a value outside it is a bug in our own writer that should fail at the
-- statement rather than silently grant an unreadable plan.
--
-- rung is null for every plan except migration_only, where it is the ladder
-- step bought and therefore both the resource allowance and the one-off
-- migration pass. Not CHECKed against the ladder's five values: the ladder is
-- a price list that moves, an operator may grant a volume no rung sells, and
-- a constraint restating a price list would make a pricing change a schema
-- change.
--
-- granted_by is closed at two values because there are exactly two ways a
-- grant comes to exist, and they are audited differently. A 'paddle' row is
-- attributable to source_ref -- the subscription or transaction id -- and to
-- nobody here; an 'operator' row is attributable to grantor_user and to the
-- reason they typed. Neither column is NOT NULL, because neither is true of
-- both kinds, and a constraint pinning which is which would encode the
-- attribution rule in two places.
--
-- expires_at null means "until revoked", which is what a one-off purchase
-- holds: the thirty-day edit window a Catalogue Import buyer gets is a
-- capability derived from the plan, not the life of the grant, so expiring
-- the row would take the buyer's catalogue away rather than making it
-- read-only. A subscription's row carries its period end plus a grace, so a
-- webhook that never arrives lapses the plan on its own rather than granting
-- it forever.
--
-- revoked_at is a column rather than a delete, because "this operator granted
-- Studio for a week and took it back" is exactly the fact a backoffice audit
-- trail exists to answer.
CREATE TABLE entitlement_grant (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    id           uuid        NOT NULL,
    plan         text        NOT NULL,
    rung         int,
    granted_by   text        NOT NULL,
    grantor_user uuid,
    reason       text,
    source_ref   text,
    granted_at   timestamptz NOT NULL,
    expires_at   timestamptz,
    revoked_at   timestamptz,

    PRIMARY KEY (org_id, id),

    CONSTRAINT entitlement_grant_plan_known CHECK (
        plan IN ('free', 'subscriber', 'migration_only', 'studio')
    ),

    CONSTRAINT entitlement_grant_granted_by_known CHECK (
        granted_by IN ('paddle', 'operator')
    ),

    CONSTRAINT entitlement_grant_rung_positive CHECK (
        rung IS NULL OR rung > 0
    ),

    CONSTRAINT entitlement_grant_reason_said CHECK (
        reason IS NULL OR reason <> ''
    )
);

-- The read this table exists for runs on every authenticated request: the
-- OrgContext extractor asks for one organisation's unexpired grants before
-- any handler runs. Partial on the revocation, because a revoked grant is
-- never part of that answer and a backoffice history read is rare enough to
-- scan for.
CREATE INDEX entitlement_grant_live
    ON entitlement_grant (org_id, granted_at DESC)
    WHERE revoked_at IS NULL;

-- One live Paddle grant per subscription or transaction, across every
-- tenant, for the reason billing_subscription's own unique index crosses the
-- fence: Paddle's identifier names one thing in Paddle's ledger, and a
-- replayed notification carrying somebody else's custom_data must collide
-- rather than quietly entitle a second organisation. Partial so a revoked
-- grant does not block a later re-grant of the same subscription, which is
-- what a resumed subscription looks like.
CREATE UNIQUE INDEX entitlement_grant_paddle_source_unique
    ON entitlement_grant (source_ref)
    WHERE granted_by = 'paddle' AND source_ref IS NOT NULL AND revoked_at IS NULL;

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form: a pooled connection reverts a transaction-local
-- app.current_org to the empty string rather than to missing, so NULLIF is
-- what keeps an unpinned statement matching nothing instead of raising 22P02.
ALTER TABLE entitlement_grant ENABLE ROW LEVEL SECURITY;
ALTER TABLE entitlement_grant FORCE ROW LEVEL SECURITY;
CREATE POLICY entitlement_grant_org_isolation ON entitlement_grant
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- Both halves of the operator's reach, for the reason migration 0037 states:
-- tam_backoffice is not BYPASSRLS, so a grant alone shows it nothing on a
-- fenced table and a policy alone gives it nothing to read.
--
-- SELECT only, even though this is the first table the operator surface
-- writes. The write goes through the application pool with the target
-- organisation pinned, exactly as a tenant write does; giving the backoffice
-- role INSERT here would create a second, unfenced way to change what a
-- tenant holds, and the one thing that makes every other handler safe is
-- that no such connection exists.
GRANT SELECT ON entitlement_grant TO tam_backoffice;

CREATE POLICY entitlement_grant_backoffice_read ON entitlement_grant
    FOR SELECT TO tam_backoffice USING (true);
