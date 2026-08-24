# Data model

Superseded on 2026-08-25 by [`2026-08-25-listing-sync-design.md`](2026-08-25-listing-sync-design.md) and [`schema.md`](schema.md), which carries the corrected data-definition language.
This document is retained for its reasoning and is not live; do not implement from it.

PostgreSQL is the system of record for everything except file bytes, which live in object storage under keys the database owns.
The schema below is a sketch rather than a migration: it shows the shape, the invariants that are enforced by the database rather than by convention, and the places where a sum type in the domain model becomes a discriminant column and a check constraint.

Three rules govern the whole schema and each of them is here because it is expensive or impossible to retrofit.
Every table holding tenant data carries an organisation identifier.
Every parent table carries a compound key so that a child's foreign key can name the tenant, which makes a cross-tenant reference unrepresentable rather than merely wrong.
Every unique index is tenant-scoped, because referential-integrity checks bypass row security, so a unique constraint on a per-tenant natural key is an oracle for another tenant's rows ([PostgreSQL row security](https://www.postgresql.org/docs/current/ddl-rowsecurity.html)).

The organisation identifier is the tenant identifier; the type is `OrgId` and the column is `org_id`, and other sections that speak of a tenant identifier mean the same value.
One name is chosen at assembly rather than carried in two.

Identifiers are assigned by the application rather than by the database.
The reason is mechanical: a `write_attempt` row is the fencing token for a marketplace write and its identifier has to exist before the transaction that inserts it commits, so that the same value can be carried into the intent record and the audit rows written alongside it.

Money is stored as a signed minor-unit integer beside a currency code, never as a floating-point type and never as a bare number.
Currency is not inferable from the tenant, because a single Tes author holds GBP prices in the GB inventory and USD prices in the US inventory.

## Tenancy, and the tables that are deliberately not tenant-scoped

| Table | Scope | Holds |
|---|---|---|
| `organisation` | tenant root | the tenant itself; its `id` is the organisation identifier |
| `app_user` | global | login identity only, deliberately holding no tenant data |
| `membership` | tenant | which users may act for which organisation, and in what role |
| `marketplace_inventory` | global | reference rows for Tes GB, Tes US, Etsy and TPT |
| `canonical_term`, `vocabulary`, `vocabulary_term`, `projection_edge` | global | the product's own taxonomy, which is the asset rather than tenant data |
| `adapter_selector_pack`, `inventory_halt` | global | operational configuration and the fleet kill switch |
| everything else | tenant | `org_id NOT NULL` on every row |

The four global taxonomy tables are the one exception that needs justifying, because a tenant's reconciliation decision would otherwise silently change every other tenant's projections.
The resolution is two tables rather than a nullable tenant column: `projection_edge_proposal` carries `org_id NOT NULL` and applies only to the tenant that raised it, and promotion to the global `projection_edge` is an explicit operator act.
That keeps the every-tenant-table-carries-`org_id` rule intact in letter, and it makes the governance question visible instead of hiding it in a nullable column.

## The repository signature

The organisation identifier is a mandatory positional parameter on every repository method and is never read from task-local state.
This is the single most expensive thing in the schema to retrofit, which is why it is here on the first commit rather than in a later cleanup.

```rust
pub trait ProductRepo {
    async fn get(&self, org: OrgId, id: ProductId) -> Result<Option<CanonicalProduct>, RepoError>;
    async fn page(&self, org: OrgId, cursor: Cursor) -> Result<Page<CanonicalProduct>, RepoError>;
    async fn upsert(&self, org: OrgId, product: &CanonicalProduct) -> Result<(), RepoError>;
}
```

Three properties follow from the parameter being positional and first, and none of them survives a task-local.
A missing tenant is a compile error rather than a runtime default.
A reviewer reading a call site can see the tenant without reading the caller, which matters when most call sites are written by an agent.
And a background job, which has no inbound request to inherit a task-local from, is forced to name the tenant it is acting for, which is exactly the case where a defaulted task-local produces a cross-tenant write that no test covers.

