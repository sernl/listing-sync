# The concrete schema

This document holds the data-definition language the design specification describes in prose.
It is a sketch rather than a migration: it shows the shape, the invariants the database enforces rather than convention, and the places where a sum type in [`sketches/domain.rs`](sketches/domain.rs) becomes a discriminant column and a check constraint.
The three governing rules — `org_id` on every tenant table, compound keys so a child's foreign key names the tenant, and tenant-scoped unique indexes — are argued in the design specification and applied without further comment below.

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

The check constraint is the SQL rendering of `PriceIntent` and rules out the two states the domain type has no variant for.

`PayloadSet` is non-empty by construction in Rust and nothing above stops a `product` row committing with zero payload-role `product_file` rows, so the invariant is closed by a deferred constraint trigger.
PostgreSQL cannot defer a `CHECK`, which is why this one is a trigger rather than a constraint.

```sql
CREATE CONSTRAINT TRIGGER product_payload_nonempty
    AFTER INSERT OR UPDATE ON product
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_product_has_payload();
```

`assert_product_has_payload()` raises unless at least one `product_file` row exists for the product with `role = 'payload'` and `deleted_at IS NULL`, and a mirror trigger on `product_file` covers the delete side.
A product and its first payload therefore land in one transaction and a payload-less product can never commit.

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

Deduplication is per tenant rather than global, because a global blob table keyed on hash alone is an existence oracle and is unachievable in any case under per-tenant file encryption.

`product_file` carries `org_id`, the product through a compound foreign key, a role of payload, preview or cover, a closed file kind, and the scan outcome with its signature and timestamp.
`product_term` joins products to canonical taxonomy terms.
`grade_declaration` holds the derived age interval and its source, and `grade_declaration_path` holds the seller's declaration verbatim as an ordered list of vocabulary terms, which is what makes the round-trip law hold by construction.

## Mapping

```sql
CREATE TABLE mapping (
    org_id             uuid        NOT NULL REFERENCES organisation (id),
    id                 uuid        NOT NULL,
    product_id         uuid        NOT NULL,
    inventory          text        NOT NULL,
    marketplace        text        NOT NULL,

    binding_state      text        NOT NULL,
    remote_id_kind     text,
    remote_url         text,
    remote_numeric_id  bigint,
    binding_attempt    uuid,
    first_seen_at      timestamptz,
    severed_at         timestamptz,
    sever_cause        text,

    verify_state       text        NOT NULL,
    verified_at        timestamptz,
    normaliser_version int         NOT NULL,

    policy_title       text        NOT NULL,
    policy_description text        NOT NULL,
    policy_price       text        NOT NULL,
    policy_taxonomy    text        NOT NULL,
    policy_grades      text        NOT NULL,
    policy_files       text        NOT NULL,

    publish_mode       text        NOT NULL,
    lifecycle_state    text        NOT NULL,
    lifecycle_since    timestamptz,

    created_at         timestamptz NOT NULL,
    updated_at         timestamptz NOT NULL,

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
    ),

    CONSTRAINT mapping_verify_total CHECK (
        (verify_state = 'stale' AND verified_at IS NULL)
     OR (verify_state IN ('clean', 'mismatched') AND verified_at IS NOT NULL)
    )
);
```

The six `policy_*` columns are the SQL rendering of `FieldPolicies`, which is a struct with one field per `FieldKey` rather than a map.
They are six `NOT NULL` columns and never a `jsonb` object, because the whole point of the struct is that a policy can be neither missing nor unknown, and a `jsonb` map reintroduces exactly the state the type exists to prevent.
Adding a syncable field is therefore a migration that adds a column and a compile error at every construction site, which is the intended cost.

`Verification::Mismatched` is non-empty by construction in Rust, and the storage half of that is a child table plus a deferred trigger, symmetric with the payload rule above.

```sql
CREATE TABLE field_mismatch (
    org_id        uuid NOT NULL REFERENCES organisation (id),
    mapping_id    uuid NOT NULL,
    field         text NOT NULL,
    class         text NOT NULL,
    observed_in   text,
    limit_observed int,

    PRIMARY KEY (org_id, mapping_id, field),
    FOREIGN KEY (org_id, mapping_id) REFERENCES mapping (org_id, id) ON DELETE CASCADE
);

CREATE CONSTRAINT TRIGGER mapping_mismatch_nonempty
    AFTER INSERT OR UPDATE ON mapping
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_mismatch_nonempty();
```

`assert_mismatch_nonempty()` raises when `verify_state = 'mismatched'` and no `field_mismatch` row exists for the mapping, so the state cannot claim a mismatch it cannot name.

## Job ledger and leases

