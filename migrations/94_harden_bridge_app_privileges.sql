SET search_path TO pay, public;

-- Harden runtime role: remove CREATE, _sqlx_migrations access, and PUBLIC EXECUTE.
-- Verified safe against code (2026-06-17 RLS audit, findings 1/3/4):
--   - No runtime CREATE statements in src/
--   - _sqlx_migrations only touched by sqlx::migrate! run as bridge_admin
--   - cleanup functions are called by the runtime scheduler, so bridge_app needs EXECUTE

-- Finding 1: runtime role should not create objects in the pay schema.
REVOKE CREATE ON SCHEMA pay FROM bridge_app;

-- Finding 3: migrations are run as bridge_admin; runtime role does not need access.
REVOKE ALL ON TABLE pay._sqlx_migrations FROM bridge_app;

-- Finding 4: drop PUBLIC EXECUTE, grant explicitly to roles that need it.
REVOKE EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION pay.current_app_id() FROM PUBLIC;

GRANT EXECUTE ON FUNCTION pay.current_app_id() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.current_app_id() TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() TO bridge_admin;
