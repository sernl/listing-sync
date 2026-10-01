-- The payments ledger the operators' Payments page reads: every money event
-- Stripe reports, and every refund an operator issues.
--
-- Stripe stays the source of truth. These rows are a copy kept so the page
-- can cross-reference a charge with its refunds, its dispute and its invoice
-- in one read, and so a refund has somewhere to record who issued it, why,
-- and whether the customer was told. Rows arrive three ways: the billing
-- webhook (one row per Stripe event, keyed on the event id), the page's
-- "Sync from Stripe" (one row per object, keyed `sync:<kind>:<object id>`),
-- and the refund route itself.
--
-- Global, carrying no row-level security, for the reason discount and
-- mail_campaign carry none (migrations 0091, 0097): only operator routes read
-- or write these tables, on the application pool, and the webhook that
-- writes them holds no tenant pin to fence by. `org_id` is a cross-reference
-- for the page, nullable because a charge made outside our checkout names no
-- organisation we can resolve. tam_backoffice is granted nothing here
-- (tests/backoffice_grants.rs).

CREATE TABLE payment_event (
    id                 uuid        NOT NULL,
    -- No foreign key, and none is needed for erasure: erase_organisation
    -- (migration 0092) deletes every row carrying the departing org_id,
    -- these included. Stripe keeps its own record of the money.
    org_id             uuid,
    provider           text        NOT NULL DEFAULT 'stripe',
    -- Stripe's `evt_…` for a webhook row, `sync:<kind>:<object id>` for a row
    -- the sync wrote. Unique, so a redelivered event is a no-op.
    provider_event_id  text        NOT NULL,
    kind               text        NOT NULL,
    amount_cents       bigint,
    currency           text,
    -- The object the event describes: a charge, payment intent, refund,
    -- dispute, invoice or subscription id.
    provider_object_id text        NOT NULL,
    -- The ids the page cross-references by. Which are known depends on the
    -- object and on the Stripe API version it was rendered under.
    charge_id          text,
    payment_intent_id  text,
    invoice_id         text,
    customer_id        text,
    status             text,
    reason             text,
    occurred_at        timestamptz NOT NULL,
    raw                jsonb       NOT NULL,
    created_at         timestamptz NOT NULL,

    PRIMARY KEY (id),
    CONSTRAINT payment_event_provider_event UNIQUE (provider_event_id),
    CONSTRAINT payment_event_provider_known CHECK (provider = 'stripe'),
    CONSTRAINT payment_event_kind_known CHECK (kind IN (
        'payment_succeeded', 'payment_failed',
        'refund_created', 'refund_updated',
        'dispute_opened', 'dispute_closed',
        'invoice_paid', 'invoice_payment_failed',
        'subscription_created', 'subscription_canceled'
    ))
);

CREATE INDEX payment_event_by_time ON payment_event (occurred_at DESC);
CREATE INDEX payment_event_by_object ON payment_event (kind, provider_object_id);
CREATE INDEX payment_event_by_charge ON payment_event (charge_id) WHERE charge_id IS NOT NULL;

-- One refund, issued from the Payments page or found in Stripe (a refund made
-- in Stripe's dashboard arrives through the webhook or the sync, with no
-- operator to name).
--
-- `status` is Stripe's, narrowed: `requires_action` reads as pending. The
-- webhook's `refund.updated` settles it.
--
-- The mail columns are the refund mail's outbox, drained by tam-server the
-- way mail_campaign_recipient is: `mail_requested_at` queues it,
-- `mail_due_at` leases and backs off a claim, and `mail_sent_at` records
-- that it went out, which is what turns the page's "Send email" into
-- "Sent on …".
CREATE TABLE refund (
    id                  uuid        NOT NULL,
    org_id              uuid,
    provider_refund_id  text        NOT NULL,
    charge_id           text        NOT NULL,
    amount_cents        bigint      NOT NULL,
    currency            text        NOT NULL,
    status              text        NOT NULL,
    reason              text,
    note                text,
    -- Who issued it here. No foreign key, for the reason
    -- platform_operator_event carries none: an operator who later leaves
    -- must not take the record with them.
    issued_by           uuid,
    issued_by_label     text,
    mail_requested_at   timestamptz,
    mail_due_at         timestamptz,
    mail_attempts       integer     NOT NULL DEFAULT 0,
    mail_error          text,
    mail_sent_at        timestamptz,
    created_at          timestamptz NOT NULL,
    updated_at          timestamptz NOT NULL,

    PRIMARY KEY (id),
    CONSTRAINT refund_provider_refund UNIQUE (provider_refund_id),
    CONSTRAINT refund_status_known CHECK (
        status IN ('pending', 'succeeded', 'failed', 'canceled')
    ),
    CONSTRAINT refund_amount_positive CHECK (amount_cents > 0),
    CONSTRAINT refund_note_bounded CHECK (note IS NULL OR length(note) <= 1000),
    CONSTRAINT refund_sent_was_requested CHECK (
        mail_sent_at IS NULL OR mail_requested_at IS NOT NULL
    )
);

CREATE INDEX refund_by_charge ON refund (charge_id);
CREATE INDEX refund_mail_due ON refund (mail_due_at)
    WHERE mail_requested_at IS NOT NULL AND mail_sent_at IS NULL;

-- The operators' trail gains one action: a refund issued from the console.
-- `user_id` is the operator who issued it and `refund_id` the refund, so the
-- trail answers "who refunded this" without a second table. The marking's
-- own reads filter to grant and revoke (OperatorRepo::events).
ALTER TABLE platform_operator_event ADD COLUMN refund_id uuid;
ALTER TABLE platform_operator_event DROP CONSTRAINT platform_operator_event_action_known;
ALTER TABLE platform_operator_event ADD CONSTRAINT platform_operator_event_action_known
    CHECK (action IN ('grant', 'revoke', 'refund'));
ALTER TABLE platform_operator_event ADD CONSTRAINT platform_operator_event_refund_named
    CHECK ((action = 'refund') = (refund_id IS NOT NULL));