```sql
CREATE TABLE job_item (
    org_id           uuid        NOT NULL REFERENCES organisation (id),
    id               uuid        NOT NULL,
    job_id           uuid        NOT NULL,
    mapping_id       uuid        NOT NULL,
    idempotency_key  uuid        NOT NULL,

    state            text        NOT NULL,
    outcome          text,
    failure_code     text,
    failure_detail   text,
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

`idempotency_key` is `uuid` because the derivation is UUIDv5 over a fixed-width canonical encoding keyed on the inventory, specified in [`sync-machine.md`](sync-machine.md).
`outcome` holds one of the six `ItemOutcome` values, `Degraded` included, so the client's stacked bar has no segment the ledger cannot store.
`failure_code` holds one of the fifteen `FailureCode` values and nothing else; the column and the type share one name across the worker, the API, the client and the operator dashboard.

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

`kind` is the serde tag of `JobEventKind` and holds no other value: `JobQueued`, `JobStarted`, `ItemQueued`, `ItemLeased`, `ItemActionStarted`, `ItemActionFinished`, `ItemBlocked`, `ItemParked`, `ItemResumed`, `ItemSettled`, `JobSettled`, `JobHalted`.
`payload` is the serde body of the matching `JobEventPayload` variant in `tam-types`, so the pair is one tagged union split across two columns rather than a free-form document, and a flake check asserts the distinct values of `kind` are exactly the tag set.
The vocabulary is the client's entire contract, so it is versioned and generated into TypeScript alongside `FailureCode`.

`org_seq` repairs the resume query and this is a judgement rather than a finding.
Resuming with `Last-Event-ID` and `WHERE id > $1` is unsound against an identity column, because identity values are allocated before commit, so a lower value can become visible after a higher one.
`org_seq` is allocated by locking a per-organisation counter row in the same transaction as the state change, at a cost of one row lock per event that is free because per-tenant concurrency is one to two browser sessions by design.

Retention is bounded, because the per-org sequence otherwise grows without limit and backs an unbounded resume query.
A scheduled job deletes `job_event` rows older than `JOB_EVENT_RETENTION_DAYS` and advances a per-organisation pruning watermark; `org_seq` values are never reused.
A client presenting a `Last-Event-ID` below its organisation's watermark receives a `resync` event carrying the current snapshot cursor rather than a partial replay, which is the same recovery routine as the lagged-broadcast path.

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
    ambiguity_cause    text,
    evidence_ref       text,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, job_item_id) REFERENCES job_item (org_id, id),
    FOREIGN KEY (org_id, mapping_id)  REFERENCES mapping  (org_id, id)
);

CREATE UNIQUE INDEX write_attempt_one_in_flight
    ON write_attempt (org_id, mapping_id)
    WHERE state = 'in_flight';
```

The row is written before the click rather than after the response, because the commit boundary is intent recorded rather than response received.
The partial unique index makes a duplicate-upload storm structurally impossible rather than merely unlikely, so a retry racing a hung worker fails at the database instead of at the marketplace.

## The outbox

`write_attempt` is a fencing token for a non-idempotent third-party write; `outbox_message` is at-least-once delivery to a party that offers idempotency, and the two are separate tables because they solve different problems.
Marketplace writes never travel through the outbox, Etsy included, because no marketplace in scope offers an idempotency key on a create.

```sql
CREATE TABLE outbox_message (
    org_id       uuid        NOT NULL REFERENCES organisation (id),
    id           uuid        NOT NULL,
    topic        text        NOT NULL,
    dedupe_key   text        NOT NULL,
    payload      jsonb       NOT NULL,
    state        text        NOT NULL DEFAULT 'pending',
    created_at   timestamptz NOT NULL,
    available_at timestamptz NOT NULL,
    attempts     int         NOT NULL DEFAULT 0,
    delivered_at timestamptz,
    last_error   text,

    PRIMARY KEY (org_id, id),
    CONSTRAINT outbox_dedupe UNIQUE (org_id, topic, dedupe_key),
    CONSTRAINT outbox_state_total CHECK (
        (state = 'delivered') = (delivered_at IS NOT NULL)
    )
);
```

The row's own identifier is the value sent as the downstream idempotency key, so at-least-once delivery cannot double-count.

| Topic | Destination | Drained by |
|---|---|---|
| `email.parked_job` | transactional email relay | `tam-server` |
| `email.job_settled` | transactional email relay | `tam-server` |
| `email.connection_revoked` | transactional email relay | `tam-server` |
| `push.parked_job` | VAPID Web Push endpoint | `tam-server` |
| `push.job_settled` | VAPID Web Push endpoint | `tam-server` |
| `billing.usage_event` | payment-processor meter | `tam-server` |
| `billing.subscription_sync` | payment-processor subscriptions | `tam-server` |

The drainer runs in `tam-server` rather than in the worker, because `tam-server` is the only process with egress to the relay, the push endpoint and the processor, and it holds no marketplace credentials.
The worker writes outbox rows in the same transaction as the state change that caused them and never delivers one itself.

Delivery is at-least-once and unordered, so every consumer must be order-insensitive; there is deliberately no global sequence, because ordering across topics with different destinations buys nothing and costs a serialisation point.
A failed attempt increments `attempts` and pushes `available_at` forward by an exponential backoff from `OUTBOX_BACKOFF_BASE`, doubling to a cap of `OUTBOX_BACKOFF_MAX`.
At `OUTBOX_MAX_ATTEMPTS` the row moves to `state = 'dead'` and raises a paging alert rather than being retried further, which is also the poison-message handling: a message that crashes the drainer is quarantined by attempt count alone and needs no separate detector.

