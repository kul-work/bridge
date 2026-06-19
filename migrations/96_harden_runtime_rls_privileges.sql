SET search_path TO pay, public;

-- Harden app/auth bootstrap paths. Broad table-level bootstrap SELECT policies
-- are replaced with narrow SECURITY DEFINER functions.

DROP FUNCTION IF EXISTS pay.get_app_by_webhook_token_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.list_enabled_app_ids_bootstrap();
DROP FUNCTION IF EXISTS pay.list_app_summaries_bootstrap();
DROP FUNCTION IF EXISTS pay.get_api_key_auth_candidates_bootstrap(TEXT);

CREATE OR REPLACE FUNCTION pay.get_app_by_webhook_token_bootstrap(p_token UUID)
RETURNS SETOF pay.apps
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT *
    FROM pay.apps
    WHERE webhook_ingress_token = p_token
      AND enabled = true
$$;

CREATE OR REPLACE FUNCTION pay.list_enabled_app_ids_bootstrap()
RETURNS TABLE(id UUID)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT apps.id
    FROM pay.apps
    WHERE enabled = true
    ORDER BY display_name
$$;

CREATE OR REPLACE FUNCTION pay.list_app_summaries_bootstrap()
RETURNS TABLE(
    id UUID,
    slug TEXT,
    display_name TEXT,
    app_url TEXT
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT apps.id, apps.slug, apps.display_name, apps.app_url
    FROM pay.apps
    ORDER BY display_name
$$;

CREATE OR REPLACE FUNCTION pay.get_api_key_auth_candidates_bootstrap(p_key_prefix TEXT)
RETURNS TABLE(
    id UUID,
    app_id UUID,
    key_hash TEXT,
    app_enabled BOOLEAN
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT k.id, k.app_id, k.key_hash, a.enabled AS app_enabled
    FROM pay.api_keys k
    JOIN pay.apps a ON a.id = k.app_id
    WHERE k.enabled = true
      AND k.key_prefix = p_key_prefix
$$;

REVOKE ALL ON FUNCTION pay.get_app_by_webhook_token_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.list_enabled_app_ids_bootstrap() FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.list_app_summaries_bootstrap() FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_api_key_auth_candidates_bootstrap(TEXT) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION pay.get_app_by_webhook_token_bootstrap(UUID) TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.list_enabled_app_ids_bootstrap() TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.list_app_summaries_bootstrap() TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_api_key_auth_candidates_bootstrap(TEXT) TO bridge_app, bridge_admin;

ALTER TABLE pay.apps ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS tenant_isolation_apps ON pay.apps;
CREATE POLICY tenant_isolation_apps ON pay.apps
    FOR ALL
    TO bridge_app
    USING (id = pay.current_app_id())
    WITH CHECK (id = pay.current_app_id());

DROP POLICY IF EXISTS tenant_isolation_api_keys_bootstrap_select ON pay.api_keys;

ALTER TABLE pay.api_keys FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.checkout_idempotency FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.fraud_prevention FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.payments FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.provider_configs FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.subscriptions FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_delivery FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_provider FORCE ROW LEVEL SECURITY;

REVOKE SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pay FROM bridge_app;
REVOKE EXECUTE ON ALL FUNCTIONS IN SCHEMA pay FROM bridge_app;

GRANT SELECT ON pay.apps TO bridge_app;
GRANT SELECT, UPDATE ON pay.api_keys TO bridge_app;
GRANT SELECT, INSERT ON pay.checkout_idempotency TO bridge_app;
GRANT SELECT, INSERT, UPDATE ON pay.fraud_prevention TO bridge_app;
GRANT SELECT, INSERT, UPDATE ON pay.payments TO bridge_app;
GRANT SELECT ON pay.provider_configs TO bridge_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON pay.subscriptions TO bridge_app;
GRANT SELECT, INSERT, UPDATE ON pay.webhook_delivery TO bridge_app;
GRANT SELECT, INSERT, UPDATE ON pay.webhook_provider TO bridge_app;

GRANT EXECUTE ON FUNCTION pay.current_app_id() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_webhook_provider_app_id_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_webhook_delivery_app_id_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_webhook_provider_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_webhook_delivery_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.webhook_delivery_exists_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_app_by_webhook_token_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.list_enabled_app_ids_bootstrap() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.list_app_summaries_bootstrap() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_api_key_auth_candidates_bootstrap(TEXT) TO bridge_app;

GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pay TO bridge_app;
REVOKE SELECT ON pay.v_data_retention_stats FROM bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
REVOKE SELECT, INSERT, UPDATE, DELETE ON TABLES FROM bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
REVOKE EXECUTE ON FUNCTIONS FROM bridge_app;