The database backstop reads the tenant from a session setting, and that is not a contradiction of the rule above.

```sql
ALTER TABLE product ENABLE ROW LEVEL SECURITY;
ALTER TABLE product FORCE ROW LEVEL SECURITY;

CREATE POLICY product_tenant ON product
    USING (org_id = current_setting('app.org_id')::uuid);
```

`FORCE` is load-bearing rather than decorative, because table owners normally bypass row security, so a policy without it silently does nothing under the obvious setup.
The application connects as a non-owner role that does not hold `BYPASSRLS`, and the session setting is written with `SET LOCAL` at the single point where a transaction is opened, from the same positional parameter the repository method received.
One writer, one read, no drift.
The primary control is the parameter; the policy is what catches the query the parameter forgot to reach.

The test that proves it is a two-tenant integration test asserting that a select under the application role returns zero rows for another tenant's data, run against a real database.
A `trybuild` compile-fail test was considered and rejected: its output is brittle across toolchain bumps and the integration test gets most of the value for a fraction of the upkeep.

## Catalogue

```sql
CREATE TABLE product (
    org_id            uuid        NOT NULL REFERENCES organisation (id),
    id                uuid        NOT NULL,
    title             text        NOT NULL,
    body              text        NOT NULL,
    price_kind        text        NOT NULL,
    price_minor_units bigint,
    price_currency    text,
    created_at        timestamptz NOT NULL,
    updated_at        timestamptz NOT NULL,
    deleted_at        timestamptz,

    PRIMARY KEY (org_id, id),

    CONSTRAINT product_price_total CHECK (
        (price_kind = 'free' AND price_minor_units IS NULL AND price_currency IS NULL)
     OR (price_kind = 'paid' AND price_minor_units > 0 AND price_currency IS NOT NULL)
    )
);
```

The check constraint is the SQL rendering of `PriceIntent`.
It rules out the two states the domain type has no variant for: a free product carrying a number, and a paid product carrying zero or a null currency.

```sql
CREATE TABLE blob (
    org_id          uuid        NOT NULL REFERENCES organisation (id),
    hash            bytea       NOT NULL,
    byte_len        bigint      NOT NULL,
    object_key      text        NOT NULL,
    dek_key_version int         NOT NULL,
    first_seen_at   timestamptz NOT NULL,

    PRIMARY KEY (org_id, hash)
);
```

Content-addressed deduplication is deliberately per tenant rather than global, and this is the least obvious decision in the schema.
A global blob table keyed on hash alone is an existence oracle: an upload that returns instantly tells the uploader that another tenant already holds that exact file.
It is also unachievable in any case, because customers' resource files are the asset that cannot be rotated after disclosure and are therefore encrypted under per-tenant data-encryption keys, so two tenants holding the same plaintext hold different ciphertext.
The cost is real and small at this scale, where storage was modelled at a few cents per user per month against a median product file of about 15 MB.

`product_file` carries `org_id`, the product it belongs to through a compound foreign key, a role of payload, preview or cover, a closed file kind, and the ClamAV scan outcome with its signature and timestamp.
`product_term` joins products to canonical taxonomy terms.
`grade_declaration` holds the derived age interval and its source, and `grade_declaration_path` holds the seller's declaration verbatim as an ordered list of vocabulary terms, which is what makes the round-trip law in the domain model hold by construction.

## Mapping, and how a sum type becomes a table