One alert is not a retry and must not be implemented as one.
The payment processor accepts backdated usage events only within a bounded window, so a `billing.usage_event` row still pending past that window is a lost meter reading rather than a delayed one, and it raises an alert immediately.
The window's length is a processor-specific figure the research does not record, so it is held as configuration in `USAGE_EVENT_BACKDATE_WINDOW` and must be confirmed in writing with the processor before billing code ships.

## Halts, budgets, credentials and audit

Three halt scopes exist because the domain needs three, and they are the SQL rendering of `HaltScope`.

| Table | Scope | Raised by |
|---|---|---|
| `inventory_halt` | one inventory, every tenant | the operator, and the circuit breaker |
| `org_halt` | one tenant, every inventory | the operator, and global revocation |
| `org_inventory_halt` | one tenant, one inventory | an ambiguous create, and a `Missing` field mismatch |

`org_inventory_halt` is the one an ambiguous create raises, which is what `Binding::AmbiguousCreate` means by halting this inventory for this tenant, and it is why the limit governing it is named `AMBIGUOUS_BEFORE_INVENTORY_HALT`.
All three fail closed: a worker that cannot read them refuses to automate.

`rate_budget` is keyed `(org_id, connection_id)` rather than on the inventory, because the budget it protects is the marketplace's own per-account fair-usage counter and a connection is exactly one marketplace account.
Under the assumption that one Tes author login reaches both inventories, that means a seller's daily ceiling is shared across GB and US rather than granted twice; if the M-1 probe finds two logins, a connection becomes per inventory and the budget follows without a schema change.

`connection` holds one row per tenant per marketplace under that same assumption, which is recorded as an assumption in the design specification's open questions.
`connection_secret` holds the wrapped data-encryption key, nonce, ciphertext and AAD context keyed by tenant, connection and key version, and only the session broker's database role may select from it.

`field_audit` records intended value, observed value before and after, the mismatch class and the normaliser version that made the comparison.
It is append-only: the application role holds insert and select and neither update nor delete, and rows ship off-box continuously.

Billing keeps its own ledger as the source of truth: `billing_customer` maps an organisation to a processor customer, `subscription` holds plan and period, and `usage_event` is the append-only meter with its own dedupe key, pushed through the outbox.

## Migration discipline

Migrations are forward-only, because a down migration is written once, never exercised, and then run under duress; backing out a bad change is a new forward migration written with the failure in front of you.
Every schema change that alters an existing shape is expand, backfill, switch and contract across four deploys rather than one, with the backfill running in bounded resumable batches outside any migration transaction.
A migration never ships with the code depending on it, because a row written by an older or newer deployment is a live case during every rollout.
Locking behaviour is checked against the deployed PostgreSQL major version rather than recalled, and migrations are tested against a production-shaped snapshot restored by the backup drill, which is close to free because that drill has to exist anyway.
Offline query metadata is checked into the repository with a flake check asserting it is current, so a schema change that breaks a query fails at build time rather than at the first request.

## The decision surface and the read leg

`election_rule` and `election_item` are the seller's decision surface, in two tables because the two questions have two keys: a rule is a policy per (inventory, axis, trigger kind, trigger key) and an item is one product's open question.
The item's dedup index is partial on the open state and includes the trigger key, so a product whose price flips free to paid does not keep the stale free-branch question and have the seller answer a Creative Commons value for a paid listing — the one combination Tes refuses.
A CHECK refuses a rule that delegates a legal axis, stated over a set of axis values rather than one, so adding a second non-delegable axis has to extend the list.
The engine may raise an item and read a rule; it may never settle an item or author a rule, which mirrors the reconciliation grant exactly and is asserted rather than reviewed.

`mapping_loss` records what a projection could not carry, against the mapping it happened on.
`tam_app` has no UPDATE or DELETE on it: a loss is a fact about a write that happened, and a fact the seller can edit is not a disclosure.

`sync_request` and `sync_request_resource` hold a sync's read leg, which is not a ledger item because the item pump is deliberately adapter-free and a job carries one inventory.
The resource row carries the product and mapping its canonicalisation produced, written in the same transaction that marks it done, because the import is not internally atomic and mints a fresh product id on every pass — so without the breadcrumb a redrained request inserts a duplicate no unique index refuses.
Neither table carries an engine grant: the drain runs under the tenant's own role, which has forced row-level security.

`job_item.requires_bound_on` names an inventory whose binding this item waits on, and `mapping.sever_generation` counts how many times a mapping's listing has been severed.
The first is what keeps a migrate's removal from running before its counterpart exists; the second enters a create's intent digest so a migrate-back is a fresh idempotency key while an ordinary re-sync of unchanged content stays the no-op the content-addressed key was built for.
