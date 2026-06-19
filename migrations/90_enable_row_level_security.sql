SET search_path TO pay, public;

-- Bridge RLS helpers and narrow bootstrap functions.
--
-- Runtime request queries set bridge.current_app_id with SET LOCAL after the
-- app is resolved. The few pre-context lookup paths use SECURITY DEFINER
-- functions instead of broad table-level bootstrap SELECT policies.

CREATE OR REPLACE FUNCTION pay.current_app_id()
RETURNS uuid
LANGUAGE sql
STABLE
AS $$
    SELECT NULLIF(current_setting('bridge.current_app_id', true), '')::uuid;
$$;

DROP FUNCTION IF EXISTS pay.get_webhook_provider_app_id_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.get_webhook_delivery_app_id_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.get_webhook_provider_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.get_webhook_delivery_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.webhook_delivery_exists_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.get_app_by_webhook_token_bootstrap(UUID);
DROP FUNCTION IF EXISTS pay.list_enabled_app_ids_bootstrap();
DROP FUNCTION IF EXISTS pay.list_app_summaries_bootstrap();
DROP FUNCTION IF EXISTS pay.get_api_key_auth_candidates_bootstrap(TEXT);

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

CREATE OR REPLACE FUNCTION pay.cleanup_old_webhook_provider()
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pay, pg_temp
AS $$
BEGIN
    DELETE FROM pay.webhook_provider
    WHERE created_at < NOW() - INTERVAL '90 days';
END;
$$;

CREATE OR REPLACE FUNCTION pay.cleanup_purged_fraud_prevention()
RETURNS void
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pay, pg_temp
AS $$
BEGIN
    DELETE FROM pay.fraud_prevention
    WHERE should_purge_at IS NOT NULL
      AND should_purge_at < NOW();
END;
$$;

REVOKE ALL ON FUNCTION pay.current_app_id() FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_webhook_provider_app_id_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_webhook_delivery_app_id_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_webhook_provider_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_webhook_delivery_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.webhook_delivery_exists_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_app_by_webhook_token_bootstrap(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.list_enabled_app_ids_bootstrap() FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.list_app_summaries_bootstrap() FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.get_api_key_auth_candidates_bootstrap(TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.cleanup_old_webhook_provider() FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.cleanup_purged_fraud_prevention() FROM PUBLIC;

GRANT EXECUTE ON FUNCTION pay.current_app_id() TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_webhook_provider_app_id_bootstrap(UUID) TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_webhook_delivery_app_id_bootstrap(UUID) TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_webhook_provider_bootstrap(UUID) TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_webhook_delivery_bootstrap(UUID) TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.webhook_delivery_exists_bootstrap(UUID) TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_app_by_webhook_token_bootstrap(UUID) TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.list_enabled_app_ids_bootstrap() TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.list_app_summaries_bootstrap() TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.get_api_key_auth_candidates_bootstrap(TEXT) TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() TO bridge_app, bridge_admin;

COMMENT ON FUNCTION pay.cleanup_old_webhook_provider() IS
    'Delete webhook_provider records older than 90 days. SECURITY DEFINER so the runtime worker does not need direct DELETE grants.';
COMMENT ON FUNCTION pay.cleanup_purged_fraud_prevention() IS
    'Delete fraud_prevention records past should_purge_at. SECURITY DEFINER so the runtime worker does not need direct DELETE grants.';
