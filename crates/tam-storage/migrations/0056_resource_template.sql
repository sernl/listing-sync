-- Named starting points for a new resource: the Template Manager's second tab.
--
-- A template is a partial draft of the create form, saved under a name the
-- seller chose, so that the next resource of the same kind starts filled in
-- rather than blank. It is the shape `/{version}/authoring/check` already
-- validates -- a `DraftInput` with every field optional -- held for later
-- instead of submitted now.
--
-- The draft is jsonb rather than a column per field, and the reason is that
-- the create form's field set is TPT's and moves when TPT's does: migrations
-- 0040, 0049 and 0050 are three amendments to the sidecar that mirrors it
-- already. A template is not read by the engine, never lowered to a
-- marketplace, and never joined against -- the console reads one whole and
-- prefills a form with it -- so a column per field would buy nothing and cost
-- a migration every time the form gained a control. What keeps the document
-- honest is the API bound: the route deserialises it as `DraftInput` and runs
-- the create form's own value rules over it before this column sees it, so a
-- template cannot hold a value the form would later refuse.
--
-- The two CHECKs below are that bound made true of the column for any writer,
-- not only for the route. `jsonb_typeof` because a bare array or scalar is not
-- a draft and would deserialise into nothing the console could read; the byte
-- ceiling because an unbounded jsonb column reachable by any tenant is an
-- upload channel, and a create form's worth of fields -- a description, a few
-- dozen slugs, four thumbnail digests -- is orders of magnitude under it.
--
-- The byte ceiling is deliberately looser than the route's own 65536, and the
-- two measure different strings. The route measures serde_json's compact
-- rendering; this one measures Postgres re-rendering the parsed value, which
-- inserts a space after every `:` and every `,`. A CHECK set to the route's own
-- number would therefore fire on a draft the route accepted, turning an answer
-- into a fault -- the exact failure the zero-byte and control-character rules
-- upstream exist to prevent. The densest separator packing a JSON value admits
-- is an array of one-character elements, one comma per two bytes, so the string
-- measured here is at most one and a half times the one measured there: 98304
-- against the 131072 below. That headroom is a backstop for a writer that is
-- not the route, not a second opinion about what a template may hold.
--
-- The multiple holds only because the route refuses a fractional number.
-- `jsonb` stores a number as `numeric` and renders it in full, while
-- serde_json renders an f64 through its shortest round-trip form, so `1e+308`
-- is six bytes there and three hundred and nine here; a draft padded with nine
-- thousand of them measures 65529 compact and 2910969 as `draft::text`. Whole
-- numbers render identically on both sides, and the create form has no
-- fractional field, so refusing the rest at the boundary is what leaves this
-- ceiling meaning what it says.
CREATE TABLE resource_template (
    org_id     uuid        NOT NULL REFERENCES organisation (id),
    id         uuid        NOT NULL,
    name       text        NOT NULL,
    draft      jsonb       NOT NULL,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT resource_template_name_bounded CHECK (
        char_length(name) BETWEEN 1 AND 80
    ),
    CONSTRAINT resource_template_draft_is_object CHECK (
        jsonb_typeof(draft) = 'object'
    ),
    CONSTRAINT resource_template_draft_bounded CHECK (
        octet_length(draft::text) <= 131072
    ),
    CONSTRAINT resource_template_updated_after_created CHECK (
        updated_at >= created_at
    )
);

-- One template per name per organisation, compared case-insensitively, for the
-- reason `label_one_per_name` gives in migration 0046: a seller who types
-- "autumn unit" after "Autumn Unit" means the template they already have, and
-- two rows differing only in case would make the picker ask a question with no
-- answer. Folded rather than exact also makes this index the listing's own
-- order, which is by name, so the table needs no second index.
--
-- `lower()` folds according to the cluster's ctype, so a `C` or `POSIX` cluster
-- folds ASCII only and "ÉTÉ" and "été" are two templates rather than one.
-- Inherited from `label_one_per_name` in 0046 rather than decided here, and
-- named because the route's own character bound reasons explicitly about
-- accented and non-Latin names; `db/ephemeral-postgres.sh` passes initdb no
-- locale, so which behaviour holds is the devshell's to state, not this file's.
CREATE UNIQUE INDEX resource_template_one_per_name
    ON resource_template (org_id, lower(name));

ALTER TABLE resource_template ENABLE ROW LEVEL SECURITY;
ALTER TABLE resource_template FORCE ROW LEVEL SECURITY;
CREATE POLICY resource_template_org_isolation ON resource_template
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

-- No backoffice grant and no backoffice policy, deliberately, and unlike every
-- other tenant table migration 0037 reaches. That set is delivery and ledger
-- data -- product, mapping, connection, job, job_item, write_attempt and the
-- two halts -- which an operator answering a support question has to be able
-- to see. A template is listing copy the seller has not published anywhere,
-- no operator route reads one, and cross-tenant reach over every tenant's
-- private drafting with no reader is privilege granted against a need nobody
-- has stated. The day an operator surface needs it is the day to add it, in
-- 0037's two halves, with the route that reads it in the same change.
