SET search_path TO pay, public;

-- Final runtime privileges for bridge_app.
--
-- Role and schema creation stays in docs/db-install-roles-rls.sql. Runtime
-- table access is intentionally narrow here, with pre-context access routed
-- through the SECURITY DEFINER functions from migration 90.

REVOKE CREATE ON SCHEMA pay FROM bridge_app;
REVOKE ALL ON TABLE pay._sqlx_migrations FROM bridge_app;
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

GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pay TO bridge_app;

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

REVOKE SELECT ON pay.v_data_retention_stats FROM bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
REVOKE SELECT, INSERT, UPDATE, DELETE ON TABLES FROM bridge_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA pay
REVOKE EXECUTE ON FUNCTIONS FROM bridge_app;
