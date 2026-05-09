SET search_path TO pay, public;

-- Bridge: RLS for checkout idempotency cache.
--
-- This table is tenant-scoped by app_id and was added after the original RLS
-- migrations. Keep it aligned with the rest of Bridge's app-scoped tables.

ALTER TABLE pay.checkout_idempotency ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS tenant_isolation_checkout_idempotency ON pay.checkout_idempotency;
CREATE POLICY tenant_isolation_checkout_idempotency ON pay.checkout_idempotency
    FOR ALL
    TO bridge_app
    USING (app_id = current_app_id())
    WITH CHECK (app_id = current_app_id());

COMMENT ON POLICY tenant_isolation_checkout_idempotency ON pay.checkout_idempotency IS
    'RLS: restrict checkout idempotency cache access to current app_id session variable.';
