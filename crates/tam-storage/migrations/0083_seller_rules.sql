-- The seller's own pricing and mapping policy, the previews they decide on,
-- and the frozen copies of both that queued work is carried out against.
--
-- Seven tables in three groups, and the grouping is the whole design:
--
--   * `seller_rule` and `seller_rule_reference` are the living policy. A rule
--     is editable and revisioned; a reference quote is an official rate this
--     deployment fetched once and never rewrites.
--   * `seller_rule_preview` and `seller_rule_preview_row` are one proposal put
--     to the seller. Each row carries what the proposal was computed from --
--     the source fingerprint, the rule revisions, the target choice that was
--     already there -- so a confirmation arriving after any of them moved is
--     refused rather than written against facts nobody saw.
--   * `seller_rule_application`, `job_item_rule_output` and
--     `sync_request_rule_snapshot` (with its `_choice` child) are the record.
--     An application is what the seller accepted, appended and never edited;
--     the other two are frozen copies taken at the moment work was queued, so
--     editing a rule afterwards cannot change what a queued item publishes.
--
-- Money never travels as JSON here. Every amount is a
-- (kind, minor_units, currency) triple with a CHECK making the split total,
-- for the same reason `product` carries one: a price inside a jsonb blob is a
-- price no constraint can keep consistent and no column type can keep from
-- being a float on the way back in.
--
-- The immutability the second and third groups claim is a trigger, not a
-- convention. `seller_rule_immutable()` raises on UPDATE, which is what makes
-- "append-only" and "frozen" statements about the database rather than about
-- the code that happens to write it today. DELETE is left alone: an item's
-- output and a request's snapshot are cascaded away with the row they
-- describe, and pruning a settled item must not be blocked by a receipt.

-- A rule as it stands now. Revisions are a counter rather than a history
-- table: everything that has to survive an edit -- a preview's captured
-- definitions, an application's matches, a request's snapshot -- holds its own
-- copy, so an old revision is never read back from here and a table of them
-- would be a second place for the same facts to drift.
CREATE TABLE seller_rule (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    id          uuid        NOT NULL,
    -- Bumped on every accepted edit. A client's stale revision is the
    -- difference between "your edit lost a race" and "that rule is gone",
    -- which the repository reports as two different answers.
    revision    bigint      NOT NULL,

    kind        text        NOT NULL,
    title       text        NOT NULL,
    description text        NOT NULL,
    enabled     boolean     NOT NULL,
    -- Inventory codes, unconstrained by a foreign key for the reason
    -- `sync_request.source` gives: the repository's round trip through
    -- `InventoryId` is what admits a value here.
    source      text        NOT NULL,
    target      text        NOT NULL,
    -- The uses this rule applies to without being asked, as an array so the
    -- runtime's "enabled rules for a copy" is one indexed containment test
    -- rather than a jsonb scan. Empty is the default and means the rule only
    -- ever runs in a preview the seller asked for.
    auto_apply  text[]      NOT NULL,

    -- The validated definition, whole. The columns above are projections of
    -- it and the CHECKs below keep them from disagreeing about the two that
    -- are cheap to state; the rest is the repository's round trip.
    definition  jsonb       NOT NULL,

    -- Lowercased title and description as one haystack, so a console search
    -- is a single predicate against a stored column rather than a pair of
    -- `lower()` calls per row. Deliberately unindexed: a leading-wildcard
    -- ILIKE cannot use a btree and pg_trgm is not an extension this
    -- deployment installs, so the scan is honest and bounded by the tenant's
    -- own rule count, which is tens.
    search_text text GENERATED ALWAYS AS (lower(title || ' ' || description)) STORED,

    author      uuid        NOT NULL REFERENCES app_user (id),
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT seller_rule_kind CHECK (kind IN ('pricing', 'mapping')),
    CONSTRAINT seller_rule_revision_positive CHECK (revision >= 1),
    CONSTRAINT seller_rule_title_bounded CHECK (char_length(title) BETWEEN 1 AND 200),
    CONSTRAINT seller_rule_description_bounded CHECK (char_length(description) <= 2000),
    CONSTRAINT seller_rule_definition_object CHECK (jsonb_typeof(definition) = 'object'),
    CONSTRAINT seller_rule_directions_differ CHECK (source <> target),
    -- The array is a set of known uses, not free text: a value nothing reads
    -- is a rule that silently never fires.
    CONSTRAINT seller_rule_auto_apply_known CHECK (
        auto_apply <@ ARRAY['copy', 'move', 'cross_list']::text[]
    ),
    -- The projections really are projections.
    CONSTRAINT seller_rule_definition_agrees CHECK (
        definition ->> 'source' IS NOT NULL
        AND definition ->> 'target' IS NOT NULL
        AND (definition -> 'action' ->> 'kind') = kind
    )
);

