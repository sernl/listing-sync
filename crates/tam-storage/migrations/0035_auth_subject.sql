-- The link from a better-auth user to ours.
--
-- The join key is the subject rather than the email, because better-auth lets
-- a user change their email: an email join would rebind an identity to
-- whichever row currently holds the address, silently and at the moment of
-- the change, while the subject is stable for the life of the account.
--
-- Nullable, because every row minted by tam-mint-session predates better-auth
-- and has no subject to carry; a NOT NULL column would have to invent one.
-- Unique under Postgres' default NULLS DISTINCT, so many unlinked rows
-- coexist while a linked subject names exactly one user.
--
-- app_user stays global rather than tenant (migration 0014): a column does not
-- change what the table is, and the row must still be readable before any
-- tenant pin exists.

ALTER TABLE app_user ADD COLUMN auth_subject uuid;

ALTER TABLE app_user ADD CONSTRAINT app_user_auth_subject UNIQUE (auth_subject);
