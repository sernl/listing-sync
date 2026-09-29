-- The operators' mail to sellers: a campaign, one row per recipient that is
-- also that recipient's outbox entry, and the pictures a campaign body shows.
--
-- The recipient row is the outbox. `outbox_message` is keyed and fenced per
-- organisation, and a campaign is one operator write across every tenant at
-- once; queueing through it would mean pinning each organisation in turn
-- inside the operator's transaction. The drainer in tam-server claims a due
-- `queued` row here instead, sends it, and writes the outcome back to the
-- same row: queued, sent (with the relay's message id), failed (with its
-- answer), or skipped (unsubscribed, no address, or an address the identity
-- service does not vouch for when the campaign asked for verified ones).
--
-- No address is stored, here or anywhere in this database. The drainer asks
-- the identity service for it at send time, exactly as the completion mail
-- does (docs/design/decisions.md, the internal address route), and holds it
-- for the length of one send.
--
-- All three tables are global: a campaign belongs to the platform, not to a
-- tenant, and the application pool writes and reads them from the operator
-- routes (tests/rls_matrix.rs). tam_backoffice is granted nothing here
-- (tests/backoffice_grants.rs), for the reason it has nothing on
-- site_setting: the operator surface that uses these runs on the application
-- pool.

CREATE TABLE mail_campaign (
    id               uuid        NOT NULL,
    subject          text        NOT NULL,
    -- The sanitised body and the button, NULL once the campaign is deleted.
    -- Deleting frees the body and every recipient row; this row stays as the
    -- log line saying what was sent, to whom by filter, how many, and by whom.
    body_html        text,
    link_url         text,
    link_label       text,
    -- The filter as the operator chose it: {segment, exclude_operators,
    -- verified_only}. Parsed in crates/tam-api/src/mail_campaigns.rs.
    audience         jsonb       NOT NULL,
    -- A test send to the operator themselves: one recipient, kept off the
    -- campaign list.
    test             boolean     NOT NULL DEFAULT false,
    -- Who sent it. No foreign key, for the reason platform_operator_event
    -- carries none: an operator who later leaves must not take the log with
    -- them. `created_by_label` is the name the console showed for them.
    created_by       uuid        NOT NULL,
    created_by_label text        NOT NULL,
    created_at       timestamptz NOT NULL,
    deleted_at       timestamptz,
    deleted_by       uuid,
    -- Recipient counts frozen at deletion, so the log line keeps its numbers
    -- after the rows they were counted from are gone: {total, queued, sent,
    -- failed, skipped}.
    final_counts     jsonb,

    PRIMARY KEY (id),

    CONSTRAINT mail_campaign_subject_bounded CHECK (length(subject) BETWEEN 1 AND 200),
    CONSTRAINT mail_campaign_body_bounded CHECK (octet_length(body_html) <= 204800),
    CONSTRAINT mail_campaign_deleted_total CHECK (
        (deleted_at IS NULL) = (deleted_by IS NULL)
        AND (deleted_at IS NULL) = (final_counts IS NULL)
        AND (deleted_at IS NULL OR body_html IS NULL)
    ),
    CONSTRAINT mail_campaign_live_has_body CHECK (deleted_at IS NOT NULL OR body_html IS NOT NULL)
);

CREATE INDEX mail_campaign_newest_first ON mail_campaign (created_at DESC, id);

CREATE TABLE mail_campaign_recipient (
    campaign_id     uuid        NOT NULL REFERENCES mail_campaign (id) ON DELETE CASCADE,
    -- Erasing a seller (erase_organisation, migration 0092) deletes their
    -- app_user row, and their place in a campaign goes with it.
    user_id         uuid        NOT NULL REFERENCES app_user (id) ON DELETE CASCADE,
    -- What the drainer resolves an address from, copied at queue time.
    auth_subject    uuid        NOT NULL,
    -- `@org` and `@plan` as they were when the campaign was queued: the mail
    -- says what the seller held when the operator chose them.
    org_name        text        NOT NULL,
    plan            text        NOT NULL,
    status          text        NOT NULL DEFAULT 'queued',
    attempts        integer     NOT NULL DEFAULT 0,
    next_attempt_at timestamptz NOT NULL,
    -- A claim: the drainer holding this row until then. Another pass skips it.
    leased_until    timestamptz,
    provider_id     text,
    error           text,
    updated_at      timestamptz NOT NULL,

    PRIMARY KEY (campaign_id, user_id),

    CONSTRAINT mail_campaign_recipient_status_known CHECK (
        status IN ('queued', 'sent', 'failed', 'skipped')
    ),
    CONSTRAINT mail_campaign_recipient_sent_has_id CHECK (
        status <> 'sent' OR provider_id IS NOT NULL
    ),
    CONSTRAINT mail_campaign_recipient_attempts_nonnegative CHECK (attempts >= 0)
);

-- The drainer's claim: the oldest due queued row.
CREATE INDEX mail_campaign_recipient_due
    ON mail_campaign_recipient (next_attempt_at)
    WHERE status = 'queued';

CREATE INDEX mail_campaign_recipient_by_user ON mail_campaign_recipient (user_id);

-- A picture a campaign body shows, uploaded by an operator. The bytes are a
-- blob under the platform organisation (like a guide's picture); this row is
-- what makes one public at GET /v1/mail/images/{handle}, which a mail client
-- fetches with no session. Guide pictures stay behind sign-in: a hash that is
-- not listed here answers 404 on the public route.
CREATE TABLE mail_image (
    hash        bytea       NOT NULL,
    uploaded_by uuid        NOT NULL,
    uploaded_at timestamptz NOT NULL,

    PRIMARY KEY (hash),

    CONSTRAINT mail_image_hash_length CHECK (octet_length(hash) = 32)
);

-- A seller's answer to the operators' news: off once they follow an
-- unsubscribe link or turn it off in their notification settings. Separate
-- from notify_email (migration 0063), which is about their own runs.
ALTER TABLE app_user ADD COLUMN marketing_opt_out boolean NOT NULL DEFAULT false;

-- The capability an unsubscribe link carries: random, one per user, and
-- naming nothing about them. A link keeps working after the campaign that
-- carried it is deleted, because it names the user and not the send.
ALTER TABLE app_user ADD COLUMN marketing_token uuid NOT NULL DEFAULT gen_random_uuid();
ALTER TABLE app_user ADD CONSTRAINT app_user_marketing_token UNIQUE (marketing_token);
