-- A source may be readable while every marketplace write remains fenced.
-- Existing halts keep their full-stop behavior. Claims continue to reject
-- either scope; only the advisory device read grant distinguishes them.
ALTER TABLE org_inventory_halt
    ADD COLUMN write_only boolean NOT NULL DEFAULT false;

-- The engine may escalate a write-only halt to a full stop, but still cannot
-- delete a halt and thereby resume marketplace writes.
GRANT UPDATE (write_only, raised_by, reason, raised_at)
    ON org_inventory_halt TO tam_engine;
