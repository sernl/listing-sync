-- What a guide is being written as, what sellers are reading, and the words
-- both are filed under.
--
-- Migration 0073 stored one copy of a guide and a `status` beside it, which
-- made saving and publishing the same act: an operator fixing a sentence in a
-- published guide changed the published guide, mid-sentence, for every reader.
-- This migration splits the row in two. The `title`/`body`/`topic_id` columns
-- and the `draft` tag assignments are the working copy, which only an operator
-- ever reads; the `published_*` columns and the `published` tag assignments are
-- the snapshot sellers read, and nothing but a publish writes them.
--
-- `status` goes away rather than staying as a third fact, and that is the
-- point of the split rather than tidying: a column saying "published" beside a
-- snapshot that is absent or stale is a way for a draft to leak, and the only
-- way to make that unrepresentable is for the snapshot's presence to *be* the
-- publication state. Every read that used to filter on `status` filters on
-- `published_at IS NOT NULL` now, and `guide_publication_whole` makes a
-- half-written snapshot impossible to commit.
--
-- `revision` is one counter over the whole aggregate -- content and taxonomy
-- alike -- and every write increments it: save, publish and unpublish. It is
-- what a conditional write compares against, so two operators in the same
-- guide collide loudly instead of overwriting each other, and it is what lets
-- an editor that lost an acknowledgement ask the server what happened rather
-- than guess. `published_revision` is the revision the snapshot was copied
-- from, which is how a reader-visible page is traced back to the exact draft
-- that was acknowledged.

-- The words a guide is filed under: one topic and any number of tags.
--
-- One table with a `kind` discriminator rather than two tables of identical
-- shape. A topic and a tag differ in how many a guide may carry, which is a
-- fact about the assignment rather than about the word, and two tables would
-- duplicate the slug shape, the name bound, the retirement flag and every
-- query that lists them for the sake of one enum value.
--
-- Identified rather than named, unlike `label` (migration 0046): a guide's
-- taxonomy is the platform's own vocabulary, an operator renames a topic when
-- the wording is wrong, and addressing it by text would make a rename a
-- rewrite of every guide filed under it.
--
-- Global, with no RLS and no policy, for `guide`'s reason in 0073: this is one
-- vocabulary every tenant reads, so it sits on neither side of the tenant
-- fence. The closed-world test in tests/rls_matrix.rs is where that is
-- recorded. No backoffice grant either: the operator routes that write and
-- read this run on the application pool, which owns the table by owning the
-- database.
CREATE TABLE guide_taxon (
    id         uuid        NOT NULL,
    kind       text        NOT NULL,
    slug       text        NOT NULL,
    name       text        NOT NULL,

    -- Retired rather than deleted, and there is no delete: a topic withdrawn
    -- from the pickers is still the topic a published guide is filed under,
    -- and deleting it would either take that guide's filing with it or refuse
    -- the operator's request at a foreign key. Retirement is therefore the
    -- whole of "stop using this", and a retired taxon a published guide still
    -- names keeps appearing to readers.
    retired    boolean     NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL,

    PRIMARY KEY (id),

    CONSTRAINT guide_taxon_kind CHECK (kind IN ('topic', 'tag')),

    -- One topic and one tag may share a slug, because they are separate
    -- vocabularies: "assessment" as a topic and "assessment" as a tag are not
    -- a collision an operator needs explaining.
    CONSTRAINT guide_taxon_one_per_slug UNIQUE (kind, slug),

    -- The shape `guide_slug_shape` carries, for the same reason: a taxon slug
    -- is what the reader's filter puts in a URL.
    CONSTRAINT guide_taxon_slug_shape CHECK (
        slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND char_length(slug) BETWEEN 1 AND 80
    ),

    -- `collection_name_bounded`'s eighty from 0072: this is a chip on a card,
    -- not a sentence.
    CONSTRAINT guide_taxon_name_bounded CHECK (char_length(name) BETWEEN 1 AND 80)
);

ALTER TABLE guide
    -- One counter for the aggregate, starting where every existing guide
    -- starts: 1, so a stored guide has a revision the first conditional write
    -- against it can name.
    ADD COLUMN revision           integer NOT NULL DEFAULT 1,

    -- The working topic. Nullable because a guide need not be filed, and the
    -- guides that exist when this migration runs are not filed: inventing a
    -- topic for them here would be this migration deciding editorial policy.
    ADD COLUMN topic_id           uuid REFERENCES guide_taxon (id),

    -- The snapshot. Every field a reader sees has its copy here, and the copy
    -- is what they see: a reader's title, prose, filing and update time all
    -- come from these columns, never from the working ones beside them.
    ADD COLUMN published_title    text,
    ADD COLUMN published_body     text,
    ADD COLUMN published_topic_id uuid REFERENCES guide_taxon (id),
    ADD COLUMN published_revision integer,
    ADD COLUMN published_at       timestamptz;

