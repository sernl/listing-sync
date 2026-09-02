-- The education-standards catalogue: the four frameworks TPT's create form
-- offers, ingested from the Common Standards Project mirror by
-- tam-standards-fetch and normalised by tam-standards.
--
-- Global reference data, like canonical_term and projection_edge: the
-- catalogue is a published corpus, not a tenant's, so there is no org_id and
-- no row-level security here. What a seller *selects* from it is tenant data
-- and belongs to a later migration beside the mapping, not to this one.
--
-- The primary key is the mirror's own identifier, because no key built from
-- the published code holds. The measurements are from the ingest of
-- 2026-09-03 and each one falsified a candidate:
--
--   (framework, code)          697 of 4,872 Texas codes and 126 of 5,162
--                              Virginia codes name a different standard under
--                              a different subject. `1.1.A` is one thing in
--                              Texas mathematics and another in Texas science,
--                              because the mirror drops the Administrative
--                              Code chapter that told them apart.
--   (framework, subject, code) still collides on 330 keys in Common Core, 414
--                              in Texas and 322 in Virginia, because the mirror
--                              serves overlapping grade and course sets and
--                              repeats the anchor standards in each. 331 of the
--                              Texas collisions carry *different* statements.
--   (framework, set_id, code)  collides on 27 keys in Texas and two in
--                              Virginia.
--
-- So a published code is a label a seller searches by, not an identity, and
-- anything that binds a standard -- a seller's tag, a TPT node id -- binds the
-- source identifier. A picker that shows a bare code without its subject and
-- grade is showing an ambiguous one.

CREATE TABLE standards_node (
    framework    text        NOT NULL,
    source_guid  text        NOT NULL,

    -- The mirrored set, and the provenance that travels with it.
    set_id       text        NOT NULL,
    subject      text        NOT NULL,
    source       text        NOT NULL,
    source_url   text        NOT NULL,
    fetched_at   timestamptz NOT NULL,

    -- The licence the mirror declared on the set this row came from. Per row
    -- rather than per framework because the Common Standards Project declares
    -- it per set and the holders differ across them, and a CC BY attribution
    -- names the holder.
    licence_title  text,
    licence_url    text,
    licence_holder text,

    -- The derived grade interval, on the one scale the four frameworks can be
    -- compared on: prekindergarten -1, kindergarten 0, then the school years.
    -- Null where the set declared no level that parsed.
    grade_low    smallint,
    grade_high   smallint,

    code         text,
    alt_code     text,
    -- The published code folded to its letters and digits, upper-cased, which
    -- is what a prefix search compares against: a teacher types `5.3B` where
    -- the mirror wrote `5.3.B`. Stored rather than computed in the index so
    -- that one fold serves both the in-memory picker and this table, and
    -- `tam_standards::fold_code` stays the only definition of it.
    code_fold    text,
    node_type    text,

    -- Verbatim and never paraphrased. The Common Core grant names copy,
    -- publish, distribute and display; modification is not among them, so
    -- nothing may rewrite this column, including generated listing copy.
    statement    text        NOT NULL,

    hierarchy_path text[]    NOT NULL DEFAULT '{}',
    uri          text,
    parent_guid  text,
    depth        integer     NOT NULL,

    -- Whether a seller tags at this node. Containers are kept because a
    -- standard's meaning depends on its parent's text, which is why this is a
    -- column rather than a reason to omit the row.
    addressable  boolean     NOT NULL,

    -- Retirement rather than deletion. A code that disappears upstream is the
    -- event that actually matters, because some seller's listing points at it;
    -- the row stays with an end date and the projection surfaces it.
    retired_at   timestamptz,

    PRIMARY KEY (framework, source_guid),

    CONSTRAINT standards_node_framework CHECK (
        framework IN ('CCSS', 'NGSS', 'TEKS', 'VA_SOL')
    ),
    -- An addressable row is one a seller can post, so it must have something
    -- to post and something to show. Both halves are load-bearing: two rows
    -- of the first ingest carried a Texas code and no statement at all, and
    -- the picker rule is that a code is shown with its full statement or not
    -- at all.
    CONSTRAINT standards_node_addressable_postable CHECK (
        NOT addressable
        OR (code IS NOT NULL AND code_fold IS NOT NULL AND btrim(statement) <> '')
    ),
    CONSTRAINT standards_node_grade_ordered CHECK (
        (grade_low IS NULL) = (grade_high IS NULL)
        AND (grade_low IS NULL OR grade_low <= grade_high)
    ),
    CONSTRAINT standards_node_code_fold_present CHECK (
        (code IS NULL) = (code_fold IS NULL)
    )
);

-- Not unique, for the reason above. A lookup by code returns candidates that
-- the caller disambiguates by subject and grade; a unique index here would
-- refuse 2,395 rows of the committed ingest, spread over 1,433 keys.
CREATE INDEX standards_node_subject_code
    ON standards_node (framework, subject, code)
    WHERE code IS NOT NULL;

-- Prefix search on the folded code, which is the first thing a teacher does.
-- text_pattern_ops so `LIKE 'CCSSMATH%'` uses the index under any collation.
CREATE INDEX standards_node_code_prefix
    ON standards_node (framework, code_fold text_pattern_ops)
    WHERE code_fold IS NOT NULL AND retired_at IS NULL;

-- Filter by grade and subject, then browse: the second thing a teacher does,
-- and the only query that reads most of a framework at once.
CREATE INDEX standards_node_grade_band
    ON standards_node (framework, grade_low, grade_high)
    WHERE addressable AND retired_at IS NULL;

CREATE INDEX standards_node_subject
    ON standards_node (framework, subject)
    WHERE retired_at IS NULL;

-- Keyword search over the statement, for the teacher who knows the concept
-- but not the code. The regconfig is named so the expression is immutable and
-- the index is usable; core Postgres, no extension.
CREATE INDEX standards_node_statement_search
    ON standards_node USING gin (to_tsvector('english', statement));

-- Expanding a node's children is how a picker drills, and the parent link is
-- the mirror's rather than the code's, because Virginia's four subjects use
-- four incompatible code grammars.
CREATE INDEX standards_node_parent
    ON standards_node (framework, parent_guid)
    WHERE parent_guid IS NOT NULL;

-- The worker projects a product's standards while executing a job, on the
-- same reasoning as the taxonomy grant in migration 0013: the engine reads
-- the catalogue, and every write to it comes from the operator seeder on the
-- API path where tam_app already holds owner privileges.
GRANT SELECT ON standards_node TO tam_engine;
