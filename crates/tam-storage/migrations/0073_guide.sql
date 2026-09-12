-- The help guides the console serves, written by an operator and read by
-- every seller.
--
-- Global, carrying no org_id, no row-level security and no policy, for the
-- reason platform_operator carries none (migration 0037): a guide is a
-- platform fact rather than a tenant's data, and every seller reads the same
-- one. The closed-world test in tests/rls_matrix.rs is where that decision is
-- recorded; a table absent from both of its lists fails it.
--
-- tam_backoffice is granted nothing here, and that is deliberate rather than
-- an omission. The backoffice role exists to read across the tenant fence,
-- and a guide sits on neither side of it: the operator routes that write and
-- read guides run on the application pool, which owns this table by owning
-- the database, so the write path needs no GRANT and the read path needs no
-- second connection.
--
-- `body` is Markdown as the operator typed it, and it is stored as typed.
-- Rendering happens on the way out, in crates/tam-api/src/guides.rs, where
-- raw HTML in the body is escaped into text before the HTML is pushed --
-- so a guide can never carry markup the console did not write, and the
-- console can render the answered HTML directly. Storing the rendering
-- instead would freeze a guide under whatever the renderer did on the day it
-- was saved.
--
-- The size bound is on octets rather than characters: what a body costs to
-- store and to send is its bytes, and a character bound would let one guide
-- of astral-plane text cost four times what the number says.
CREATE TABLE guide (
    id         uuid        NOT NULL,
    slug       text        NOT NULL,
    title      text        NOT NULL,
    body       text        NOT NULL,
    status     text        NOT NULL,

    -- Who last wrote it, and nullable because the reference outlives nobody:
    -- an operator's app_user row is never deleted, but a guide seeded by a
    -- migration or by a one-shot has no author to name and saying so is
    -- better than naming the wrong one.
    updated_by uuid        REFERENCES app_user (id),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,

    PRIMARY KEY (id),

    -- The slug is the guide's address, so it is unique across the table
    -- rather than per anything: /guides/{slug} names exactly one document.
    CONSTRAINT guide_slug UNIQUE (slug),

    -- The same lowercase-kebab shape organisation slugs carry (migration
    -- 0057), and longer, because a guide's slug is a title in a URL rather
    -- than a handle somebody types.
    CONSTRAINT guide_slug_shape CHECK (
        slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND length(slug) BETWEEN 1 AND 80
    ),

    CONSTRAINT guide_title_present CHECK (length(title) BETWEEN 1 AND 120),

    CONSTRAINT guide_body_bounded CHECK (octet_length(body) <= 204800),

    CONSTRAINT guide_status CHECK (status IN ('draft', 'published'))
);

-- Published guides, newest first: the reader's whole listing, and the one
-- index it needs. Partial, because a draft is never listed by that route and
-- the operator's listing reads every row in one unfiltered pass anyway.
CREATE INDEX guide_published ON guide (updated_at DESC) WHERE status = 'published';

-- A guide's pictures live under a reserved platform organisation whose slug
-- is `guides`, which crates/tam-api/src/org.rs has always refused to let a
-- tenant claim. The row itself is not created here.
--
-- It is created by the first image upload instead (`ensure_platform_org` in
-- crates/tam-storage/src/guide.rs), and the reason is that this table is not
-- the only closed world in the repository. `organisation` is the tenant
-- roster: `SELECT count(*) FROM organisation` is how several suites assert
-- that a refused signup provisioned nothing and that a returning subject made
-- no second tenant, and the operator's own listing renders every row in it as
-- a tenant. Seeding a row here would make all of those read one higher than
-- the number of tenants that exist, on every database, including the ones
-- that never serve a guide image. Creating it on the first upload keeps
-- "a row in organisation is a tenant somebody provisioned" true until the
-- moment the platform genuinely needs one, and the insert is idempotent, so
-- two uploads racing produce one organisation.