-- What is published stays published, reading exactly as it did before this
-- migration ran.
--
-- `updated_at` as the publication time, because for a guide written under 0073
-- the last save *was* the publication -- there was no other act -- and it is
-- the timestamp the reader was already being shown. `revision` is 1 for every
-- row here, so the snapshot's source revision is 1: the content came from the
-- draft as it stands, which is true, because under 0073 there was one copy.
UPDATE guide
   SET published_title    = title,
       published_body     = body,
       published_revision = revision,
       published_at       = updated_at
 WHERE status = 'published';

-- The old visibility column and everything that read it. A guide that was a
-- draft under 0073 is unavailable to readers under 0075 for the same reason it
-- was then: nothing has published it.
DROP INDEX guide_published;

ALTER TABLE guide
    DROP CONSTRAINT guide_status,
    DROP COLUMN status;

ALTER TABLE guide
    ADD CONSTRAINT guide_revision_positive CHECK (revision >= 1),

    -- A snapshot is whole or absent. This is the constraint that makes
    -- "published" unambiguous: there is no row where a reader would find a
    -- title and no prose, or a publication time with nothing published at it.
    ADD CONSTRAINT guide_publication_whole CHECK (
        (published_title    IS NULL
         AND published_body IS NULL
         AND published_revision IS NULL
         AND published_at   IS NULL)
        OR
        (published_title    IS NOT NULL
         AND published_body IS NOT NULL
         AND published_revision IS NOT NULL
         AND published_at   IS NOT NULL)
    ),

    -- The working copy's bounds, applied to the snapshot: what is published
    -- was a draft, so it cannot be longer than one.
    ADD CONSTRAINT guide_published_title_present CHECK (
        published_title IS NULL OR char_length(published_title) BETWEEN 1 AND 120
    ),
    ADD CONSTRAINT guide_published_body_bounded CHECK (
        published_body IS NULL OR octet_length(published_body) <= 204800
    ),

    -- A filing with nothing published under it is a snapshot field written
    -- outside a publish, which is the mistake this whole migration exists to
    -- make impossible.
    ADD CONSTRAINT guide_published_topic_only_published CHECK (
        published_topic_id IS NULL OR published_title IS NOT NULL
    ),

    -- A snapshot comes from a revision that happened: the copy cannot claim a
    -- draft the guide has not reached.
    ADD CONSTRAINT guide_published_revision_reached CHECK (
        published_revision IS NULL OR published_revision <= revision
    );

-- Published guides, newest publication first: the reader's whole listing,
-- keeping 0073's name and partiality now that the predicate is the snapshot's
-- presence rather than a status word.
CREATE INDEX guide_published ON guide (published_at DESC) WHERE published_at IS NOT NULL;

-- Which tags a guide carries, in the working copy and in the snapshot.
--
-- One table with a `scope` discriminator, so publishing is a copy of rows from
-- one scope to the other inside the same transaction that copies the columns:
-- the snapshot's taxonomy is replaced atomically with its prose, and there is
-- no moment at which a reader sees new tags on old text.
--
-- The key is (guide, scope, taxon), so a guide carries a tag at most once per
-- scope and the order of a tag set is not a fact anybody stores -- tags render
-- by name, which is an order the reader can predict.
--
-- `ON DELETE CASCADE` from `guide` because an assignment naming no guide
-- answers no question. No cascade from `guide_taxon`, deliberately: there is
-- no delete to cascade from, and the foreign key is what makes retirement the
-- only way to withdraw a word.
--
-- The kind is not constrained here, because a CHECK cannot read another table.
-- What keeps a topic out of the tag column is `crates/tam-storage/src/guide.rs`,
-- which resolves every identifier a write names against the kind the column
-- expects and refuses the write before it starts.
CREATE TABLE guide_tag_assignment (
    guide_id uuid NOT NULL REFERENCES guide (id) ON DELETE CASCADE,
    taxon_id uuid NOT NULL REFERENCES guide_taxon (id),
    scope    text NOT NULL,

    PRIMARY KEY (guide_id, scope, taxon_id),

    CONSTRAINT guide_tag_assignment_scope CHECK (scope IN ('draft', 'published'))
);

-- The reverse read the reader's filter performs: "which published guides carry
-- this tag", and the read that lists the taxonomy published content actually
-- references. The primary key leads with the guide, so neither is served by it.
CREATE INDEX guide_tag_assignment_by_taxon ON guide_tag_assignment (taxon_id, scope);
