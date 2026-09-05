-- D32: a resource kept on Teachouse alone may carry no file; a marketplace
-- listing may not. The payload requirement is therefore not a property of the
-- product but of the mapping, and this migration moves it there.
--
-- Relaxing, never dropping. `product_payload_nonempty` and
-- `product_file_payload_nonempty` in 0003_catalogue.sql are applied and frozen,
-- so the function they both call is redefined rather than the triggers being
-- removed: `assert_product_has_payload` keeps its name, its signature and both
-- attachments, and stops raising for a product no mapping names.
--
-- Not reversible past this point. Relaxing the rule reverses cleanly; the
-- payload-less products created afterwards do not, so a rollback has to deal
-- with those rows first. That obligation belongs to the runbook, and is named
-- here so it is not discovered from a failed migration.

CREATE OR REPLACE FUNCTION assert_product_has_payload() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    checked_org     uuid;
    checked_product uuid;
BEGIN
    IF TG_TABLE_NAME = 'product' THEN
        checked_org     := NEW.org_id;
        checked_product := NEW.id;
    ELSE
        checked_org     := COALESCE(NEW.org_id, OLD.org_id);
        checked_product := COALESCE(NEW.product_id, OLD.product_id);
    END IF;

    PERFORM 1 FROM product p
        WHERE p.org_id = checked_org AND p.id = checked_product
          AND p.deleted_at IS NULL;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    -- The relaxation, and the whole of it. A product no marketplace carries is
    -- a draft kept here, and a draft is allowed to have no file yet.
    PERFORM 1 FROM mapping m
        WHERE m.org_id = checked_org AND m.product_id = checked_product;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    PERFORM 1 FROM product_file f
        WHERE f.org_id = checked_org AND f.product_id = checked_product
          AND f.role = 'payload' AND f.deleted_at IS NULL;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'product % reaches a marketplace and has no live payload file',
            checked_product
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

-- The other direction, which the two frozen triggers cannot see: they fire on
-- `product` and on `product_file`, so a mapping inserted onto a payload-less
-- product touches neither and would commit. This is the rule the API refuses
-- by name in `add_mapping`; the trigger is what holds when the code is wrong.
--
-- A constraint trigger, deferred like its siblings, so a create that writes the
-- product, its payload and its mappings in one transaction is judged on what
-- the transaction leaves behind rather than on the order it wrote it in.
CREATE FUNCTION assert_mapping_has_payload() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM 1 FROM product p
        WHERE p.org_id = NEW.org_id AND p.id = NEW.product_id
          AND p.deleted_at IS NULL;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;

    PERFORM 1 FROM product_file f
        WHERE f.org_id = NEW.org_id AND f.product_id = NEW.product_id
          AND f.role = 'payload' AND f.deleted_at IS NULL;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'mapping % names a product with no live payload file', NEW.id
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END
$$;

CREATE CONSTRAINT TRIGGER mapping_payload_nonempty
    AFTER INSERT OR UPDATE ON mapping
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION assert_mapping_has_payload();
