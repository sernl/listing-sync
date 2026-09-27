-- An administrator deleting an identity account is recorded.
--
-- better-auth's admin plugin deletes the user row, its sessions and its
-- accounts at /admin/remove-user, and nothing of the account survives it --
-- so this table, which has no foreign key to the user, is the only place the
-- act can be read afterwards. Both parties are named, as for impersonation:
-- user_id is the administrator who acted and target_user_id the account that
-- was removed.
--
-- The console deletes the platform user and their organisation through
-- tam-api first (DELETE /v1/admin/users/{subject}) and only then removes the
-- identity account, so a refusal there leaves both halves standing.

SET search_path = auth;

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
        'user_impersonation_stopped',
        'user_removed'
    )
);

ALTER TABLE auth_event DROP CONSTRAINT auth_event_impersonation_parties_identified;

-- Renamed as it widens: it now states that every two-party event names both.
ALTER TABLE auth_event ADD CONSTRAINT auth_event_two_parties_identified CHECK (
    event NOT IN ('user_impersonated', 'user_impersonation_stopped', 'user_removed')
    OR (user_id IS NOT NULL AND target_user_id IS NOT NULL)
);
