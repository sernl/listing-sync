-- What Paddle has told us about one organisation's subscription, and nothing
-- more. State tracking only: no entitlement is decided here and no surface is
-- gated on this table yet.
--
-- One row per organisation, so the primary key is the tenant. A seller holds
-- one subscription to this product at a time; a second concurrent
-- subscription for the same tenant is a billing situation to answer
-- deliberately, not a shape to leave the schema open to.
--
-- paddle_subscription_id is unique across every tenant. Paddle's identifier
-- names one subscription in Paddle's ledger, so two organisations claiming
-- one is either a replayed event carrying somebody else's custom_data or our
-- own checkout mislabelling an org, and both must fail loudly rather than
-- silently attach a paying customer to a second tenant. The constraint is
-- enforced by the index rather than by any policy, so it holds across the
-- tenant fence, which is the whole reason it is written here.
--
-- status is text, deliberately, and carries no CHECK. Paddle's subscription
-- vocabulary is its own and it versions on Paddle's schedule: today's set is
-- active, trialing, past_due, paused and canceled, and a value outside it
-- arriving at three in the morning must land in a row an operator can read
-- rather than abort a webhook Paddle will then retry forever. Storing what
-- was received keeps the state legible; a closed enum invented before any
-- real traffic has been seen would decide the vocabulary from documentation
-- rather than from delivery. Closing it later is a forward migration once the
-- observed set is known.
--
-- current_period_end is nullable because Paddle does not always carry a
-- billing period: a subscription in trial or newly cancelled may arrive with
-- none, and a fabricated one would read as a fact.
--
-- occurred_at is the event time Paddle stamped on the notification that
-- produced this state, not the instant we wrote it. It exists so an
-- out-of-order delivery can be refused: webhooks are retried and can arrive
-- reordered, and applying a stale event would regress a live subscription to
-- a state it has already left. The upsert in crates/tam-storage/src/billing.rs
-- compares against this column and declines to overwrite with anything older.
-- updated_at is the separate wall instant of the write itself, so the two
-- questions -- when did this happen, and when did we learn it -- stay
-- answerable apart.
CREATE TABLE billing_subscription (
    org_id                 uuid        NOT NULL REFERENCES organisation (id),
    paddle_subscription_id text        NOT NULL,
    paddle_customer_id     text        NOT NULL,
    status                 text        NOT NULL,
    current_period_end     timestamptz,
    occurred_at            timestamptz NOT NULL,
    updated_at             timestamptz NOT NULL,

    PRIMARY KEY (org_id),

    CONSTRAINT billing_subscription_paddle_id_unique
        UNIQUE (paddle_subscription_id),

    CONSTRAINT billing_subscription_identifiers_named CHECK (
        paddle_subscription_id <> '' AND paddle_customer_id <> '' AND status <> ''
    )
);

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form: a pooled connection reverts a transaction-local
-- app.current_org to the empty string rather than to missing, so NULLIF is
-- what keeps an unpinned statement matching nothing instead of raising 22P02.
ALTER TABLE billing_subscription ENABLE ROW LEVEL SECURITY;
ALTER TABLE billing_subscription FORCE ROW LEVEL SECURITY;
CREATE POLICY billing_subscription_org_isolation ON billing_subscription
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);