CREATE INDEX seller_rule_by_direction
    ON seller_rule (org_id, source, target, enabled);
CREATE INDEX seller_rule_by_kind
    ON seller_rule (org_id, kind, enabled);
-- The runtime's read: enabled, this direction, containing this use.
CREATE INDEX seller_rule_auto_apply
    ON seller_rule USING gin (auto_apply);

-- One official quote, fetched once and never rewritten.
--
-- Tenant-owned rather than global because the fetch happens on a seller's
-- request and the rate they were shown is part of what they approved; a
-- shared table would let one tenant's refresh change the number another
-- tenant's pending preview claims to have used.
--
-- The natural key includes the rate, so an identical refetch returns the row
-- already there instead of minting a second identifier for one fact, while a
-- same-day correction is a second observation with its own identity rather
-- than a silent answer of the superseded amount. Both stay, both immutable: a
-- rule approved against the first was approved against that number, and a
-- rule approved against the correction has to be able to name it.
CREATE TABLE seller_rule_reference (
    org_id          uuid        NOT NULL REFERENCES organisation (id),
    id              uuid        NOT NULL,
    source_currency text        NOT NULL,
    target_currency text        NOT NULL,
    -- The rate as an integer, in millionths. A rule's own rate is stored the
    -- same way inside its definition, so a quote and a hand-typed estimate
    -- are the same kind of number and the arithmetic has one form.
    rate_micros     bigint      NOT NULL,
    as_of           date        NOT NULL,
    provider        text        NOT NULL,
    source_url      text        NOT NULL,
    fetched_at      timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT seller_rule_reference_natural
        UNIQUE (org_id, provider, source_currency, target_currency, as_of, rate_micros),
    CONSTRAINT seller_rule_reference_rate_positive CHECK (rate_micros > 0),
    CONSTRAINT seller_rule_reference_currencies CHECK (
        source_currency IN ('gbp', 'usd') AND target_currency IN ('gbp', 'usd')
        AND source_currency <> target_currency
    ),
    CONSTRAINT seller_rule_reference_provider_bounded
        CHECK (char_length(provider) BETWEEN 1 AND 64),
    CONSTRAINT seller_rule_reference_url_bounded
        CHECK (char_length(source_url) BETWEEN 1 AND 2048)
);

-- One proposal, as it was put to the seller.
--
-- `captured_rules` holds the definitions whole, not their identifiers: the
-- preview was computed against these, a confirmation is checked against
-- these, and a rule edited in between is a stale confirmation rather than a
-- silently different proposal. There is deliberately no foreign key onto
-- `seller_rule` -- deleting a rule must not delete the record of what it once
-- proposed, and must not be blocked by it either.
CREATE TABLE seller_rule_preview (
    org_id         uuid        NOT NULL REFERENCES organisation (id),
    id             uuid        NOT NULL,
    source         text        NOT NULL,
    target         text        NOT NULL,
    request        jsonb       NOT NULL,
    captured_rules jsonb       NOT NULL,
    -- Whether the captured set was named by the client or derived from
    -- "every enabled rule in this direction". The distinction survives into
    -- the confirmation: an implicit set must be rechecked against the
    -- direction as it now stands, because a rule enabled since the preview
    -- would have matched and the seller never saw it. An explicit set is
    -- exactly the rules the client named and nothing else can join it.
    rule_scope     text        NOT NULL,
    actor          uuid        NOT NULL REFERENCES app_user (id),
    created_at     timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT seller_rule_preview_request_object CHECK (jsonb_typeof(request) = 'object'),
    CONSTRAINT seller_rule_preview_rules_array CHECK (jsonb_typeof(captured_rules) = 'array'),
    CONSTRAINT seller_rule_preview_scope CHECK (rule_scope IN ('implicit', 'explicit')),
    CONSTRAINT seller_rule_preview_directions_differ CHECK (source <> target)
);

