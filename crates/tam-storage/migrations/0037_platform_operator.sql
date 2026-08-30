-- The operator marking and the backoffice role's reach, in one reviewed file.
--
-- Two things land together because neither is any use alone: a row saying who
-- may read across tenants, and the enumerated privilege that read runs under.

-- Who may operate the platform. Keyed on app_user rather than carried as a
-- column there: app_user.org_id is NOT NULL, so a column on that table would
-- read as an org-scoped attribute of a tenant's user, while being an operator
-- is a platform fact about a human. A separate table also records the grant
-- and its withdrawal without widening a tenant-facing table.
--
-- Global, and carrying no row-level-security policy, for the reason app_user
-- and user_session carry none: the row must be readable before any tenant pin
-- exists, and it names a person rather than a tenant's data. The closed-world
-- test in tests/rls_matrix.rs is where that decision is recorded; a table
-- absent from both of its lists fails it.
--
-- Revocation is a nullable instant rather than a deleted row, so a withdrawn
-- grant stays legible afterwards. Re-granting a revoked operator updates this
-- row rather than inserting a second: one human, one marking, whose current
-- state is `revoked_at IS NULL`.
--
-- granted_by is one free-text actor name rather than the closed
-- actor_kind/actor_id sum migration 0033 makes mandatory on audit-bearing
-- rows, because the design note this table implements specifies one column
-- (docs/notes/design/admin-backoffice.md, section 1). Giving operator grants
-- the same attribution discipline field_audit has is a forward migration and
-- a founder decision, not something to widen the ratified shape with here.
CREATE TABLE platform_operator (
    user_id    uuid        NOT NULL REFERENCES app_user (id),
    granted_at timestamptz NOT NULL,
    granted_by text        NOT NULL,
    revoked_at timestamptz,

    PRIMARY KEY (user_id),

    CONSTRAINT platform_operator_granted_by_named CHECK (granted_by <> ''),

    -- A revocation cannot predate the grant it withdraws.
    CONSTRAINT platform_operator_revoked_after_granted CHECK (
        revoked_at IS NULL OR revoked_at >= granted_at
    )
);

-- The backoffice role's whole reach, and it is SELECT and nothing else.
--
-- tam_backoffice is not BYPASSRLS (db/init/03-backoffice-role.sql), so a
-- grant alone shows it nothing on a fenced table: the policies below are the
-- second half, and a table needs its name in both lists before one row of it
-- is visible. That is the property the cluster attribute tam_engine and
-- tam_broker hold cannot offer -- theirs applies to every table they are ever
-- granted anything on, whereas this one is revisited table by table here.
--
-- connection_secret is absent and stays absent. The vault is the broker's
-- alone (migration 0010), tam_app's own access to it was revoked there, and
-- tests/backoffice_grants.rs asserts the denial for this role empirically
-- rather than trusting this file's silence.
GRANT SELECT ON product, mapping, connection, job, job_item, write_attempt,
    org_halt, org_inventory_halt TO tam_backoffice;

-- The two global tables, granted for one reason: the organisation listing
-- counts fenced rows per tenant, and without these the same page needs a
-- second pool and a join performed in Rust. They widen nothing -- both are
-- already readable across every tenant by tam_app, which carries no policy on
-- either (migration 0014 states why), so this grant moves an existing read to
-- a different connection rather than reaching anything new.
GRANT SELECT ON organisation, app_user TO tam_backoffice;

-- The cross-tenant read, once per fenced table. Permissive policies are OR-ed
-- together, so each of these sits beside the tenant policy migration 0001
-- established rather than replacing it: tam_app still sees only its pinned
-- organisation, and only this role reads past the pin.
--
-- FOR SELECT is load-bearing. A policy written FOR ALL would extend to
-- INSERT, UPDATE and DELETE the moment anybody granted one of them, and the
-- SELECT-only property would then rest on the grant list alone.
--
-- Written out one statement per table rather than looped over an array: this
-- is the list the design note promises a reviewer can read end to end, and
-- the loop that would shorten it also hides it.
CREATE POLICY product_backoffice_read ON product
    FOR SELECT TO tam_backoffice USING (true);
CREATE POLICY mapping_backoffice_read ON mapping
    FOR SELECT TO tam_backoffice USING (true);
CREATE POLICY connection_backoffice_read ON connection
    FOR SELECT TO tam_backoffice USING (true);
CREATE POLICY job_backoffice_read ON job
    FOR SELECT TO tam_backoffice USING (true);
CREATE POLICY job_item_backoffice_read ON job_item
    FOR SELECT TO tam_backoffice USING (true);
CREATE POLICY write_attempt_backoffice_read ON write_attempt
    FOR SELECT TO tam_backoffice USING (true);
CREATE POLICY org_halt_backoffice_read ON org_halt
    FOR SELECT TO tam_backoffice USING (true);
CREATE POLICY org_inventory_halt_backoffice_read ON org_inventory_halt
    FOR SELECT TO tam_backoffice USING (true);
