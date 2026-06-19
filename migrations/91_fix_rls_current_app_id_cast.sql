SET search_path TO pay, public;

-- Bridge row-level security for tenant-scoped runtime access.
--
-- All normal runtime access is scoped by pay.current_app_id(). Pre-context
-- auth/webhook/admin-list lookups use the narrow SECURITY DEFINER functions
-- created in the previous migration.

ALTER TABLE pay.apps                 ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.api_keys             ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.checkout_idempotency ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.fraud_prevention     ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.payments             ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.provider_configs     ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.subscriptions        ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_delivery     ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_provider     ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS tenant_isolation_apps ON pay.apps;
CREATE POLICY tenant_isolation_apps ON pay.apps
    FOR ALL
    TO bridge_app
    USING (id = pay.current_app_id())
    WITH CHECK (id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_api_keys ON pay.api_keys;
CREATE POLICY tenant_isolation_api_keys ON pay.api_keys
    FOR ALL
    TO bridge_app
    USING (app_id = pay.current_app_id())
    WITH CHECK (app_id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_checkout_idempotency ON pay.checkout_idempotency;
CREATE POLICY tenant_isolation_checkout_idempotency ON pay.checkout_idempotency
    FOR ALL
    TO bridge_app
    USING (app_id = pay.current_app_id())
    WITH CHECK (app_id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_fraud_prevention ON pay.fraud_prevention;
CREATE POLICY tenant_isolation_fraud_prevention ON pay.fraud_prevention
    FOR ALL
    TO bridge_app
    USING (app_id = pay.current_app_id())
    WITH CHECK (app_id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_payments ON pay.payments;
CREATE POLICY tenant_isolation_payments ON pay.payments
    FOR ALL
    TO bridge_app
    USING (app_id = pay.current_app_id())
    WITH CHECK (app_id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_provider_configs ON pay.provider_configs;
CREATE POLICY tenant_isolation_provider_configs ON pay.provider_configs
    FOR ALL
    TO bridge_app
    USING (app_id = pay.current_app_id())
    WITH CHECK (app_id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_subscriptions ON pay.subscriptions;
CREATE POLICY tenant_isolation_subscriptions ON pay.subscriptions
    FOR ALL
    TO bridge_app
    USING (app_id = pay.current_app_id())
    WITH CHECK (app_id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_webhook_delivery ON pay.webhook_delivery;
CREATE POLICY tenant_isolation_webhook_delivery ON pay.webhook_delivery
    FOR ALL
    TO bridge_app
    USING (app_id = pay.current_app_id())
    WITH CHECK (app_id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_webhook_provider ON pay.webhook_provider;
CREATE POLICY tenant_isolation_webhook_provider ON pay.webhook_provider
    FOR ALL
    TO bridge_app
    USING (app_id = pay.current_app_id())
    WITH CHECK (app_id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_api_keys_bootstrap_select ON pay.api_keys;
DROP POLICY IF EXISTS tenant_isolation_provider_configs_bootstrap_select ON pay.provider_configs;
DROP POLICY IF EXISTS tenant_isolation_webhook_provider_bootstrap_select ON pay.webhook_provider;
DROP POLICY IF EXISTS tenant_isolation_webhook_delivery_bootstrap_select ON pay.webhook_delivery;

ALTER TABLE pay.apps                 FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.api_keys             FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.checkout_idempotency FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.fraud_prevention     FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.payments             FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.provider_configs     FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.subscriptions        FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_delivery     FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_provider     FORCE ROW LEVEL SECURITY;

COMMENT ON POLICY tenant_isolation_apps ON pay.apps IS
    'RLS: restrict app metadata access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_api_keys ON pay.api_keys IS
    'RLS: restrict api_keys access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_checkout_idempotency ON pay.checkout_idempotency IS
    'RLS: restrict checkout idempotency cache access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_fraud_prevention ON pay.fraud_prevention IS
    'RLS: restrict fraud_prevention access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_payments ON pay.payments IS
    'RLS: restrict payments access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_provider_configs ON pay.provider_configs IS
    'RLS: restrict provider_configs access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_subscriptions ON pay.subscriptions IS
    'RLS: restrict subscriptions access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_webhook_delivery ON pay.webhook_delivery IS
    'RLS: restrict webhook_delivery access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_webhook_provider ON pay.webhook_provider IS
    'RLS: restrict webhook_provider access to current app_id session variable.';