-- One resource inside one proposal, and the three facts a confirmation is
-- rechecked against.
--
-- `source_fingerprint` is a digest of what the evaluator actually read off the
-- resource -- its title, body, price, rights, terms and grades -- rather than
-- the product's `updated_at`, which a caller could pass stale and which moves
-- for edits the evaluation never looked at.
--
-- `previous_choice` is the application row that was the resource's target
-- choice when the preview was computed, and NULL is the positive statement
-- that there was none. Either way a second approval landing in between makes
-- this confirmation stale: the seller decided about a "before" that is no
-- longer the before.
--
-- `proposed` and `patch` are two different facts and both are needed.
-- `proposed` is the merged choice the seller was shown -- what the resource
-- would carry, inherited fields included -- and `patch` is what this
-- evaluation actually changed. The approval draws field authorship from
-- `patch` alone: a field the seller merely saw is not a field they decided,
-- and attributing an inherited licence to a price approver would restate
-- somebody else's rights decision as theirs.
CREATE TABLE seller_rule_preview_row (
    org_id             uuid        NOT NULL REFERENCES organisation (id),
    preview_id         uuid        NOT NULL,
    product_id         uuid        NOT NULL,

    title              text        NOT NULL,
    source_price_kind  text        NOT NULL,
    source_price_minor bigint,
    source_price_ccy   text,

    source_fingerprint uuid        NOT NULL,
    previous_choice    uuid,

    before             jsonb       NOT NULL,
    proposed           jsonb       NOT NULL,
    patch              jsonb       NOT NULL,
    matches            jsonb       NOT NULL,
    blockers           jsonb       NOT NULL,

    status             text        NOT NULL,
    decision           text        NOT NULL,
    decided_at         timestamptz,
    decided_by         uuid        REFERENCES app_user (id),

    PRIMARY KEY (org_id, preview_id, product_id),
    FOREIGN KEY (org_id, preview_id) REFERENCES seller_rule_preview (org_id, id) ON DELETE CASCADE,
    -- Deliberately no foreign key onto `product`. A preview row is a record
    -- of what the seller was shown, and its identity has to outlive the
    -- resource: if a resource disappears between the preview and the
    -- confirmation, the confirmation must say so -- `Missing`, refused, no
    -- partial write -- rather than have the row quietly removed underneath
    -- it and an `all` confirmation report success over a smaller set than
    -- the seller approved.

    CONSTRAINT seller_rule_preview_row_status
        CHECK (status IN ('proposed', 'unchanged', 'blocked')),
    CONSTRAINT seller_rule_preview_row_decision
        CHECK (decision IN ('pending', 'accepted', 'rejected')),
    -- A decision and its provenance arrive together or not at all.
    CONSTRAINT seller_rule_preview_row_decided_total CHECK (
        (decision = 'pending') = (decided_at IS NULL AND decided_by IS NULL)
    ),
    -- A blocked row is the one thing an accept may not reach. Stated here as
    -- well as in the repository, because "blocked rows cannot be accepted" is
    -- an invariant about the record and not only a branch in one function.
    CONSTRAINT seller_rule_preview_row_blocked_not_accepted
        CHECK (NOT (status = 'blocked' AND decision = 'accepted')),
    CONSTRAINT seller_rule_preview_row_price_total CHECK (
        (source_price_kind = 'free' AND source_price_minor IS NULL AND source_price_ccy IS NULL)
        OR (source_price_kind = 'paid' AND source_price_minor IS NOT NULL AND source_price_ccy IS NOT NULL)
    ),
    CONSTRAINT seller_rule_preview_row_price_positive
        CHECK (source_price_minor IS NULL OR source_price_minor > 0),
    CONSTRAINT seller_rule_preview_row_matches_array CHECK (jsonb_typeof(matches) = 'array'),
    CONSTRAINT seller_rule_preview_row_blockers_array CHECK (jsonb_typeof(blockers) = 'array'),
    CONSTRAINT seller_rule_preview_row_before_object CHECK (jsonb_typeof(before) = 'object'),
    CONSTRAINT seller_rule_preview_row_proposed_object CHECK (jsonb_typeof(proposed) = 'object'),
    CONSTRAINT seller_rule_preview_row_patch_object CHECK (jsonb_typeof(patch) = 'object')
);

