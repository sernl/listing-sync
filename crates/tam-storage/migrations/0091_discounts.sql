-- Discounts: sale periods, one-off discounts and the codes sellers type at
-- checkout, each one a Stripe Coupon this table remembers the identifier of.
--
-- Three kinds share one table because they are one Stripe object with three
-- ways of reaching a checkout:
--
--   sale     applied automatically to every plan while its window is open,
--            and announced: its banner and theme live in site_setting under
--            `sale.<id>` (migration 0087), because they are presentation the
--            landing and the console both read, not terms Stripe enforces.
--   one_off  applied automatically to the price keys it names while its
--            window is open, and announced nowhere.
--   code     applied only when a seller types one of its codes; the codes
--            are rows of discount_code, one Stripe Promotion Code each.
--
-- Nothing here is tenant data. A discount is the platform's offer to every
-- organisation, so the table carries no org_id, forces no row-level security
-- and grants the backoffice role nothing -- the same footing as `guide`
-- (migration 0073). The application role owns it by owning the database.
--
-- The window is [starts_at, ends_at): the admin page names inclusive dates
-- and the API turns the last one into the midnight (UTC) after it, so the
-- comparison is one half-open interval everywhere and never a date in one
-- place and an instant in another.
--
-- Terms are immutable once the coupon exists. Stripe refuses to change a
-- coupon's amount, duration or product set after creation, and a row whose
-- terms had drifted from its coupon's would show one price and charge
-- another. What an operator can change is whether it is still offered:
-- ended_at records that it was withdrawn before its window closed, and the
-- coupon is deleted in Stripe at the same moment so it can no longer be
-- redeemed there either.

CREATE TABLE discount (
    id               uuid        NOT NULL,
    kind             text        NOT NULL,
    name             text        NOT NULL,

    -- Exactly one of the two is set. percent_off is a whole percentage;
    -- amount_off_cents is in `currency`, which is the currency the price
    -- list is quoted in.
    percent_off      integer,
    amount_off_cents integer,
    currency         text        NOT NULL DEFAULT 'usd',

    -- `once` discounts the first invoice (or the one payment), `repeating`
    -- the first duration_months invoices of a subscription.
    duration         text        NOT NULL,
    duration_months  integer,

    -- The tam-limits price keys this applies to. Empty means every plan
    -- (a subscription price key); a sale is always empty.
    price_keys       text[]      NOT NULL DEFAULT '{}',

    starts_at        timestamptz NOT NULL,
    ends_at          timestamptz NOT NULL,

    stripe_coupon_id text        NOT NULL,

    ended_at         timestamptz,
    created_by       uuid        REFERENCES app_user (id),
    created_at       timestamptz NOT NULL,

    PRIMARY KEY (id),
    CONSTRAINT discount_stripe_coupon UNIQUE (stripe_coupon_id),
    CONSTRAINT discount_kind CHECK (kind IN ('sale', 'one_off', 'code')),
    CONSTRAINT discount_name_present CHECK (length(name) BETWEEN 1 AND 80),
    CONSTRAINT discount_one_amount CHECK (
        (percent_off IS NULL) <> (amount_off_cents IS NULL)
    ),
    CONSTRAINT discount_percent_range CHECK (
        percent_off IS NULL OR percent_off BETWEEN 1 AND 100
    ),
    CONSTRAINT discount_amount_positive CHECK (
        amount_off_cents IS NULL OR amount_off_cents > 0
    ),
    CONSTRAINT discount_duration CHECK (
        (duration = 'once' AND duration_months IS NULL)
        OR (duration = 'repeating' AND duration_months BETWEEN 1 AND 36)
    ),
    CONSTRAINT discount_window CHECK (ends_at > starts_at),
    CONSTRAINT discount_sale_is_percent_on_every_plan CHECK (
        kind <> 'sale' OR (percent_off IS NOT NULL AND price_keys = '{}')
    )
);

CREATE INDEX discount_open ON discount (starts_at, ends_at) WHERE ended_at IS NULL;

-- One typed code. `code` is stored as the operator wrote it and matched
-- case-insensitively, which is also how Stripe matches a promotion code on
-- its own checkout page.
CREATE TABLE discount_code (
    id                        uuid        NOT NULL,
    discount_id               uuid        NOT NULL REFERENCES discount (id),
    code                      text        NOT NULL,
    stripe_promotion_code_id  text        NOT NULL,
    max_redemptions           integer,
    ended_at                  timestamptz,
    created_at                timestamptz NOT NULL,

    PRIMARY KEY (id),
    CONSTRAINT discount_code_stripe UNIQUE (stripe_promotion_code_id),
    CONSTRAINT discount_code_shape CHECK (code ~ '^[A-Za-z0-9_-]{3,40}$'),
    CONSTRAINT discount_code_redemptions CHECK (
        max_redemptions IS NULL OR max_redemptions > 0
    )
);

-- Stripe keeps one active promotion code per string, so this does too: a
-- code can be reused only once the earlier one has been ended.
CREATE UNIQUE INDEX discount_code_live ON discount_code (lower(code)) WHERE ended_at IS NULL;
