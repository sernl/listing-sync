-- The rest of the breadcrumb: what the read observed about the source
-- listing, so a redrained migrate can rebuild the removal leg for a resource
-- an earlier pass already canonicalised.
--
-- 0025 recorded the product and the mapping alone, which is enough to skip the
-- work and not enough to repeat its output. `Canonicalisation` also carries
-- the source listing's identifier and the lifecycle the read saw, and the
-- removal item needs both: it names the listing it takes down and states the
-- state it takes it down from. So a resumed pass built the removal leg from
-- the current pass's rows only, and every resource canonicalised earlier lost
-- its removal item -- silently, because the request was then marked enqueued
-- naming a removal job that would never remove them. With every resource
-- already done, the job was minted with no items at all and read back as
-- settled, so the migrate degraded into a plain sync and said nothing.
--
-- The identifier is the three-column rendering `mapping.binding` and
-- `write_attempt` already use, so there is one encoding of a RemoteListingId
-- in the schema rather than a second one here.

ALTER TABLE sync_request_resource
    ADD COLUMN source_kind       text,
    ADD COLUMN source_url        text,
    ADD COLUMN source_numeric_id bigint,
    -- Nullable even on a canonicalised row: a read that carried no lifecycle
    -- is a source the removal must refuse rather than take down from a state
    -- nobody observed, and that refusal is the drain's to make.
    ADD COLUMN source_state      text;

ALTER TABLE sync_request_resource
    ADD CONSTRAINT sync_request_resource_source_shape CHECK (
        (source_kind = 'tes'  AND source_url IS NOT NULL AND source_numeric_id IS NULL)
     OR (source_kind = 'tpt'  AND source_url IS NULL     AND source_numeric_id IS NOT NULL)
     OR (source_kind = 'etsy' AND source_url IS NULL     AND source_numeric_id IS NOT NULL)
     OR (source_kind IS NULL  AND source_url IS NULL     AND source_numeric_id IS NULL)
    ),
    ADD CONSTRAINT sync_request_resource_source_state CHECK (
        source_state IS NULL OR source_state IN ('draft', 'live')
    ),
    -- A state without an identifier states a lifecycle for nothing.
    ADD CONSTRAINT sync_request_resource_state_needs_source CHECK (
        source_state IS NULL OR source_kind IS NOT NULL
    );

-- The breadcrumb is the whole row or it is not a breadcrumb: 0025 said so of
-- the product and the mapping, and the source identifier joins them. Replaced
-- rather than added beside, so there is one statement of what a canonicalised
-- row carries.
ALTER TABLE sync_request_resource
    DROP CONSTRAINT sync_request_resource_canonicalised_total,
    ADD CONSTRAINT sync_request_resource_canonicalised_total CHECK (
        (state = 'canonicalised')
            = (product_id IS NOT NULL AND mapping_id IS NOT NULL
               AND source_kind IS NOT NULL)
    );