CREATE INDEX seller_rule_preview_row_pending
    ON seller_rule_preview_row (org_id, preview_id, decision);

-- What the seller accepted, appended.
--
-- Every row is the resource's whole target choice at that moment, not the
-- patch that produced it: accepting a price for a resource whose licence was
-- approved last week writes both, with the licence's own author carried
-- across. That is what makes "the latest row is the current choice" a single
-- indexed read, and it is also the only way separate field authorship
-- survives -- a patch-only row would leave the reader to fold a history and
-- guess which approval each field came from.
--
-- `sequence` is per resource and direction, monotonic, assigned under the
-- product's own lock, so two concurrent approvals of one resource cannot both
-- believe they are the latest.
CREATE TABLE seller_rule_application (
    org_id                   uuid        NOT NULL REFERENCES organisation (id),
    id                       uuid        NOT NULL,
    product_id               uuid        NOT NULL,
    -- Provenance, not key. Which catalogue the proposal was computed from is
    -- worth keeping and is not part of the resource's identity in the
    -- target: a listing on TES has one price and one licence, whoever it was
    -- derived from, and keying on the pair would let two sources hold two
    -- contradictory current choices for one listing.
    source                   text        NOT NULL,
    target                   text        NOT NULL,
    sequence                 bigint      NOT NULL,

    price_kind               text,
    price_minor_units        bigint,
    price_currency           text,
    price_author             uuid        REFERENCES app_user (id),

    licence_native_id        text,
    licence_author           uuid        REFERENCES app_user (id),

    resource_type_native_id  text,
    resource_type_author     uuid        REFERENCES app_user (id),

    matches                  jsonb       NOT NULL,
    preview_id               uuid,
    author                   uuid        NOT NULL REFERENCES app_user (id),
    approved_at              timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),
    FOREIGN KEY (org_id, preview_id) REFERENCES seller_rule_preview (org_id, id),

    CONSTRAINT seller_rule_application_ordered
        UNIQUE (org_id, product_id, target, sequence),
    CONSTRAINT seller_rule_application_sequence_positive CHECK (sequence >= 1),
    CONSTRAINT seller_rule_application_directions_differ CHECK (source <> target),

    -- The money triple, plus its author, is present or absent as a whole.
    CONSTRAINT seller_rule_application_price_total CHECK (
        price_kind IS NULL
        OR (price_kind = 'free' AND price_minor_units IS NULL AND price_currency IS NULL)
        OR (price_kind = 'paid' AND price_minor_units IS NOT NULL AND price_currency IS NOT NULL)
    ),
    CONSTRAINT seller_rule_application_price_authored
        CHECK ((price_kind IS NULL) = (price_author IS NULL)),
    CONSTRAINT seller_rule_application_price_positive
        CHECK (price_minor_units IS NULL OR price_minor_units > 0),
    CONSTRAINT seller_rule_application_currency_known
        CHECK (price_currency IS NULL OR price_currency IN ('gbp', 'usd')),

    -- Each native field carries the author who approved that field, which is
    -- what a later price-only approval must not overwrite.
    CONSTRAINT seller_rule_application_licence_authored
        CHECK ((licence_native_id IS NULL) = (licence_author IS NULL)),
    CONSTRAINT seller_rule_application_resource_type_authored
        CHECK ((resource_type_native_id IS NULL) = (resource_type_author IS NULL)),

    -- A row stating nothing is not an approval of anything.
    CONSTRAINT seller_rule_application_states_something CHECK (
        price_kind IS NOT NULL
        OR licence_native_id IS NOT NULL
        OR resource_type_native_id IS NOT NULL
    ),
    CONSTRAINT seller_rule_application_matches_array CHECK (jsonb_typeof(matches) = 'array')
);

