-- What the import measured about one resource's taxonomy coverage.
--
-- The founder's kill gate is a coverage number compared across a series of
-- migrations, and until now it existed only as a job event on the run that
-- produced it: readable once, in a stream, by whoever was watching. These four
-- columns put it where the request's own view reads it, so the console and the
-- review step can show it per resource for as long as the request exists.
--
-- Per resource rather than totalled on `sync_request`, which is the choice
-- worth recording because the cheaper one is wrong. A request-level total can
-- say a migration lost eleven terms; it cannot say which product lost them,
-- and the seller's next action is always about a product. The request-level
-- figure is a SUM over these, computed on read, which is exact rather than an
-- approximation of the totals it replaces.
--
-- The definition is `tam_import::uncovered_terms` and nothing else: Subject and
-- Topic only, distinct terms, a recorded no-counterpart omitted because that is
-- a decision somebody took rather than a gap, and a term the catalogue cannot
-- classify counted. A column filled by any other reckoning is worse than an
-- absent one, because the gate compares it against earlier runs.
--
-- Nullable with no default, which is the whole point rather than a detail. A
-- zero would say the import measured this resource and found nothing uncovered;
-- null says nothing measured it at all. Those are different facts and the gate
-- is exactly where confusing them is expensive: a server-branch migrate's
-- breadcrumbs, and every resource a device skipped, have never been through the
-- taxonomy, and a default of zero would enter them into the founder's average
-- as perfect coverage.
ALTER TABLE sync_request_resource
    ADD COLUMN terms_seen      integer,
    ADD COLUMN terms_mapped    integer,
    ADD COLUMN terms_unmapped  integer,
    ADD COLUMN terms_uncovered integer;

-- A count is not negative. NULL satisfies a CHECK, so this constrains the
-- measured rows and leaves the unmeasured ones to the constraint below.
--
-- Non-negativity only, deliberately. The obvious companions — that a term
-- cannot map without having been seen, that the uncovered are a subset of the
-- mapped — are facts about today's resolver rather than definitions: the
-- founder's mapping direction admits one native term resolving to several
-- canonical ones, and the day that lands, a frozen constraint would refuse a
-- correct import inside the database. The golden row report and the coverage
-- tests catch a mis-wired field, and they can be changed.
ALTER TABLE sync_request_resource ADD CONSTRAINT sync_request_resource_counts CHECK (
    terms_seen >= 0 AND terms_mapped >= 0 AND terms_unmapped >= 0 AND terms_uncovered >= 0
);

-- The four are one measurement, so they are present together or absent
-- together. Three counts and a null is not a partial measurement; it is a
-- writer that got one field wrong, and this refuses it at the write rather than
-- leaving a reader to decide what a missing quarter of a measurement means.
ALTER TABLE sync_request_resource ADD CONSTRAINT sync_request_resource_coverage_total CHECK (
    (terms_seen IS NULL AND terms_mapped IS NULL
        AND terms_unmapped IS NULL AND terms_uncovered IS NULL)
 OR (terms_seen IS NOT NULL AND terms_mapped IS NOT NULL
        AND terms_unmapped IS NOT NULL AND terms_uncovered IS NOT NULL)
);
