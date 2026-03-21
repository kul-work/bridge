-- Bridge: Row-Level Security (RLS) for Multi-App Tenant Isolation
--
-- Defense-in-depth: ensures app_id isolation at the database level, even if
-- application code has a bug or search_path is misconfigured in a shared-database
-- deployment (see architecture doc Section 5: Shared Database → Separate Databases).
--
-- How it works:
--   1. Bridge sets a session variable before each request:
--      SET LOCAL bridge.current_app_id = '<app-uuid>';
--   2. RLS policies restrict all SELECT/INSERT/UPDATE/DELETE to rows matching that app_id.
--   3. The superuser / migration role (bridge_admin) bypasses RLS for admin operations.
--
-- Roles (bridge_admin, bridge_app) are created in db-install-roles-rls.sql
-- Schema access and grants are set up there as well.
--   bridge_admin  — used for migrations, background jobs, admin UI (BYPASSRLS)
--   bridge_app    — used by the Axum application for per-request queries (subject to RLS)

-- ============================================================================
-- 1. Enable RLS on all tenant-scoped tables in pay schema
-- ============================================================================

ALTER TABLE pay.api_keys              ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.subscriptions         ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.payments              ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_log           ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_delivery      ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.agent_credits         ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.agent_transactions    ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.agent_payment_tokens  ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.fraud_prevention      ENABLE ROW LEVEL SECURITY;

-- apps table is the registry itself — no tenant scoping needed.
-- bridge_admin bypasses RLS via BYPASSRLS role attribute.

-- ============================================================================
-- 2. Create RLS policies (bridge_app role only)
-- ============================================================================
-- Each policy restricts rows to current_setting('bridge.current_app_id').
-- The app MUST call SET LOCAL bridge.current_app_id = '...' at the start of
-- each transaction/request, otherwise queries return zero rows (fail-closed).

-- api_keys
CREATE POLICY tenant_isolation_api_keys ON pay.api_keys
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- subscriptions
CREATE POLICY tenant_isolation_subscriptions ON pay.subscriptions
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- payments
CREATE POLICY tenant_isolation_payments ON pay.payments
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- webhook_log
CREATE POLICY tenant_isolation_webhook_log ON pay.webhook_log
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- webhook_delivery
CREATE POLICY tenant_isolation_webhook_delivery ON pay.webhook_delivery
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- agent_credits
CREATE POLICY tenant_isolation_agent_credits ON pay.agent_credits
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- agent_transactions
CREATE POLICY tenant_isolation_agent_transactions ON pay.agent_transactions
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- agent_payment_tokens
CREATE POLICY tenant_isolation_agent_payment_tokens ON pay.agent_payment_tokens
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- fraud_prevention
CREATE POLICY tenant_isolation_fraud_prevention ON pay.fraud_prevention
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id')::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id')::uuid);

-- ============================================================================
-- 3. Comments
-- ============================================================================

COMMENT ON POLICY tenant_isolation_api_keys ON pay.api_keys IS
    'RLS: restrict api_keys access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_subscriptions ON pay.subscriptions IS
    'RLS: restrict subscriptions access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_payments ON pay.payments IS
    'RLS: restrict payments access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_webhook_log ON pay.webhook_log IS
    'RLS: restrict webhook_log access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_webhook_delivery ON pay.webhook_delivery IS
    'RLS: restrict webhook_delivery access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_agent_credits ON pay.agent_credits IS
    'RLS: restrict agent_credits access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_agent_transactions ON pay.agent_transactions IS
    'RLS: restrict agent_transactions access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_agent_payment_tokens ON pay.agent_payment_tokens IS
    'RLS: restrict agent_payment_tokens access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_fraud_prevention ON pay.fraud_prevention IS
    'RLS: restrict fraud_prevention access to current app_id session variable.';