-- The latest-choice read, which is the runtime's hot path.
CREATE INDEX seller_rule_application_latest
    ON seller_rule_application (org_id, product_id, target, sequence DESC);

-- What one queued item will publish, frozen at the instant it was queued.
--
-- Keyed on the item and cascading with it, so the snapshot cannot outlive the
-- work it describes and pruning a settled item is not blocked by a receipt.
-- The price is mandatory and typed: every non-remove item gets an exact
-- amount, and an engine reading it twice for one lease gets the same number
-- both times because nothing here is recomputed.
CREATE TABLE job_item_rule_output (
    org_id                  uuid   NOT NULL REFERENCES organisation (id),
    item_id                 uuid   NOT NULL,

    price_kind              text   NOT NULL,
    price_minor_units       bigint,
    price_currency          text,

    licence_native_id       text,
    licence_author          uuid   REFERENCES app_user (id),
    resource_type_native_id text,
    resource_type_author    uuid   REFERENCES app_user (id),

    matches                 jsonb  NOT NULL,

    PRIMARY KEY (org_id, item_id),
    FOREIGN KEY (org_id, item_id) REFERENCES job_item (org_id, id) ON DELETE CASCADE,

    CONSTRAINT job_item_rule_output_price_total CHECK (
        (price_kind = 'free' AND price_minor_units IS NULL AND price_currency IS NULL)
        OR (price_kind = 'paid' AND price_minor_units IS NOT NULL AND price_currency IS NOT NULL)
    ),
    CONSTRAINT job_item_rule_output_price_positive
        CHECK (price_minor_units IS NULL OR price_minor_units > 0),
    CONSTRAINT job_item_rule_output_currency_known
        CHECK (price_currency IS NULL OR price_currency IN ('gbp', 'usd')),
    CONSTRAINT job_item_rule_output_licence_authored
        CHECK ((licence_native_id IS NULL) = (licence_author IS NULL)),
    CONSTRAINT job_item_rule_output_resource_type_authored
        CHECK ((resource_type_native_id IS NULL) = (resource_type_author IS NULL)),
    CONSTRAINT job_item_rule_output_matches_array CHECK (jsonb_typeof(matches) = 'array')
);

-- The rules and choices a sync, copy or move was confirmed against.
--
-- A request's native reads can finish minutes later, and the evaluation that
-- follows canonicalisation must use what the seller confirmed rather than
-- whatever the rule has since been edited to. Definitions travel whole for
-- the same reason the preview's do.
CREATE TABLE sync_request_rule_snapshot (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    request_id  uuid        NOT NULL,
    source      text        NOT NULL,
    target      text        NOT NULL,
    rules       jsonb       NOT NULL,
    captured_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, request_id),
    FOREIGN KEY (org_id, request_id) REFERENCES sync_request (org_id, id) ON DELETE CASCADE,

    CONSTRAINT sync_request_rule_snapshot_rules_array CHECK (jsonb_typeof(rules) = 'array'),
    CONSTRAINT sync_request_rule_snapshot_directions_differ CHECK (source <> target)
);