```sql
CREATE TABLE mapping (
    org_id            uuid        NOT NULL REFERENCES organisation (id),
    id                uuid        NOT NULL,
    product_id        uuid        NOT NULL,
    inventory         text        NOT NULL,
    marketplace       text        NOT NULL,

    binding_state     text        NOT NULL,
    remote_id_kind    text,
    remote_url        text,
    remote_numeric_id bigint,
    binding_attempt   uuid,
    first_seen_at     timestamptz,
    severed_at        timestamptz,
    sever_cause       text,

    verify_state      text        NOT NULL,
    verified_at       timestamptz,
    normaliser_version int        NOT NULL,

    policy_title       text NOT NULL,
    policy_description text NOT NULL,
    policy_price       text NOT NULL,
    policy_taxonomy    text NOT NULL,
    policy_grades      text NOT NULL,
    policy_files       text NOT NULL,

    publish_mode      text        NOT NULL,
    lifecycle_state   text        NOT NULL,
    lifecycle_since   timestamptz,

    created_at        timestamptz NOT NULL,
    updated_at        timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),
    FOREIGN KEY (inventory, marketplace)
        REFERENCES marketplace_inventory (code, marketplace),

    CONSTRAINT mapping_one_per_inventory UNIQUE (org_id, product_id, inventory),

    CONSTRAINT mapping_binding_total CHECK (
        CASE binding_state
            WHEN 'unbound'          THEN remote_id_kind IS NULL AND binding_attempt IS NULL
            WHEN 'creating'         THEN remote_id_kind IS NULL AND binding_attempt IS NOT NULL
            WHEN 'bound'            THEN remote_id_kind IS NOT NULL AND first_seen_at IS NOT NULL
            WHEN 'ambiguous_create' THEN remote_id_kind IS NULL AND binding_attempt IS NOT NULL
            WHEN 'severed'          THEN remote_id_kind IS NOT NULL
                                     AND severed_at IS NOT NULL
                                     AND sever_cause IS NOT NULL
            ELSE false
        END
    ),

    CONSTRAINT mapping_remote_id_shape CHECK (
        remote_id_kind IS NULL
     OR (remote_id_kind = 'tes'
         AND remote_url IS NOT NULL AND remote_numeric_id IS NULL)
     OR (remote_id_kind IN ('tpt', 'etsy')
         AND remote_numeric_id IS NOT NULL AND remote_url IS NULL)
    ),

    CONSTRAINT mapping_remote_id_marketplace CHECK (
        remote_id_kind IS NULL OR remote_id_kind = marketplace
    )
);
```

The `ELSE false` arm is the point of the first check.
It closes the enumeration, so a new binding state added without a matching arm is rejected at insert rather than accepted with whatever columns the writer happened to fill, which is the behaviour a sum type gives for free in Rust and that a discriminant column does not.

The third check earns the denormalised `marketplace` column.
It makes the one invariant a mapping can violate unaided, holding a durable identifier minted by a different marketplace, into a database constraint rather than a Rust method that a background job might not call.
The compound foreign key into `marketplace_inventory` keeps the denormalisation honest.

`mapping_one_per_inventory` is the enforcement point for the duplicate-listing rules on both marketplaces: TPT states each resource may be listed only once, and the Tes Author Code asks authors not to upload duplicate copies.
It is scoped by `org_id`, which is both correct and required by the tenant-scoped-unique-index rule.

## Job ledger, leases and the outbox

```sql
CREATE TABLE job_item (
    org_id           uuid        NOT NULL REFERENCES organisation (id),
    id               uuid        NOT NULL,
    job_id           uuid        NOT NULL,
    mapping_id       uuid        NOT NULL,
    idempotency_key  uuid        NOT NULL,

    state            text        NOT NULL,
    outcome          text,
    reason_code      text,
    reason_detail    text,
    blocked_on       text,
    evidence_ref     text,
    attempt_count    int         NOT NULL DEFAULT 0,

    lease_owner      text,
    lease_epoch      bigint      NOT NULL DEFAULT 0,
    lease_expires_at timestamptz,
    park_expires_at  timestamptz,

    created_at       timestamptz NOT NULL,
    settled_at       timestamptz,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, job_id)     REFERENCES job (org_id, id),
    FOREIGN KEY (org_id, mapping_id) REFERENCES mapping (org_id, id),

    CONSTRAINT job_item_idempotent UNIQUE (org_id, idempotency_key),

    CONSTRAINT job_item_lease_total CHECK (
        (state IN ('leased', 'running', 'verifying'))
      = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
    ),

    CONSTRAINT job_item_settled_total CHECK (
        (state = 'settled') = (outcome IS NOT NULL AND settled_at IS NOT NULL)
    )
);
```

