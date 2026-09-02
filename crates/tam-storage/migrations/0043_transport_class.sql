-- The automation rule's branch, as a database fact rather than a Rust match a
-- SQL statement can route around.
--
-- `Marketplace::transport_class()` is the source of truth and a pg-gated test
-- asserts this column equals it for every marketplace, so a no-API marketplace
-- gaining a server transport fails the build. That test is what makes D1's
-- non-negotiable enforceable rather than declarative: without a column, the
-- claim statement has nothing to filter on and the two branches draw one queue.
ALTER TABLE marketplace_inventory ADD COLUMN transport_class text;

UPDATE marketplace_inventory SET transport_class = 'seller_device'
    WHERE marketplace IN ('tes', 'tpt');
UPDATE marketplace_inventory SET transport_class = 'official_api'
    WHERE marketplace = 'etsy';

ALTER TABLE marketplace_inventory ALTER COLUMN transport_class SET NOT NULL;

ALTER TABLE marketplace_inventory ADD CONSTRAINT marketplace_inventory_transport_class_closed
    CHECK (transport_class IN ('seller_device', 'official_api'));

-- The branch is a property of the marketplace rather than of the inventory,
-- so tes_gb and tes_us cannot disagree about it. That is not a uniqueness
-- constraint and no index expresses it; what enforces it is the pg-gated test
-- asserting this column equals `Marketplace::transport_class()`, which is a
-- total function of the marketplace alone and therefore cannot differ between
-- two inventories of one.
