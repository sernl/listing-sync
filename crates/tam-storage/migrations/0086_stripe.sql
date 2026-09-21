-- The billing rail moves from Paddle to Stripe, and the schema stops naming
-- a vendor where it never needed to.
--
-- Three tables are touched and one is added. Nothing here decides an
-- entitlement differently; what changes is whose identifier a column holds
-- and what vocabulary a CHECK admits.
--
-- granted_by gains 'stripe'. It is a widening rather than a rename because
-- the rows already written under 'paddle' are real grants with real
-- attribution, and rewriting them would restate history to match a decision
-- taken after it. 0069's argument for the closed set stands: this column's
-- vocabulary is ours, enumerated in one Rust enum, and a value outside it is
-- a bug in our own writer.
--
-- The partial unique index on source_ref was keyed on granted_by = 'paddle',
-- which is exactly the shape that stops protecting anything the moment a
-- second vendor writes: a Stripe session id would have collided with nothing
-- and double-granted on every retried delivery. It becomes vendor-agnostic —
-- any non-null source_ref on a live grant is unique across the deployment —
-- which is the property that was meant all along. An operator grant carries
-- no source_ref and so is unaffected; the partial index skips nulls.
--
-- billing_subscription's two vendor-named columns become provider-named, and
-- a provider column records which rail wrote the row. The rename is a rename
-- rather than a new table: the row is one per organisation, the ordering
-- fence on occurred_at is unchanged, and a parallel table would have meant
-- two places to ask "is this tenant paying".
--
-- provider_price_id is new and nullable. It is what lets the billing page
-- answer the cadence a seller is on and whether they are a founding member
-- without a second read of Stripe: the price map already says what a price
-- identifier sells, and the identifier is the one fact the subscription
-- events carry that the row did not keep.
--
-- service_booking is new. "Move with me" is a booking, not an entitlement: it
-- buys 45 minutes of the founder's time and grants no capability, so
-- recording it as a grant would have made the entitlement query answer a
-- question about a calendar. provider_ref is Stripe's checkout session id and
-- is unique across every tenant for the reason billing_subscription's own
-- unique constraint crosses the fence -- one session is one purchase, and a
-- replayed delivery must collide rather than book twice.

ALTER TABLE entitlement_grant
    DROP CONSTRAINT entitlement_grant_granted_by_known;

ALTER TABLE entitlement_grant
    ADD CONSTRAINT entitlement_grant_granted_by_known CHECK (
        granted_by IN ('paddle', 'stripe', 'operator')
    );

DROP INDEX entitlement_grant_paddle_source_unique;

CREATE UNIQUE INDEX entitlement_grant_source_unique
    ON entitlement_grant (source_ref)
    WHERE source_ref IS NOT NULL AND revoked_at IS NULL;

ALTER TABLE billing_subscription
    RENAME COLUMN paddle_subscription_id TO provider_subscription_id;

ALTER TABLE billing_subscription
    RENAME COLUMN paddle_customer_id TO provider_customer_id;

ALTER TABLE billing_subscription
    RENAME CONSTRAINT billing_subscription_paddle_id_unique
        TO billing_subscription_provider_id_unique;

-- Every row that exists today was written by the Paddle webhook, so the
-- column arrives defaulted to 'paddle', which backfills them with a fact
-- rather than an assumption, and the default then moves to 'stripe' for
-- every row written from here. Two DEFAULTs rather than an UPDATE because
-- this table forces row-level security and the migration role is not exempt
-- from it: an UPDATE here would match no rows and say nothing about it.
ALTER TABLE billing_subscription
    ADD COLUMN provider text NOT NULL DEFAULT 'paddle';

ALTER TABLE billing_subscription
    ALTER COLUMN provider SET DEFAULT 'stripe';

ALTER TABLE billing_subscription
    ADD COLUMN provider_price_id text;

ALTER TABLE billing_subscription
    ADD CONSTRAINT billing_subscription_provider_known CHECK (
        provider IN ('paddle', 'stripe')
    );

ALTER TABLE billing_subscription
    DROP CONSTRAINT billing_subscription_identifiers_named;

ALTER TABLE billing_subscription
    ADD CONSTRAINT billing_subscription_identifiers_named CHECK (
        provider_subscription_id <> '' AND provider_customer_id <> '' AND status <> ''
    );

CREATE TABLE service_booking (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    id           uuid        NOT NULL,
    key          text        NOT NULL,
    provider_ref text        NOT NULL,
    created_at   timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT service_booking_provider_ref_unique
        UNIQUE (provider_ref),

    CONSTRAINT service_booking_named CHECK (
        key <> '' AND provider_ref <> ''
    )
);

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form: a pooled connection reverts a transaction-local
-- app.current_org to the empty string rather than to missing, so NULLIF is
-- what keeps an unpinned statement matching nothing instead of raising 22P02.
ALTER TABLE service_booking ENABLE ROW LEVEL SECURITY;
ALTER TABLE service_booking FORCE ROW LEVEL SECURITY;
CREATE POLICY service_booking_org_isolation ON service_booking
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- Both halves of the operator's reach, for the reason migration 0037 states:
-- tam_backoffice is not BYPASSRLS, so a grant alone shows it nothing on a
-- fenced table and a policy alone gives it nothing to read. SELECT only: a
-- booking is written by the webhook through the application pool with the
-- organisation pinned, exactly as every other tenant write is.
GRANT SELECT ON service_booking TO tam_backoffice;

CREATE POLICY service_booking_backoffice_read ON service_booking
    FOR SELECT TO tam_backoffice USING (true);