The lease is per item and carries a deadline, not per worker and keyed on a heartbeat.
That is the reason the queue is hand-written against a Postgres lease table rather than adopted from a crate.
The leading candidate, `apalis`, re-enqueues orphaned jobs on worker heartbeat timeout with the predicate entirely on the worker row and no per-job deadline column, and the dominant failure mode here is a browser that hangs while its process stays healthy, which such a design is structurally blind to.
`lease_epoch` is the fencing token: a steal increments it, a worker's writes carry the epoch it leased at, and a resurrected worker's write is rejected rather than racing the new one.

```sql
CREATE TABLE job_event (
    id          bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    org_seq     bigint      NOT NULL,
    job_id      uuid        NOT NULL,
    job_item_id uuid,
    kind        text        NOT NULL,
    payload     jsonb       NOT NULL,
    created_at  timestamptz NOT NULL,

    FOREIGN KEY (org_id, job_id) REFERENCES job (org_id, id),
    CONSTRAINT job_event_org_seq UNIQUE (org_id, org_seq)
);
```

`org_seq` exists to repair a hole in the streaming design and this is a judgement rather than a finding, so it is stated as one.
The progress stream resumes with `Last-Event-ID` and a `WHERE id > $1` query, which is unsound against an identity column: identity values are allocated before commit, so a lower value can become visible after a higher one, and a client that resumed at the higher value never sees the lower.
`org_seq` is allocated by locking a per-organisation counter row in the same transaction as the state change, which serialises event insertion per tenant and makes the cursor a scalar the client can carry across every job it is watching.
The cost is one row lock per event, which is free here because per-tenant concurrency is one to two browser sessions by design and inter-tenant parallelism is unaffected.

```sql
CREATE TABLE write_attempt (
    org_id             uuid        NOT NULL REFERENCES organisation (id),
    id                 uuid        NOT NULL,
    job_item_id        uuid        NOT NULL,
    mapping_id         uuid        NOT NULL,
    lease_epoch        bigint      NOT NULL,
    intent             jsonb       NOT NULL,
    intent_hash        bytea       NOT NULL,
    correlation_marker text,
    state              text        NOT NULL,
    opened_at          timestamptz NOT NULL,
    settled_at         timestamptz,
    remote_id_kind     text,
    remote_url         text,
    remote_numeric_id  bigint,
    failure_code       text,
    evidence_ref       text,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, job_item_id) REFERENCES job_item (org_id, id),
    FOREIGN KEY (org_id, mapping_id)  REFERENCES mapping  (org_id, id)
);

CREATE UNIQUE INDEX write_attempt_one_in_flight
    ON write_attempt (org_id, mapping_id)
    WHERE state = 'in_flight';
```

The row is written before the click, not after the response, because the commit boundary is intent recorded rather than response received.
The partial unique index is what makes a duplicate-upload storm structurally impossible rather than merely unlikely: a second in-flight attempt against the same mapping cannot be inserted, so a retry that races a hung worker fails at the database instead of at the marketplace.

The outbox is separate from the write attempt, because they solve different problems: `write_attempt` is a fencing token for a non-idempotent third-party write, and `outbox_message` is at-least-once delivery to a party that offers idempotency.

```sql
CREATE TABLE outbox_message (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    id           uuid        NOT NULL,
    topic        text        NOT NULL,
    dedupe_key   text        NOT NULL,
    payload      jsonb       NOT NULL,
    created_at   timestamptz NOT NULL,
    available_at timestamptz NOT NULL,
    attempts     int         NOT NULL DEFAULT 0,
    delivered_at timestamptz,
    last_error   text,

    PRIMARY KEY (org_id, id),
    CONSTRAINT outbox_dedupe UNIQUE (org_id, topic, dedupe_key)
);
```

