SET search_path TO pay, public;

-- Bridge: Harden RLS app_id lookup against empty session values.
--
-- Some pooled connections can end up with bridge.current_app_id reset to an
-- empty string after a transaction-scoped set_config(..., true). Casting that
-- directly to uuid raises `invalid input syntax for type uuid: ""`.
--
-- This helper normalizes empty strings to NULL before the cast, so bootstrap
-- SELECT policies on webhook tables keep working after prior tx-local resets.

CREATE OR REPLACE FUNCTION current_app_id()
RETURNS uuid
LANGUAGE sql
STABLE
AS $$
    SELECT NULLIF(current_setting('bridge.current_app_id', true), '')::uuid;
$$;

GRANT EXECUTE ON FUNCTION current_app_id() TO bridge_app, bridge_admin;

DROP POLICY IF EXISTS tenant_isolation_api_keys ON pay.api_keys;
CREATE POLICY tenant_isolation_api_keys ON pay.api_keys
    FOR ALL
    TO bridge_app
    USING (app_id = current_app_id())
    WITH CHECK (app_id = current_app_id());

DROP POLICY IF EXISTS tenant_isolation_subscriptions ON pay.subscriptions;
CREATE POLICY tenant_isolation_subscriptions ON pay.subscriptions
    FOR ALL
    TO bridge_app
    USING (app_id = current_app_id())
    WITH CHECK (app_id = current_app_id());

DROP POLICY IF EXISTS tenant_isolation_payments ON pay.payments;
CREATE POLICY tenant_isolation_payments ON pay.payments
    FOR ALL
    TO bridge_app
    USING (app_id = current_app_id())
    WITH CHECK (app_id = current_app_id());

DROP POLICY IF EXISTS tenant_isolation_webhook_provider ON pay.webhook_provider;
CREATE POLICY tenant_isolation_webhook_provider ON pay.webhook_provider
    FOR ALL
    TO bridge_app
    USING (app_id = current_app_id())
    WITH CHECK (app_id = current_app_id());

DROP POLICY IF EXISTS tenant_isolation_webhook_delivery ON pay.webhook_delivery;
CREATE POLICY tenant_isolation_webhook_delivery ON pay.webhook_delivery
    FOR ALL
    TO bridge_app
    USING (app_id = current_app_id())
    WITH CHECK (app_id = current_app_id());

DROP POLICY IF EXISTS tenant_isolation_provider_configs ON pay.provider_configs;
CREATE POLICY tenant_isolation_provider_configs ON pay.provider_configs
    FOR ALL
    TO bridge_app
    USING (app_id = current_app_id())
    WITH CHECK (app_id = current_app_id());

DROP POLICY IF EXISTS tenant_isolation_fraud_prevention ON pay.fraud_prevention;
CREATE POLICY tenant_isolation_fraud_prevention ON pay.fraud_prevention
    FOR ALL
    TO bridge_app
    USING (app_id = current_app_id())
    WITH CHECK (app_id = current_app_id());

DROP POLICY IF EXISTS tenant_isolation_api_keys_bootstrap_select ON pay.api_keys;
CREATE POLICY tenant_isolation_api_keys_bootstrap_select ON pay.api_keys
    FOR SELECT
    TO bridge_app
    USING (
        current_app_id() IS NULL
        OR app_id = current_app_id()
    );

DROP POLICY IF EXISTS tenant_isolation_provider_configs_bootstrap_select ON pay.provider_configs;
CREATE POLICY tenant_isolation_provider_configs_bootstrap_select ON pay.provider_configs
    FOR SELECT
    TO bridge_app
    USING (
        current_app_id() IS NULL
        OR app_id = current_app_id()
    );

DROP POLICY IF EXISTS tenant_isolation_webhook_provider_bootstrap_select ON pay.webhook_provider;
CREATE POLICY tenant_isolation_webhook_provider_bootstrap_select ON pay.webhook_provider
    FOR SELECT
    TO bridge_app
    USING (
        current_app_id() IS NULL
        OR app_id = current_app_id()
    );

DROP POLICY IF EXISTS tenant_isolation_webhook_delivery_bootstrap_select ON pay.webhook_delivery;
CREATE POLICY tenant_isolation_webhook_delivery_bootstrap_select ON pay.webhook_delivery
    FOR SELECT
    TO bridge_app
    USING (
        current_app_id() IS NULL
        OR app_id = current_app_id()
    );
