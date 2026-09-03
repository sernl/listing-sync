-- The localisation flag on the TPT-base sidecar.
--
-- data[ItemsLocalization][country_id_flag] is the last control TPT renders in
-- its Categories group: a checkbox whose label names the seller's own country,
-- read as "Appropriate for New Zealand" in the capture the product model was
-- decoded from. The country id beside it is posted by neither the create nor
-- the edit, and no country list appears in the DOM or in the page bootstrap,
-- so the boolean is the whole field. Which country the label names is
-- presentation a caller supplies and is deliberately not stored here.
--
-- NOT NULL DEFAULT false, because false is what an unticked checkbox posts and
-- is also what every row written before this column carried: the TPT write
-- model sends the flag off as a constant on both the create and the edit. The
-- default therefore states what those writes already meant rather than
-- guessing at them.
--
-- No CHECK, and no cap. A boolean column has exactly the two members the
-- control has, and TPT marks the control optional, so there is no refusal here
-- for the domain to mirror.
--
-- The tenant fence is inherited rather than restated: product_tpt_base already
-- carries forced row-level security and the product_tpt_base_org_isolation
-- policy from 0040, and adding a column to a table leaves both in force.

ALTER TABLE product_tpt_base
    ADD COLUMN appropriate_for_country boolean NOT NULL DEFAULT false;