The row's own identifier is the value sent as the downstream idempotency key, so at-least-once delivery cannot double-count.
For usage metering specifically this matters twice over, because the payment processor's metering surface accepts backdated events only within a bounded window, so a message that has been stuck past that window is an alert rather than a retry.

## Per-field audit, credentials and billing

`field_audit` records intended value, observed value before and after, the mismatch class, and the version of the normaliser that made the comparison.
It is append-only: the application role holds insert and select and neither update nor delete, and rows are shipped off-box continuously to an append-only sink, because an attacker with root on the only machine can rewrite any local log regardless of hash chaining.
It carries more weight than an ordinary audit trail because TPT gives a seller only the timestamp of a delegated edit and not its content, so this log is the only record either party will hold.
Recording the normaliser version alongside the comparison is what lets a change in our own normalisation be told apart from a change at the marketplace.

`connection` holds one row per tenant per marketplace, not per inventory, because one Tes login reaches both inventories.
`connection_secret` holds the wrapped data-encryption key, the nonce, the ciphertext and the additional-authenticated-data context, keyed by tenant, connection and key version.
The AAD binds tenant, marketplace, connection and key version together, so a ciphertext row replayed into another tenant fails authentication rather than decrypting: a cryptographic backstop for tenancy that holds even if row security is misconfigured.
Only the session broker's database role may select from that table, and the broker is the only process that can reach the key-encryption key.

Billing keeps its own ledger as the source of truth rather than treating the processor's records as authoritative.
`billing_customer` maps an organisation to a processor customer, `subscription` holds plan and period, and `usage_event` is the append-only meter with its own dedupe key, pushed to the processor through the outbox.
The reason for owning the ledger is that the processor's metering product is in flux and its own documentation now points new integrations elsewhere, so the push is a thin replaceable adapter and the ledger is not.

`rate_budget` holds the per-tenant, per-inventory write ceiling, server-controlled so it can be lowered fleet-wide without a release, starting new accounts conservatively and rising with account age and clean history.
`inventory_halt` and `org_halt` are the two halves of the kill switch, global and per tenant, and both fail closed.

## Migration discipline

Migrations are forward-only.
There are no down migrations, because a down migration is written once, never exercised, and then run under duress; backing out a bad change is a new forward migration written with the failure in front of you.

Every schema change that alters an existing shape is expand and contract, and the four steps are four deploys rather than one.
Expand adds the new column or table, nullable and unread.
Backfill populates it in bounded resumable batches outside any migration transaction, so a long backfill cannot hold a lock or block a deploy.
Switch moves reads to the new shape while writes go to both.
Contract drops the old shape once no running deployment reads it.
A migration and the code that depends on it never ship in the same step, because a row written by an older or a newer deployment is a live case during every rollout and the schema has to tolerate both.

Locking behaviour is the part that takes a site down, and it is checked against the deployed PostgreSQL major version rather than recalled.
Statements that rewrite a table or take an exclusive lock hold it for the duration of the rewrite, and index builds that avoid that lock cannot run inside a transaction, which means a migration runner that wraps each file in one cannot run them at all.
Both facts are verified against the running version before the first migration that needs either, and the verification is recorded beside the migration.

Migrations are tested against a production-shaped snapshot rather than an empty schema, which is the only way the timing and the lock waits are real.
This is close to free, because a tested restore has to exist anyway: Article 32(1)(c) requires "the ability to restore the availability and access to personal data in a timely manner" and 32(1)(d) requires "regularly testing" it, so an untested backup is documented non-compliance rather than merely a risk ([GDPR Article 32](https://gdpr-info.eu/art-32-gdpr/)).
The drill restores the most recent backup into a scratch database, runs the pending migrations against it, and records wall-clock duration and lock waits.

Query metadata for offline compilation is checked into the repository and a flake check asserts it is current, so a schema change that breaks a query fails at build time rather than at the first request that runs it.