-- The per-resource confirmed policy answer as it stood at confirmation. A
-- child table rather than a column on the snapshot, because these carry money
-- and money does not go in jsonb.
--
-- Two shapes, told apart by `application_id` rather than by a kind column or
-- a second table, because the identity *is* the distinction.
--
--   * With an identity, the row is an approval this request carries forward:
--     the seller's own accepted choice, whose price may be absent (a licence
--     was approved and no price was) and whose price, where present, has an
--     author.
--   * Without one, the row is an output evaluated at confirmation and frozen:
--     a resource already in this catalogue, whose exact price the drain must
--     publish even if the source price is edited in between. The amount is
--     mandatory and carries no author, because it is what the rules computed
--     rather than something a person granted. Its native values keep their
--     own authors, because those are approvals.
--
-- A fabricated approval identity would have been the alternative, and it
-- would have made an automatic amount indistinguishable from a manual grant
-- in the one table whose whole job is recording which is which.
CREATE TABLE sync_request_rule_choice (
    org_id                  uuid   NOT NULL REFERENCES organisation (id),
    request_id              uuid   NOT NULL,
    product_id              uuid   NOT NULL,
    application_id          uuid,

    price_kind              text,
    price_minor_units       bigint,
    price_currency          text,
    price_author            uuid   REFERENCES app_user (id),

    licence_native_id       text,
    licence_author          uuid   REFERENCES app_user (id),
    resource_type_native_id text,
    resource_type_author    uuid   REFERENCES app_user (id),

    matches                 jsonb  NOT NULL,
    approved_at             timestamptz NOT NULL,

    PRIMARY KEY (org_id, request_id, product_id),
    FOREIGN KEY (org_id, request_id)
        REFERENCES sync_request_rule_snapshot (org_id, request_id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),

    CONSTRAINT sync_request_rule_choice_price_total CHECK (
        price_kind IS NULL
        OR (price_kind = 'free' AND price_minor_units IS NULL AND price_currency IS NULL)
        OR (price_kind = 'paid' AND price_minor_units IS NOT NULL AND price_currency IS NOT NULL)
    ),
    -- An approval's price carries an author; an output's price never does,
    -- and an output without a price is not an output at all. Both halves are
    -- stated here so neither shape can be written half-formed.
    CONSTRAINT sync_request_rule_choice_price_authored CHECK (
        CASE WHEN application_id IS NULL
             THEN price_kind IS NOT NULL AND price_author IS NULL
             ELSE (price_kind IS NULL) = (price_author IS NULL)
        END
    ),
    CONSTRAINT sync_request_rule_choice_price_positive
        CHECK (price_minor_units IS NULL OR price_minor_units > 0),
    CONSTRAINT sync_request_rule_choice_currency_known
        CHECK (price_currency IS NULL OR price_currency IN ('gbp', 'usd')),
    CONSTRAINT sync_request_rule_choice_licence_authored
        CHECK ((licence_native_id IS NULL) = (licence_author IS NULL)),
    CONSTRAINT sync_request_rule_choice_resource_type_authored
        CHECK ((resource_type_native_id IS NULL) = (resource_type_author IS NULL)),
    CONSTRAINT sync_request_rule_choice_matches_array CHECK (jsonb_typeof(matches) = 'array')
);

-- Immutability, enforced rather than assumed.
--
-- An official quote, an accepted approval, a queued item's output and a
-- request's snapshot are records of something that happened. Rewriting one
-- rewrites history: an edited application would change what a queued item was
-- approved to publish, and an edited output would change what a lease that
-- already ran believed it sent. DELETE is not covered, so a cascade from the
-- item or request that owns a frozen copy still works and pruning is not
-- blocked by a receipt.
CREATE FUNCTION seller_rule_immutable() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '% rows are immutable once written', TG_TABLE_NAME
        USING HINT = 'append a new row instead of updating this one';
END $$;

CREATE TRIGGER seller_rule_reference_immutable
    BEFORE UPDATE ON seller_rule_reference
    FOR EACH ROW EXECUTE FUNCTION seller_rule_immutable();

CREATE TRIGGER seller_rule_application_immutable
    BEFORE UPDATE ON seller_rule_application
    FOR EACH ROW EXECUTE FUNCTION seller_rule_immutable();

CREATE TRIGGER job_item_rule_output_immutable
    BEFORE UPDATE ON job_item_rule_output
    FOR EACH ROW EXECUTE FUNCTION seller_rule_immutable();

