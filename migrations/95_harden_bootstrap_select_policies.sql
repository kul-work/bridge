-- Replace broad bootstrap SELECT policies with narrow SECURITY DEFINER lookups.
-- Only true pre-context webhook lookups may bypass tenant RLS; all app-scoped
-- reads must set bridge.current_app_id and use the normal tenant policy.

DROP FUNCTION IF EXISTS pay.get_webhook_provider_app_id_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.get_webhook_delivery_app_id_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.get_webhook_provider_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.get_webhook_delivery_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.webhook_delivery_exists_bootstrap(UUID);

CREATE OR REPLACE FUNCTION pay.get_webhook_provider_app_id_bootstrap(p_webhook_id UUID)
RETURNS UUID
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT app_id
    FROM pay.webhook_provider
    WHERE id = p_webhook_id
$$;

CREATE OR REPLACE FUNCTION pay.get_webhook_delivery_app_id_bootstrap(p_delivery_id UUID)
RETURNS UUID
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT app_id
    FROM pay.webhook_delivery
    WHERE id = p_delivery_id
$$;

CREATE OR REPLACE FUNCTION pay.get_webhook_provider_bootstrap(p_webhook_id UUID)
RETURNS SETOF pay.webhook_provider
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT *
    FROM pay.webhook_provider
    WHERE id = p_webhook_id
$$;

CREATE OR REPLACE FUNCTION pay.get_webhook_delivery_bootstrap(p_delivery_id UUID)
RETURNS SETOF pay.webhook_delivery
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT *
    FROM pay.webhook_delivery
    WHERE id = p_delivery_id
$$;

CREATE OR REPLACE FUNCTION pay.webhook_delivery_exists_bootstrap(p_webhook_provider_id UUID)
RETURNS BOOLEAN
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT EXISTS(
        SELECT 1
        FROM pay.webhook_delivery
        WHERE webhook_provider_id = p_webhook_provider_id
    )
$$;

REVOKE ALL ON FUNCTION pay.get_webhook_provider_app_id_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_webhook_delivery_app_id_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_webhook_provider_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_webhook_delivery_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.webhook_delivery_exists_bootstrap(UUID) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION pay.get_webhook_provider_app_id_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_webhook_delivery_app_id_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_webhook_provider_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.get_webhook_delivery_bootstrap(UUID) TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.webhook_delivery_exists_bootstrap(UUID) TO bridge_app;

GRANT EXECUTE ON FUNCTION pay.get_webhook_provider_app_id_bootstrap(UUID) TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_webhook_delivery_app_id_bootstrap(UUID) TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_webhook_provider_bootstrap(UUID) TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_webhook_delivery_bootstrap(UUID) TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.webhook_delivery_exists_bootstrap(UUID) TO bridge_admin;

DROP POLICY IF EXISTS tenant_isolation_provider_configs_bootstrap_select ON pay.provider_configs;
DROP POLICY IF EXISTS tenant_isolation_webhook_provider_bootstrap_select ON pay.webhook_provider;
DROP POLICY IF EXISTS tenant_isolation_webhook_delivery_bootstrap_select ON pay.webhook_delivery;
