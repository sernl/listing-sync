-- Where each user stands with the console's guided tour.
--
-- On `app_user` rather than the organisation, because the tour is shown to a
-- person: a second teacher invited into an organisation has not seen it just
-- because the first one has.
--
-- Four states, not a nullable timestamp, because "has not seen it" means two
-- different things. A user created from here on is `due`: the console offers
-- the tour on their first sign-in. A user who already existed when the tour
-- shipped is `predates`: they know the console already, so the tour is not
-- pushed at them on an ordinary visit, and is offered only on the first visit
-- after they buy a plan. `completed` and `skipped` are the two ways a user
-- ends it; either one stops it being offered again, and `tour_settled_at`
-- says when. "Show me around" in Help and guides restarts it by hand
-- whatever the state.
--
-- The column is added with `predates` as its default so every existing row is
-- filled with that value, then the default becomes `due` for the rows to come.
ALTER TABLE app_user
    ADD COLUMN tour_state text NOT NULL DEFAULT 'predates'
        CONSTRAINT app_user_tour_state_known
        CHECK (tour_state IN ('due', 'predates', 'completed', 'skipped')),
    ADD COLUMN tour_settled_at timestamptz,
    ADD CONSTRAINT app_user_tour_settled_dated
        CHECK ((tour_state IN ('completed', 'skipped')) = (tour_settled_at IS NOT NULL));

ALTER TABLE app_user ALTER COLUMN tour_state SET DEFAULT 'due';
