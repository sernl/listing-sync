-- A seller deleting their own account leaves one row behind: that it
-- happened, when, and what was cancelled on the way out.
--
-- Everything else about them goes. `DELETE /v1/account` cancels the Stripe
-- subscription, deletes the identity account (sessions, linked accounts and
-- passkeys with it), then erases the organisation through
-- erase_organisation (migration 0092) and writes this row in the same
-- transaction, after the erasure, so the erasure cannot take it and a failed
-- erasure leaves no row claiming an account is gone.
--
-- What it keeps is the least that answers "did this person delete their
-- account, and when": the identity subject, a SHA-256 of the address they
-- signed in with (lower-cased and trimmed first, so a support question
-- arriving from that address can be matched without the address being kept),
-- the organisation the account was, the subscription Stripe was told to
-- cancel, and the reason they gave, if they gave one. The domain database
-- still holds no seller's address (migration 0063's note on app_user).
--
-- `subject` is the key: an identity is deleted once, and a retry after a
-- failure before the erasure committed writes the row then rather than twice.
-- `org_id` carries no foreign key, because the organisation it names is gone
-- by the time the row is written. erase_organisation walks every table with
-- an org_id column, this one included, and finds nothing here for the
-- organisation it is erasing: the row arrives after it.
--
-- Global, carrying no row-level security, like platform_operator_event
-- (tests/rls_matrix.rs): written by the seller's own request on the
-- application pool after their organisation no longer exists, so there is no
-- tenant left to fence by. Append-only by use: only ErasureRepo inserts here,
-- and nothing updates it. tam_backoffice reads it, because "was this account
-- deleted, and did we cancel the subscription" is a support question
-- (tests/backoffice_grants.rs).
CREATE TABLE account_deletion (
    subject                uuid        NOT NULL,
    email_hash             text        NOT NULL,
    org_id                 uuid        NOT NULL,
    requested_at           timestamptz NOT NULL,
    stripe_subscription_id text,
    reason                 text,

    PRIMARY KEY (subject),

    CONSTRAINT account_deletion_email_hash_shape CHECK (email_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT account_deletion_reason_bounded CHECK (
        reason IS NULL OR (reason <> '' AND length(reason) <= 1000)
    )
);

CREATE INDEX account_deletion_by_time ON account_deletion (requested_at DESC);

GRANT SELECT ON account_deletion TO tam_backoffice;
