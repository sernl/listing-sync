-- Starter joins the plan ladder below Sync
-- (docs/notes/design/research/2026-09-27-subscription-tiers.md). `Plan` in
-- tam-limits is the other copy of this list, and the two are meant to be read
-- together. Nothing is rewritten: every stored grant already names a plan in
-- the wider set.
ALTER TABLE entitlement_grant
    DROP CONSTRAINT entitlement_grant_plan_known;

ALTER TABLE entitlement_grant
    ADD CONSTRAINT entitlement_grant_plan_known CHECK (
        plan IN ('free', 'starter', 'subscriber', 'studio')
    );