CREATE TRIGGER sync_request_rule_snapshot_immutable
    BEFORE UPDATE ON sync_request_rule_snapshot
    FOR EACH ROW EXECUTE FUNCTION seller_rule_immutable();

CREATE TRIGGER sync_request_rule_choice_immutable
    BEFORE UPDATE ON sync_request_rule_choice
    FOR EACH ROW EXECUTE FUNCTION seller_rule_immutable();

-- A preview is not in that list on purpose: its rows carry the seller's
-- decision, and recording one is an UPDATE. What must not change is the
-- captured evidence, which is guarded the other way -- the repository only
-- ever writes the decision columns, and an accepted row's evidence is copied
-- into an application before the decision is recorded.

-- The tenant fence every table referencing organisation carries, in 0009's
-- null-safe form.
ALTER TABLE seller_rule ENABLE ROW LEVEL SECURITY;
ALTER TABLE seller_rule FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_rule_org_isolation ON seller_rule
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE seller_rule_reference ENABLE ROW LEVEL SECURITY;
ALTER TABLE seller_rule_reference FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_rule_reference_org_isolation ON seller_rule_reference
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE seller_rule_preview ENABLE ROW LEVEL SECURITY;
ALTER TABLE seller_rule_preview FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_rule_preview_org_isolation ON seller_rule_preview
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE seller_rule_preview_row ENABLE ROW LEVEL SECURITY;
ALTER TABLE seller_rule_preview_row FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_rule_preview_row_org_isolation ON seller_rule_preview_row
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE seller_rule_application ENABLE ROW LEVEL SECURITY;
ALTER TABLE seller_rule_application FORCE ROW LEVEL SECURITY;
CREATE POLICY seller_rule_application_org_isolation ON seller_rule_application
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE job_item_rule_output ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_item_rule_output FORCE ROW LEVEL SECURITY;
CREATE POLICY job_item_rule_output_org_isolation ON job_item_rule_output
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE sync_request_rule_snapshot ENABLE ROW LEVEL SECURITY;
ALTER TABLE sync_request_rule_snapshot FORCE ROW LEVEL SECURITY;
CREATE POLICY sync_request_rule_snapshot_org_isolation ON sync_request_rule_snapshot
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE sync_request_rule_choice ENABLE ROW LEVEL SECURITY;
ALTER TABLE sync_request_rule_choice FORCE ROW LEVEL SECURITY;
CREATE POLICY sync_request_rule_choice_org_isolation ON sync_request_rule_choice
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- What the worker's role may reach, and nothing beyond it.
--
-- The engine writes an item's frozen output where it is the thing minting the
-- item -- a scheduled auto-publication, a drained request -- and reads it
-- back on every lease, twice per lease where `live_lease_files` prepares the
-- item again. It reads the request snapshot for the same reason: the
-- evaluation that follows canonicalisation happens on the worker's side of
-- the fence.
--
-- Rules and applications are SELECT only: the worker evaluates the seller's
-- policy and never authors it. Previews, decisions and quotes are not granted
-- at all -- they belong to the console path under `tam_app`.
GRANT SELECT, INSERT ON job_item_rule_output TO tam_engine;
GRANT SELECT, INSERT ON sync_request_rule_snapshot, sync_request_rule_choice TO tam_engine;
GRANT SELECT ON seller_rule, seller_rule_application TO tam_engine;

-- Existing queued work keeps no frozen output, and that is the cutover.
--
-- Nothing is backfilled here. A `job_item` minted before this migration has
-- no `job_item_rule_output` row, and its absence is the explicit statement
-- that no seller ever approved a target policy for it: the reader falls back
-- to the behaviour that item was queued under. The alternative -- copying the
-- product's current canonical price, or the old `mapping.price_rule`
-- snapshot, into a table whose whole meaning is "what the seller approved" --
-- would label a source fact as a seller decision, which is the one thing the
-- design forbids. Items minted from here on always carry a row, written in
-- the same transaction as the item.
