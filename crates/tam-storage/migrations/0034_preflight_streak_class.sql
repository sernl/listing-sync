-- Whether every failure in the current preflight streak was the edge's
-- rather than the seller's.
--
-- The streak's verdict decides two seller-visible things: whether the
-- connection is gated to `needs_reauth`, and whether the item's row tells the
-- seller to re-link or tells them the retry is ours. Deciding that from the
-- error that happened to arrive last makes both non-deterministic, because a
-- streak can mix classes -- a Cloudflare block on one lease and a genuine
-- lapsed session on the next -- and which one lands last is arrival order.
--
-- So the streak carries the conjunction instead. It starts true, because the
-- conjunction over no failures is true; any failure that is not edge-class
-- clears it; and a healthy preflight resets it with the count. The verdict
-- takes the seller-actionable floor unless the whole streak was the edge's,
-- since a real auth failure anywhere in the streak is a fact the seller can
-- act on and an edge block is not.

ALTER TABLE job_item
    ADD COLUMN preflight_challenge_only boolean NOT NULL DEFAULT true;
