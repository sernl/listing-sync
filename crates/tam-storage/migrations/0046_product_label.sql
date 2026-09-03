-- Seller-defined labels, and which items carry them.
--
-- Gap G6 in `docs/notes/design/seller-dashboard.md`: labels are the dimension
-- a teacher files units, seasons and sale participation by, and filtering was
-- title, marketplace and standing until this table existed. They are the
-- seller's own vocabulary rather than ours, so nothing here constrains what a
-- label may say beyond a length bound.
--
-- Labels are per organisation rather than per product: the same "Autumn term"
-- is one label across a catalogue, which is what makes filtering by it
-- meaningful and what stops a rename having to visit every item.
--
-- The colour is a closed set of names rather than free text or a hex triple,
-- following `device_marketplace_session.status` in 0042: a closed set keeps
-- the console's rendering total, so a colour added here fails the web lane
-- rather than reaching a seller as an unstyled chip. Names rather than hex
-- because the console maps them to its own theme tokens, and a light and a
-- dark theme need different triples for one colour; storing hex would pick
-- one theme's answer and freeze it into the data.
CREATE TABLE label (
    org_id     uuid        NOT NULL REFERENCES organisation (id),
    id         uuid        NOT NULL,
    name       text        NOT NULL,
    colour     text        NOT NULL,
    created_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, id),

    CONSTRAINT label_name_bounded CHECK (char_length(name) BETWEEN 1 AND 60),
    CONSTRAINT label_colour_closed CHECK (
        colour IN ('slate', 'red', 'amber', 'green', 'teal', 'blue', 'violet', 'pink')
    )
);

-- One label per name per organisation, compared case-insensitively: a seller
-- typing "autumn term" after "Autumn term" means the label they already have,
-- and two rows differing only in case would split a filter in half without
-- either half looking wrong.
CREATE UNIQUE INDEX label_one_per_name
    ON label (org_id, lower(name));

CREATE TABLE product_label (
    org_id     uuid        NOT NULL REFERENCES organisation (id),
    product_id uuid        NOT NULL,
    label_id   uuid        NOT NULL,
    applied_at timestamptz NOT NULL,

    PRIMARY KEY (org_id, product_id, label_id),

    FOREIGN KEY (org_id, product_id) REFERENCES product (org_id, id),
    -- A label removed from the organisation's vocabulary is removed from every
    -- item carrying it; the alternative is a filter offering a label no item
    -- can be found by, or an item carrying a label with no name.
    FOREIGN KEY (org_id, label_id) REFERENCES label (org_id, id) ON DELETE CASCADE
);

-- The filter reads "which items carry this label", so the index is on the
-- label rather than on the product the primary key already leads with.
CREATE INDEX product_label_by_label
    ON product_label (org_id, label_id);

ALTER TABLE label ENABLE ROW LEVEL SECURITY;
ALTER TABLE label FORCE ROW LEVEL SECURITY;
CREATE POLICY label_org_isolation ON label
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);

ALTER TABLE product_label ENABLE ROW LEVEL SECURITY;
ALTER TABLE product_label FORCE ROW LEVEL SECURITY;
CREATE POLICY product_label_org_isolation ON product_label
    FOR ALL
    USING (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid)
    WITH CHECK (org_id = NULLIF(current_setting('app.current_org', true), '')::uuid);
