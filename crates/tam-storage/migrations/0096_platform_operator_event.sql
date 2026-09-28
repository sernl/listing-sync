-- Every grant and withdrawal of the operator marking, as a trail.
--
-- platform_operator holds the marking's current state, and one row per human
-- (migration 0037): a re-grant restamps `granted_at`/`granted_by` and a
-- withdrawal records only its instant. That was enough while the one-shot on
-- the box was the only writer. The console now grants and withdraws the
-- marking too, so "who made this person an operator, and who took it away" is
-- a question with more than one possible answer, and the state row forgets it
-- the moment it changes. This table remembers.
--
-- Append-only by use: `OperatorRepo::grant` and `::revoke` insert one row in
-- the same transaction as the state change, and nothing updates or deletes
-- here.
--
-- `user_id` carries no foreign key, deliberately. Deleting a seller erases
-- their app_user row (migration 0092), and a trail that went with them would
-- stop answering the question it exists for exactly when it matters. The row
-- holds a subject-free uuid and the actor's name, nothing else about them.
--
-- `actor` is free text for the reason `platform_operator.granted_by` is: the
-- one-shot names whoever ran it, and the console names the operator as
-- `operator:<uuid>`.
--
-- Global, carrying no org_id and no row-level security, like the marking it
-- records (tests/rls_matrix.rs). tam_backoffice is granted nothing here: the
-- pool that reads every tenant cannot read who may use it, and cannot read
-- its history either (tests/backoffice_grants.rs).
CREATE TABLE platform_operator_event (
    id      bigint      GENERATED ALWAYS AS IDENTITY,
    user_id uuid        NOT NULL,
    action  text        NOT NULL,
    actor   text        NOT NULL,
    at      timestamptz NOT NULL,

    PRIMARY KEY (id),

    CONSTRAINT platform_operator_event_action_known CHECK (action IN ('grant', 'revoke')),
    CONSTRAINT platform_operator_event_actor_named CHECK (actor <> '')
);

CREATE INDEX platform_operator_event_by_user ON platform_operator_event (user_id, at);
