-- The session floor and the operation-resource key. app_user and
-- user_session are global rather than tenant tables: a session row is the
-- authentication root that must be readable before any tenant pin exists,
-- and the stored value is the token's 32-byte BLAKE3 digest, so the table holds a
-- verifier and never the capability itself. Minting is an operator one-shot
-- until M5 lands self-serve signup.

CREATE TABLE app_user (
    id         uuid        NOT NULL,
    org_id     uuid        NOT NULL REFERENCES organisation (id),
    email      text        NOT NULL,
    created_at timestamptz NOT NULL,

    PRIMARY KEY (id),

    CONSTRAINT app_user_email UNIQUE (email)
);

CREATE TABLE user_session (
    token_digest bytea       NOT NULL,
    user_id      uuid        NOT NULL REFERENCES app_user (id),
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    expires_at   timestamptz NOT NULL,
    created_at   timestamptz NOT NULL,

    PRIMARY KEY (token_digest),

    CONSTRAINT user_session_digest_len CHECK (octet_length(token_digest) = 32)
);

-- The request that starts a sync carries its own idempotency key, so the
-- retry and the double-click are the same job rather than two.
ALTER TABLE job ADD COLUMN request_idempotency_key uuid;
ALTER TABLE job ADD CONSTRAINT job_request_idempotent
    UNIQUE (org_id, request_idempotency_key);

-- The per-organisation pruning watermark the stream's resync decision reads.
-- org_seq values below it may be pruned; a client resuming below it receives
-- a resync event rather than a partial replay. The pruning job that advances
-- it is M1j's.
ALTER TABLE org_event_counter
    ADD COLUMN prune_watermark bigint NOT NULL DEFAULT 0;
