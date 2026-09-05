-- The handle the seller chooses for their own organisation.
--
-- `organisation.name` is what the seller wants printed and has been since
-- migration 0001; it is neither unique nor URL-safe, and until a seller sets
-- one it is the provisional `org-{uuid}` that self-serve signup writes. The
-- slug is the other half: the unique, URL-safe handle a seller types and a
-- support email quotes. Both columns pay rent, so this adds one beside the
-- other rather than replacing it.
--
-- Nullable, and no backfill. A slug derived from the row's own UUID is the
-- provisional-name problem wearing a different hat, and it would burn a handle
-- the seller may want. NULL is the honest "not yet chosen", and it is exactly
-- what the console's prompt keys on. Postgres' default NULLS DISTINCT lets
-- every unclaimed row coexist under the unique index while a claimed slug
-- names exactly one organisation -- the pattern migration 0035 already uses
-- for `app_user.auth_subject`, with the reasoning written down there.
--
-- The index is on `lower(slug)` rather than on `slug`. The API lowercases on
-- the way in, so the two are the same for every row this product writes; the
-- functional index is what makes case-insensitive uniqueness true of a writer
-- that is not the API -- a psql session, a backfill script -- rather than only
-- of the route that promises it.
--
-- The CHECK restates the shape rule and not the reserved list. That split is
-- deliberate. Shape is short enough to keep in step across two dialects and
-- stops a malformed slug arriving from a path that is not the API, while the
-- reserved list is product policy that moves with our own route table and
-- belongs in one place: the validator in `crates/tam-api/src/org.rs`, which
-- also holds the refusal of a 32-character hexadecimal string, so the slug
-- namespace can never collide with an unhyphenated UUID.

ALTER TABLE organisation ADD COLUMN slug text;

CREATE UNIQUE INDEX organisation_slug_lower ON organisation (lower(slug));

ALTER TABLE organisation ADD CONSTRAINT organisation_slug_shape
    CHECK (slug IS NULL
           OR (slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND length(slug) BETWEEN 3 AND 32));

-- Whether this organisation is allowed to carry no slug.
--
-- A newly provisioned organisation and one that predates this migration both
-- hold `slug IS NULL`, and the console treats them differently: the first
-- meets the claim screen as a gate before the console, because a new seller
-- has nothing to do there before naming their organisation, and the second
-- meets a banner it can dismiss, because it is mid-work and a gate would be a
-- rude surprise for no gain. Nothing in the row distinguishes them, so this
-- column records the distinction at the one instant it is knowable: now.
--
-- Written once, by the UPDATE below, and never again. The alternative was
-- comparing `created_at` against a cutoff instant, which puts a date literal
-- in whichever layer does the comparing and cannot be checked by a test that
-- does not also control the clock.
--
-- The default is false, so every organisation created after this point is
-- gated. That reaches the development mint path too (`SessionRepo::ensure_org`),
-- and correctly: a minted development tenant has chosen no slug either.

ALTER TABLE organisation ADD COLUMN slug_deferred boolean NOT NULL DEFAULT false;

UPDATE organisation SET slug_deferred = true;
