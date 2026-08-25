-- Global reference data, not tenant data: the closed set of inventories a
-- listing can be created in. Keyed (code, marketplace) so a mapping's compound
-- FK cannot pair an inventory with the wrong marketplace.
CREATE TABLE marketplace_inventory (
    code        text NOT NULL,
    marketplace text NOT NULL,

    PRIMARY KEY (code, marketplace)
);

INSERT INTO marketplace_inventory (code, marketplace) VALUES
    ('tes_gb', 'tes'),
    ('tes_us', 'tes'),
    ('etsy',   'etsy'),
    ('tpt',    'tpt');
