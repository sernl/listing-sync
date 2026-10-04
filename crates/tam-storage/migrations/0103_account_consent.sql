-- What a person agreed to when their account was made, and again each time the
-- terms changed: the durable record behind the two boxes on the sign-up form.
--
-- One row per statement, three per agreement: the Terms of Service and the
-- Privacy Policy, ownership of what they publish, and being 18 or older. The
-- form ties the first two to one box, but they are separate promises and an
-- operator answering "did this person say they own that file?" reads one row
-- rather than parsing a sentence.
--
-- Keyed on the identity subject (auth."user".id), not on app_user: the
-- identity service writes the sign-up's rows the moment the account exists,
-- before the person has opened the console and so before any app_user row
-- names them. The subject is a uuid with no foreign key, because the identity
-- schema is not this migration set's to reference, and because the record
-- outlives the account it describes: an agreement that existed is still
-- evidence after the person has gone.
--
-- `document_version` is the Terms' effective date as the console and the
-- landing page both read it (`TERMS_VERSION`, emitted by tam-typegen), so
-- "which terms did they accept" is answered by a date a person can look up.
--
-- Global, carrying no row-level security, for the reason payment_event carries
-- none (migration 0102): no tenant owns an identity, and the writers are the
-- identity service's internal route and the signed-in person's own re-consent,
-- neither of which reads through a tenant pin. tam_backoffice is granted
-- SELECT, because the operator's Users page is where a disputed agreement is
-- looked up.
--
-- Append-only. UPDATE, DELETE and TRUNCATE are revoked from tam_app, and
-- because the owning role can always grant itself back what it lost, a
-- statement-level trigger refuses UPDATE and DELETE regardless (the pattern
-- of migration 0092): nothing legitimately rewrites or removes an agreement.

CREATE TYPE account_consent_kind AS ENUM ('terms_privacy', 'ip_ownership', 'age_18');

CREATE TABLE account_consent (
    id               uuid                 NOT NULL DEFAULT gen_random_uuid(),
    subject          uuid                 NOT NULL,
    -- The address the account held when it agreed, as the identity service
    -- reported it. Nullable: the console's re-consent knows the app user's
    -- address, but a deployment could record an agreement without one.
    email            text,
    kind             account_consent_kind NOT NULL,
    document_version text                 NOT NULL,
    accepted_at      timestamptz          NOT NULL,
    ip_address       inet,
    user_agent       text,
    created_at       timestamptz          NOT NULL DEFAULT now(),

    PRIMARY KEY (id),
    CONSTRAINT account_consent_version_is_a_date
        CHECK (document_version ~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'),
    CONSTRAINT account_consent_user_agent_bounded
        CHECK (user_agent IS NULL OR length(user_agent) <= 1024)
);

CREATE INDEX account_consent_by_subject ON account_consent (subject, accepted_at DESC);

REVOKE UPDATE, DELETE, TRUNCATE ON account_consent FROM tam_app;

CREATE FUNCTION refuse_delete_append_only() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '% is append-only; a row never leaves it', TG_TABLE_NAME
        USING ERRCODE = 'insufficient_privilege';
END
$$;

CREATE TRIGGER account_consent_never_updated
    BEFORE UPDATE ON account_consent
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_update_append_only();

CREATE TRIGGER account_consent_never_deleted
    BEFORE DELETE ON account_consent
    FOR EACH STATEMENT EXECUTE FUNCTION refuse_delete_append_only();

GRANT SELECT ON account_consent TO tam_backoffice;
