-- Custody, on three axes the connection row did not carry: which platform
-- account a link speaks for, how fresh its session is, and who the seller
-- attested authorship to.
--
-- Exclusivity. A marketplace account belongs to one organisation at a time,
-- and nothing until now said so: two tenants could both link the same
-- storefront, and the second would drive the first seller's listings. The
-- account is identified by `platform_account_digest` rather than by the
-- account reference itself, so a compromised database yields no storefront
-- identifiers; the digest is HMAC-SHA-256 over (marketplace || account_ref)
-- under a pepper the broker derives from its own key material, so recovering
-- an identifier needs the key-encryption key the broker alone holds.
--
-- `connection_platform_account_exclusive` is GLOBAL rather than tenant-scoped,
-- and is the one deliberate exception to the tenant-scoped uniqueness every
-- other constraint in this schema keeps. Its purpose IS the crossing: a
-- constraint that admitted the same account once per tenant would enforce
-- nothing. The predicate is what makes the lock releasable — unlink moves the
-- row to 'revoked' or 'unlinked', which leaves the partial index, and the
-- account is immediately claimable by anyone. A NULL digest is likewise
-- outside the index, which is what makes the two-phase link safe: `Link`
-- seals the credential and sets 'linking' with no digest, `Claim` writes the
-- digest and sets 'linked', and a crash between the two leaves a row holding
-- no lock at all. `LeaseRepo::acquire` requires 'linked', so a half-linked
-- connection is inert rather than half-usable.
--
-- Rotating the key-encryption key rotates the derived pepper and therefore
-- invalidates every stored digest: the rows keep their locks but no live
-- identity read will ever match them again. Re-claiming each connection is
-- part of a KEK rotation, not a separate incident.
--
-- Freshness. A session ages independently of whether it is authorised, so
-- these three columns are a second axis rather than more `state` values:
-- `session_verified_at` is when a read last proved the session live,
-- `session_refresh_after` is when the refresh loop should next touch it, and
-- `refresh_failures` counts consecutive refresh failures so a transient is
-- distinguishable from a dead session. Only an auth-class failure flips
-- `state` to 'needs_reauth'; a refresh that merely failed advances the
-- counter and leaves the state alone.
--
-- Authorship. `docs/design/decisions.md` records that the TPT authorship
-- attestation is configuration for an interim reason and that "the durable
-- answer is two columns on connection written by the broker's link step".
-- These are those two columns. The attestation is the seller's declaration of
-- who authored the listing, not a credential, so it sits in the row rather
-- than inside the sealed envelope, where the API path could never read it.
-- `authorship_attested_at` is the instant the seller attested, supplied with
-- the name and never minted from a server clock: a server-minted instant
-- would be this system attesting on the seller's behalf.

ALTER TABLE connection ADD COLUMN platform_account_digest  bytea;
ALTER TABLE connection ADD COLUMN platform_account_seen_at timestamptz;
ALTER TABLE connection ADD COLUMN session_verified_at      timestamptz;
ALTER TABLE connection ADD COLUMN session_refresh_after    timestamptz;
ALTER TABLE connection ADD COLUMN refresh_failures         int NOT NULL DEFAULT 0;
ALTER TABLE connection ADD COLUMN authorship_name          text;
ALTER TABLE connection ADD COLUMN authorship_attested_at   timestamptz;

CREATE UNIQUE INDEX connection_platform_account_exclusive
    ON connection (marketplace, platform_account_digest)
    WHERE platform_account_digest IS NOT NULL
      AND state IN ('linking', 'linked', 'needs_reauth');
