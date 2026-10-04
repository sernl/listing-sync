-- Abuse prevention, part two: what the scorer concluded, what an operator
-- decided about it, and what a ban refuses afterwards.
--
-- abuse_flag is one suspicion about one organisation: a rule that fired
-- (`kind`), how much it weighs (`score`), and why in words (`reason`). The
-- scorer writes it open; an operator resolves it on the Abuse page with an
-- action. The operator's decision is about the organisation rather than the
-- one flag, so acting resolves every open flag the organisation has, and the
-- organisation's standing is the action of its most recently decided flag
-- (abuse_standing below). Dismissing is deciding "none", which is also how a
-- ban is lifted. A flag the scorer raises later is open and changes nothing
-- until somebody decides it.
--
-- `warn` and `ban` queue a mail on the flag they were taken on: the mail_*
-- columns are that outbox, drained by tam-server exactly as refund mail is
-- (0102), and the outcome lands back on the row the page shows.
--
-- Global, like the ledger it is computed from (0105): written by the scorer
-- across tenants and by operator routes. org_id is a real column, so an
-- erasure takes an organisation's flags with it.
--
-- banned_identity is what a ban refuses for twenty-four months: the keyed
-- digests of the banned organisation's email addresses, shops, devices and
-- cards. Deliberately keyed by origin_org rather than org_id, so the erasure
-- walk (0092) does not find it: a banned seller deleting their account must
-- not be the way back in. It holds no plaintext -- every value is a keyed
-- digest (tam_secrets::abuse_digest, and the shop's own 0031 digest) -- and
-- tam-server's abuse loop deletes rows past expires_at. Lifting the ban
-- deletes the rows it wrote.

CREATE TABLE abuse_flag (
    id                 uuid        NOT NULL,
    org_id             uuid        NOT NULL REFERENCES organisation (id),
    kind               text        NOT NULL,
    score              int         NOT NULL,
    reason             text        NOT NULL,
    created_at         timestamptz NOT NULL,
    resolved_at        timestamptz,
    -- The operator's app_user id. No foreign key: an operator can leave, and
    -- the decision they made stays a decision.
    resolved_by        uuid,
    action             text        NOT NULL DEFAULT 'none',
    action_reason      text,
    mail_requested_at  timestamptz,
    mail_due_at        timestamptz,
    mail_attempts      int         NOT NULL DEFAULT 0,
    mail_sent_at       timestamptz,
    mail_error         text,

    PRIMARY KEY (id),

    CONSTRAINT abuse_flag_kind_known CHECK (
        kind IN ('shared_shop', 'shared_device', 'shared_payment', 'signup_burst',
                 'disposable_email', 'quick_unlink')
    ),
    CONSTRAINT abuse_flag_score_bounded CHECK (score BETWEEN 0 AND 100),
    CONSTRAINT abuse_flag_reason_said CHECK (reason <> '' AND length(reason) <= 500),
    CONSTRAINT abuse_flag_action_known CHECK (action IN ('none', 'warn', 'limit', 'ban')),
    CONSTRAINT abuse_flag_action_reason_bounded CHECK (
        action_reason IS NULL OR length(action_reason) <= 500
    ),
    -- Decided as a whole or not at all: an action nobody took, or a decision
    -- with no decider, would each be a standing nobody can account for.
    CONSTRAINT abuse_flag_decision_whole CHECK (
        (resolved_at IS NULL AND resolved_by IS NULL AND action = 'none')
        OR (resolved_at IS NOT NULL AND resolved_by IS NOT NULL)
    ),
    CONSTRAINT abuse_flag_mail_attempts_counted CHECK (mail_attempts >= 0)
);

-- One open suspicion per rule per organisation, which is what makes the
-- scorer idempotent: a nightly pass over the same evidence raises nothing new.
CREATE UNIQUE INDEX abuse_flag_one_open ON abuse_flag (org_id, kind) WHERE resolved_at IS NULL;
-- The standing read, made on every authenticated request.
CREATE INDEX abuse_flag_decided ON abuse_flag (org_id, resolved_at DESC) WHERE resolved_at IS NOT NULL;
CREATE INDEX abuse_flag_mail_due ON abuse_flag (mail_due_at)
    WHERE mail_requested_at IS NOT NULL AND mail_sent_at IS NULL;

-- An organisation's standing: the action of its most recently decided flag,
-- or 'none'. The session gate, the free-move grant and the heartbeat's link
-- derivation all ask this one question, so it is said once.
CREATE FUNCTION abuse_standing(target uuid) RETURNS text
    LANGUAGE sql STABLE AS $$
    SELECT COALESCE(
        (SELECT action FROM abuse_flag
          WHERE org_id = target AND resolved_at IS NOT NULL
          ORDER BY resolved_at DESC, id DESC
          LIMIT 1),
        'none')
$$;

CREATE TABLE banned_identity (
    kind        text        NOT NULL,
    value_hash  bytea       NOT NULL,
    origin_org  uuid        NOT NULL,
    flag_id     uuid        NOT NULL,
    reason      text        NOT NULL,
    created_at  timestamptz NOT NULL,
    expires_at  timestamptz NOT NULL,

    PRIMARY KEY (kind, value_hash),

    CONSTRAINT banned_identity_kind_known CHECK (
        kind IN ('email', 'shop_digest', 'device_fingerprint', 'payment_fingerprint')
    ),
    CONSTRAINT banned_identity_value_sized CHECK (octet_length(value_hash) = 32),
    CONSTRAINT banned_identity_reason_said CHECK (reason <> '' AND length(reason) <= 500),
    CONSTRAINT banned_identity_expiry_after CHECK (expires_at > created_at)
);

CREATE INDEX banned_identity_by_origin ON banned_identity (origin_org);
CREATE INDEX banned_identity_by_expiry ON banned_identity (expires_at);

-- The Abuse page reads both on the backoffice pool; every write is the
-- application pool's.
GRANT SELECT ON abuse_flag, banned_identity TO tam_backoffice;
