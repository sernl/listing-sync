-- The refund policy of the Terms (teachouse.io/terms/#refunds), applied by
-- the code rather than by an operator's arithmetic: what the policy quoted
-- for a refund beside what was actually refunded, and the seller's own
-- "Ask for a refund".
--
-- A refund issued against a quote records the quote: `policy_basis` is the
-- rule that decided it (tam-api's `RefundBasis`, spelled as the CHECK below
-- enumerates) and `quoted_cents` what that rule gave. An operator may refund
-- another amount, and must say why in `override_reason`, so the record
-- answers "policy said X, we refunded Y, because Z" without a second table.
-- A refund issued before this migration, or found in Stripe's dashboard,
-- carries no quote, which is why the three are nullable and paired rather
-- than backfilled with a guess.
ALTER TABLE refund ADD COLUMN policy_basis text;
ALTER TABLE refund ADD COLUMN quoted_cents bigint;
ALTER TABLE refund ADD COLUMN override_reason text;

ALTER TABLE refund ADD CONSTRAINT refund_policy_basis_known CHECK (
    policy_basis IS NULL OR policy_basis IN (
        'pack_unused', 'pack_used', 'pack_window_closed',
        'monthly_started', 'monthly_not_started',
        'yearly_unused_months', 'yearly_ended', 'unmatched'
    )
);
ALTER TABLE refund ADD CONSTRAINT refund_quote_paired CHECK (
    (policy_basis IS NULL) = (quoted_cents IS NULL)
);
ALTER TABLE refund ADD CONSTRAINT refund_quote_not_negative CHECK (
    quoted_cents IS NULL OR quoted_cents >= 0
);
ALTER TABLE refund ADD CONSTRAINT refund_override_reason_bounded CHECK (
    override_reason IS NULL OR (override_reason <> '' AND length(override_reason) <= 1000)
);
ALTER TABLE refund ADD CONSTRAINT refund_override_explained CHECK (
    quoted_cents IS NULL OR quoted_cents = amount_cents OR override_reason IS NOT NULL
);

-- One seller's "Ask for a refund" on one charge, and what an operator made
-- of it.
--
-- Global like refund and payment_event (migration 0102), and for their
-- reason: the operator routes that list and decide these hold no tenant pin,
-- and the seller's own routes read and write only `WHERE org_id = <their
-- organisation>`, which the route supplies from the session rather than the
-- request. erase_organisation (migration 0092) deletes the departing
-- organisation's rows here as it does in every table carrying an org_id.
-- tam_backoffice is granted nothing (tests/backoffice_grants.rs).
--
-- `quoted_cents` is what the policy gave on the day the seller asked, and is
-- what Approve refunds: the seller was shown that amount, and a month
-- turning over while the request waits for an operator must not shrink it.
-- A request is only made for a quote above zero; the console says why there
-- is nothing to ask for otherwise.
--
-- `refund_id` is the refund Approve issued, under the request's own id, so
-- an approval retried after a lost reply answers the same refund.
CREATE TABLE refund_request (
    id                uuid        NOT NULL,
    org_id            uuid        NOT NULL,
    charge_id         text        NOT NULL,
    quoted_cents      bigint      NOT NULL,
    currency          text        NOT NULL,
    policy_basis      text        NOT NULL,
    status            text        NOT NULL DEFAULT 'requested',
    -- What the seller wrote, for the operator. Optional.
    note              text,
    -- The app_user who asked. No foreign key, for the reason refund's
    -- issued_by carries none.
    requested_by      uuid,
    decided_by        uuid,
    decided_by_label  text,
    decided_at        timestamptz,
    -- Why it was declined, which the seller's mail carries.
    decline_reason    text,
    refund_id         uuid,
    created_at        timestamptz NOT NULL,
    updated_at        timestamptz NOT NULL,

    PRIMARY KEY (id),
    CONSTRAINT refund_request_status_known CHECK (
        status IN ('requested', 'approved', 'declined')
    ),
    CONSTRAINT refund_request_basis_known CHECK (policy_basis IN (
        'pack_unused', 'pack_used', 'pack_window_closed',
        'monthly_started', 'monthly_not_started',
        'yearly_unused_months', 'yearly_ended', 'unmatched'
    )),
    CONSTRAINT refund_request_quote_positive CHECK (quoted_cents > 0),
    CONSTRAINT refund_request_note_bounded CHECK (note IS NULL OR length(note) <= 1000),
    CONSTRAINT refund_request_reason_bounded CHECK (
        decline_reason IS NULL OR (decline_reason <> '' AND length(decline_reason) <= 1000)
    ),
    CONSTRAINT refund_request_decided CHECK ((status = 'requested') = (decided_at IS NULL)),
    CONSTRAINT refund_request_declined_says_why CHECK (
        (status = 'declined') = (decline_reason IS NOT NULL)
    ),
    CONSTRAINT refund_request_approved_refunds CHECK (
        (status = 'approved') = (refund_id IS NOT NULL)
    )
);

-- One open request per charge: asking twice is asking once.
CREATE UNIQUE INDEX refund_request_one_open ON refund_request (charge_id)
    WHERE status = 'requested';
CREATE INDEX refund_request_by_org ON refund_request (org_id, created_at DESC);
CREATE INDEX refund_request_by_time ON refund_request (created_at DESC);
