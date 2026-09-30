-- The inbox takes the console's own notices beside finished runs.
--
-- A toast the seller never looked at -- it came and went while they were in
-- another tab -- is kept here, so the bell can say what they missed. Those
-- rows are the seller's own rather than the organisation's: a notice says
-- what happened to the person who pressed the button, so it carries a
-- `user_id` and only that user reads it. A finished run stays organisation
-- wide, with no user.
--
-- One table rather than a second one beside it, because the bell and the
-- Notifications page are one list, newest first, with one keyset and one
-- unread count; two tables would make every page of it a union.
--
-- `kind` gains `notice`. A notice names no run, so `subject_id` and `counts`
-- are null for it and only for it; it says its own `title` and `body` and
-- carries the `tone` of the toast it was. A run's tone is not stored: it is
-- its worst outcome, read from `counts`, so the two cannot disagree.
--
-- `dismissed_at` is the seller's Delete. The row stays, so a dismissal is an
-- update the seller can make and no role here ever needs DELETE.

ALTER TABLE notification
    ALTER COLUMN subject_id DROP NOT NULL,
    ALTER COLUMN counts DROP NOT NULL,
    ADD COLUMN user_id uuid REFERENCES app_user (id) ON DELETE CASCADE,
    ADD COLUMN tone text,
    ADD COLUMN title text,
    ADD COLUMN body text,
    -- The console's own id for the toast, so a notice posted twice -- two
    -- tabs, or a retry after a timeout -- is one row.
    ADD COLUMN client_id text,
    ADD COLUMN dismissed_at timestamptz;

ALTER TABLE notification DROP CONSTRAINT notification_kind;
ALTER TABLE notification ADD CONSTRAINT notification_kind
    CHECK (kind IN ('sync', 'migration', 'import', 'notice'));

ALTER TABLE notification DROP CONSTRAINT notification_import_names_no_marketplace;
ALTER TABLE notification ADD CONSTRAINT notification_import_names_no_marketplace
    CHECK ((kind IN ('import', 'notice')) = (inventory IS NULL));

ALTER TABLE notification ADD CONSTRAINT notification_notice_shape CHECK (
    CASE WHEN kind = 'notice' THEN
        subject_id IS NULL AND counts IS NULL
        AND user_id IS NOT NULL AND client_id IS NOT NULL
        AND tone IN ('success', 'info', 'warning', 'error')
        AND title IS NOT NULL AND body IS NOT NULL
        AND char_length(title) BETWEEN 1 AND 200
        AND char_length(body) <= 1000
        AND char_length(client_id) BETWEEN 1 AND 64
    ELSE
        subject_id IS NOT NULL AND counts IS NOT NULL
        AND user_id IS NULL AND client_id IS NULL
        AND tone IS NULL AND title IS NULL AND body IS NULL
    END
);

-- One row per toast, as the run's own index is one row per run.
CREATE UNIQUE INDEX notification_one_per_notice
    ON notification (org_id, user_id, client_id)
    WHERE client_id IS NOT NULL;

-- The fence gains the user. A row with no user is the organisation's, as
-- every row was before; a row with one is visible to that user alone, keyed
-- on `app.current_user`, which the notification repository pins beside
-- `app.current_org` in the same transaction. A path that pins no user -- the
-- settle that writes a run's row -- sees and writes organisation rows only.
DROP POLICY notification_org_isolation ON notification;
CREATE POLICY notification_org_isolation ON notification
    FOR ALL
    USING (
        org_id = NULLIF(current_setting('app.current_org', true), '')::uuid
        AND (
            user_id IS NULL
            OR user_id = NULLIF(current_setting('app.current_user', true), '')::uuid
        )
    )
    WITH CHECK (
        org_id = NULLIF(current_setting('app.current_org', true), '')::uuid
        AND (
            user_id IS NULL
            OR user_id = NULLIF(current_setting('app.current_user', true), '')::uuid
        )
    );
