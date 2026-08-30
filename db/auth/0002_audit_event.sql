-- The identity audit trail: one append-only row per platform-identity event.
--
-- This is the self-hosted substitute for better-auth's hosted Infrastructure,
-- whose `dash()` plugin collects the same record behind a paid subscription
-- (`docs/content/docs/infrastructure/plugins/audit-logs.mdx` in the 1.7.2
-- clone). The event names below are that plugin's vocabulary verbatim, so a
-- later move onto it is a change of writer rather than of schema. One name is
-- ours: `user_sign_in_failed` has no counterpart outside the paid Sentinel
-- plugin, whose `security_*` names all presuppose detection logic we do not
-- run, and a compliance record that omits failed sign-ins is not one.
--
-- Written by tam-auth from the after-hooks in auth/src/auth.ts. Read by the
-- Rust side as tam_app, which holds SELECT here and on nothing else in this
-- schema.

SET search_path = auth;

CREATE TABLE auth_event (
    id         bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event      text        NOT NULL,
    user_id    uuid,
    identifier text,
    session_id uuid,
    ip_address text,
    user_agent text,
    detail     text,

    -- Stamped by the database rather than passed by the writer: the
    -- writer is the role the grants below confine to INSERT and SELECT,
    -- and letting it choose the instant would hand back part of what
    -- that confinement takes away.
    at         timestamptz NOT NULL DEFAULT now(),

    -- Closed here rather than in the writer, as connection_audit closes its
    -- own: a log admitting an unplanned verb reads as coverage it does not
    -- have.
    CONSTRAINT auth_event_event CHECK (
        event IN (
            'user_signed_up',
            'user_signed_in',
            'user_sign_in_failed',
            'user_signed_out',
            'password_reset_requested',
            'password_reset_completed'
        )
    ),

    -- Every event names its subject, by user id once one is known and by the
    -- submitted email before that -- a failed sign-in has no user id and is
    -- the reason identifier exists at all. The single exception is not a
    -- relaxation but a fact about the endpoint: /reset-password consumes the
    -- reset token and answers {status: true}, so no hook downstream of it can
    -- see whose password changed. That row records the act, its instant and
    -- its network origin; the preceding password_reset_requested row carries
    -- the identity.
    CONSTRAINT auth_event_subject_identified CHECK (
        event = 'password_reset_completed'
        OR user_id IS NOT NULL
        OR identifier IS NOT NULL
    )
);

-- No foreign key to "user" or "session", matching connection_audit and
-- field_audit: an audit row must outlive whatever it records, and sign-out
-- deletes the very session row this column correlates to.

-- The read path a per-user activity view takes. Further indexes wait for a
-- read path that needs them.
CREATE INDEX auth_event_by_user ON auth_event (user_id, at DESC);

-- Append-only against its own writer, the way connection_audit is against
-- tam_app. tam_auth owns this table, so the privileges revoked here are the
-- implicit ones ownership carries; what remains is INSERT and SELECT.
REVOKE UPDATE, DELETE ON auth_event FROM tam_auth;

-- The one-directional grant. tam_app reads the audit trail and nothing else
-- in this schema: no user, no session, no account, and above all no
-- account.password or jwks.privateKey. The matching USAGE on the schema is in
-- db/init/02-auth-role.sql, which is where the schema exists at initdb time;
-- this table does not yet, so its grant belongs here beside it.
GRANT SELECT ON auth_event TO tam_app;
