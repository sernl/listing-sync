-- Impersonation, added to the identity audit trail.
--
-- An identity admin signing in as a user is ratified on one condition: the act
-- leaves an append-only record readable by someone other than the impersonator.
-- That record is this table's, and the two events below are the whole of what
-- is added to it.
--
-- The two names are the hosted audit-logs plugin's, verbatim, as every name in
-- 0002_audit_event.sql but one is: they are the `user_impersonated` and
-- `user_impersonation_stopped` of its Session tab
-- (`docs/content/docs/infrastructure/plugins/audit-logs.mdx` in the 1.7.2
-- clone), so a later move onto that plugin remains a change of writer rather
-- than of schema.
--
-- Written by tam-auth from the session database hooks in auth/src/auth.ts.

SET search_path = auth;

-- user_id stays what it is for every other event here: the party who performed
-- the act, which for these two is the admin. target_user_id is the party it
-- was performed upon. Only impersonation has two parties -- for every other
-- event the actor is the subject and this column stays null.
ALTER TABLE auth_event ADD COLUMN target_user_id uuid;

-- Replaced whole rather than relaxed, for the reason the constraint exists at
-- all: a log admitting a name its writer never sends reads as coverage it does
-- not have.
ALTER TABLE auth_event DROP CONSTRAINT auth_event_event;

ALTER TABLE auth_event ADD CONSTRAINT auth_event_event CHECK (
    event IN (
        'user_signed_up',
        'user_signed_in',
        'user_sign_in_failed',
        'user_signed_out',
        'password_reset_requested',
        'password_reset_completed',
        'user_impersonated',
        'user_impersonation_stopped'
    )
);

-- Both parties, or the row is not a record of an impersonation. The column
-- stays nullable because six of the eight events have no second party;
-- auth_event_subject_identified demands an actor of every event and is
-- satisfied by user_id alone, so it cannot demand a target. Stated here so a
-- reader of these two events never has to ask whether the pair is complete,
-- and so the read that lists them can say so in its types.
ALTER TABLE auth_event ADD CONSTRAINT auth_event_impersonation_parties_identified CHECK (
    event NOT IN ('user_impersonated', 'user_impersonation_stopped')
    OR (user_id IS NOT NULL AND target_user_id IS NOT NULL)
);

-- Append-only means append-only. TRUNCATE empties a table without holding
-- UPDATE or DELETE, and 0002_audit_event.sql revoked only those two, so the
-- owner kept it. Revoked here rather than there because that file is landed.
REVOKE TRUNCATE ON auth_event FROM tam_auth;

-- Re-asserted rather than assumed. Nothing above touches the table's ACL, so
-- the UPDATE and DELETE revokes hold across the new column and tam_app's
-- table-level SELECT already reaches it; saying so again costs one idempotent
-- statement and puts the property in the file that could have broken it.
REVOKE UPDATE, DELETE ON auth_event FROM tam_auth;
