-- Templates that describe a whole resource, and collections of resources.
--
-- Two halves of the same phase, and they share a migration because they share
-- a seller's sentence: "these resources, finished this way". The template is
-- what "this way" means and the collection is what "these" means, and the one
-- route that joins them -- applying a template to a collection -- would
-- otherwise straddle two migrations for no reason.
--
-- What changes about a template. Migration 0056 stored a name and a partial
-- draft, and the console filled five controls of the create form into it. The
-- draft column is unchanged and still holds whatever the route validated as a
-- `DraftInput`: widening the form to every control is a console change and a
-- column per field was already refused there. What the column set gains is the
-- two facts the draft cannot hold, because neither is a field of the create
-- form:
--
-- `description` is the seller's own note about when to reach for this
-- template. It is not the resource description -- that is `draft.description`
-- and travels into the listing -- and the two are deliberately different
-- columns rather than one, because a note reading "for the phonics packs, not
-- the assessments" must never reach a marketplace.
--
-- `scope` is the marketplace a template is written for, or null for one that
-- is not written for any. A scoped template still fills catalogue fields; what
-- the scope decides is which marketplace's panel the console shows while the
-- seller writes it, and whether an auto-publish rule may name it -- a rule
-- publishing to TPT cannot apply a Tes-scoped template, which
-- `PUT /{version}/sync/settings/{inventory}` refuses. Stored as the inventory
-- token every other column in this schema spells it with, so `inventory_to_db`
-- is the one encoding.
ALTER TABLE resource_template
    ADD COLUMN description text,
    ADD COLUMN scope       text;

-- Bounded at a thousand characters: it is a note in a picker row, not a
-- second listing body. Null and empty are not both admissible, because two
-- spellings of "the seller wrote no note" would make every reader check for
-- each; the route trims and sends null.
ALTER TABLE resource_template
    ADD CONSTRAINT resource_template_description_bounded CHECK (
        description IS NULL OR char_length(description) BETWEEN 1 AND 1000
    ),
    ADD CONSTRAINT resource_template_scope_known CHECK (
        scope IS NULL OR scope IN ('tes', 'etsy', 'tpt')
    );

-- Which template an auto-publish rule fills a pulled resource from.
--
-- On the rule rather than on the setting, and nullable, because the rule is
-- the row that names a target: a seller publishing to two marketplaces from
-- one shop may want each filled from its own scoped template. The API writes
-- one template across a source's whole rule set today and the column admits
-- the finer grain without a migration when a control for it exists.
--
-- `SET NULL (template_id)` rather than bare `SET NULL`: the reference is
-- composite and `org_id` is NOT NULL, so an unqualified set-null would try to
-- blank the tenant column and turn deleting a template into a fault. Naming
-- the column is Postgres 15's feature and this cluster is 17.
--
-- Not cascade: a seller who deletes a template did not ask to stop publishing
-- what their shop brings in. The rule survives without a template and fills
-- nothing, which is exactly what it did before this column existed.
ALTER TABLE auto_publish_rule
    ADD COLUMN template_id uuid,
    ADD CONSTRAINT auto_publish_rule_template_fkey
        FOREIGN KEY (org_id, template_id) REFERENCES resource_template (org_id, id)
        ON DELETE SET NULL (template_id);

-- A named, ordered set of resources the seller acts on together.
--
-- The shape is `label`'s from migration 0046, which is the right ancestor: a
-- collection is a second dimension a seller slices their catalogue by, it is
-- named rather than identified by the client's own vocabulary, and it is
-- private drafting state no engine reads. Three differences, each earned:
--
-- The client does learn a collection's identifier, unlike a label's. A label
-- is its text and a rename is a rename; a collection carries a description and
-- an order, so it is a thing with fields rather than a word, and addressing it
-- by name would make renaming it a migration of every URL the seller has open.
--
-- A description, bounded as the template's is and for the same reason.
--
-- `updated_at`, which `label` has no use for: membership and order change
-- under a stable name, and a list sorted by name with no sense of recency
-- cannot show the seller which collection they were last working in.
CREATE TABLE collection (
    org_id      uuid        NOT NULL REFERENCES organisation (id),
    id          uuid        NOT NULL,
    name        text        NOT NULL,
    description text,
    created_at  timestamptz NOT NULL,
    updated_at  timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT collection_name_bounded CHECK (
        char_length(name) BETWEEN 1 AND 80
    ),
    CONSTRAINT collection_description_bounded CHECK (
        description IS NULL OR char_length(description) BETWEEN 1 AND 1000
    ),
    CONSTRAINT collection_updated_after_created CHECK (
        updated_at >= created_at
    )
);

-- One collection per name per organisation, folded, for `label_one_per_name`'s
-- reason in 0046: a seller who types "autumn unit" after "Autumn Unit" means
-- the one they already have. Folded rather than exact also makes this index the
-- listing's order, so the table needs no second one.
CREATE UNIQUE INDEX collection_one_per_name
    ON collection (org_id, lower(name));

-- Which resources a collection holds, and in what order.
--
-- The key is the pair, so a resource is in a collection once: the order is a
-- field of the membership rather than its identity, and a unique index on
-- (collection, position) would refuse a reorder that the whole-set replace
-- performs as a delete and an insert anyway.
--
-- No cascade from `product`, which is `product_label`'s choice and is load
-- bearing here: a product is soft-deleted -- `product.deleted_at` -- so a
-- cascade would never fire, and a membership row surviving a soft delete is
-- correct. What reads it filters on `deleted_at IS NULL`, so a resource in the
-- bin is out of every collection without any row being moved, and restoring it
-- puts it back where the seller had it.
CREATE TABLE collection_member (
    org_id        uuid        NOT NULL REFERENCES organisation (id),
    collection_id uuid        NOT NULL,
    product_id    uuid        NOT NULL,
    position      integer     NOT NULL,
    added_at      timestamptz NOT NULL,

    PRIMARY KEY (org_id, collection_id, product_id),

    -- A deleted collection takes its membership with it: a row naming no
    -- collection is reachable from nothing and answers no question.
    FOREIGN KEY (org_id, collection_id) REFERENCES collection (org_id, id) ON DELETE CASCADE,
    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),

    CONSTRAINT collection_member_position_nonnegative CHECK (position >= 0)
);

-- The reverse read: "which collections is this resource in", which the
-- resource page asks once per open. Indexed on the product because the
-- primary key leads with the collection.
CREATE INDEX collection_member_by_product
    ON collection_member (org_id, product_id);

ALTER TABLE collection ENABLE ROW LEVEL SECURITY;
ALTER TABLE collection FORCE ROW LEVEL SECURITY;
CREATE POLICY collection_org_isolation ON collection
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE collection_member ENABLE ROW LEVEL SECURITY;
ALTER TABLE collection_member FORCE ROW LEVEL SECURITY;
CREATE POLICY collection_member_org_isolation ON collection_member
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No backoffice grant on either, for the reason migration 0056 states about
-- templates: this is a seller's own filing of work they have not published,
-- no operator route reads it, and cross-tenant reach granted against a need
-- nobody has stated is privilege with no reader.
