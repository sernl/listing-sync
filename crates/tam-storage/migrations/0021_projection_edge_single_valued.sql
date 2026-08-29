-- The seeder's refresh becomes a genuine upsert, which needs a key that does
-- not carry the label. projection_edge's primary key includes to_segments, so
-- a re-polled label produces a *second* edge from the same term into the same
-- vocabulary rather than replacing the first, and two such edges are exactly
-- the input the projection reads as Ambiguous. A docs/ JSON edit renaming a
-- target would otherwise degrade every listing carrying it from resolved to
-- blocked.

-- Narrower edges are excluded deliberately, and the exclusion is the whole
-- reason this is an index rather than a narrowed primary key. The projection
-- ignores narrower edges entirely (crates/tam-taxonomy/src/project.rs), so
-- they cannot manufacture ambiguity; and the grade relation needs several of
-- them from one term into one vocabulary, because one Tes GB age band covers
-- several US year groups and enumerating them is what makes that election
-- answerable rather than a gap.

-- The pre-check runs first so pre-existing violations name themselves, rather
-- than surfacing as an opaque index build failure part-way through.
DO $$
DECLARE
    offending bigint;
BEGIN
    SELECT count(*) INTO offending FROM (
        SELECT 1 FROM projection_edge
        WHERE kind <> 'narrower'
        GROUP BY from_term, to_inventory, to_term_kind, kind
        HAVING count(*) > 1
    ) AS duplicates;
    IF offending > 0 THEN
        RAISE EXCEPTION
            'projection_edge holds % (term, vocabulary, kind) groups with more than one edge; reconcile them before the relation becomes single-valued',
            offending;
    END IF;
END
$$;

CREATE UNIQUE INDEX projection_edge_single_valued
    ON projection_edge (from_term, to_inventory, to_term_kind, kind)
    WHERE kind <> 'narrower';
