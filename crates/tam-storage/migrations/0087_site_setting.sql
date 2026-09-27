-- Site-wide switches an operator flips from the console: maintenance mode,
-- the seasonal theme on the landing page, the announcement banner, and the
-- presentation of a sale.
--
-- Global, carrying no org_id, no row-level security and no policy, for the
-- reason guide carries none (migration 0073): a switch is a platform fact
-- that every visitor reads the same way, signed in or not. The closed-world
-- test in tests/rls_matrix.rs records that decision.
--
-- tam_backoffice is granted nothing here. The operator routes that write
-- these rows run on the application pool, and the public read does too.
--
-- One row per key, and the value is jsonb because each key has its own
-- shape: `maintenance` is {on, message}, `theme` is {name, from, until},
-- `banner` is {text, href} or null, and `sale.<id>` rows belong to the
-- discounts surface. The shapes are checked in crates/tam-api/src/site.rs,
-- where they are parsed; a key nobody reads is inert.
--
-- No row is seeded. An absent key reads as its off state (maintenance off,
-- no theme, no banner), so a database that never had an operator touch it
-- behaves exactly as the site did before this table existed.
CREATE TABLE site_setting (
    key        text        NOT NULL,
    value      jsonb       NOT NULL,
    updated_at timestamptz NOT NULL,

    -- Who last wrote it. Nullable for the reason guide.updated_by is: a row
    -- written by a one-shot has no author to name.
    updated_by uuid        REFERENCES app_user (id),

    PRIMARY KEY (key),

    -- Lowercase dotted words, so a key is something a reader can grep for.
    CONSTRAINT site_setting_key_shape CHECK (
        key ~ '^[a-z0-9_-]+(\.[a-z0-9_-]+)*$' AND length(key) BETWEEN 1 AND 120
    ),

    -- A switch is small. The bound keeps an admin form from becoming storage.
    CONSTRAINT site_setting_value_bounded CHECK (octet_length(value::text) <= 16384)
);
