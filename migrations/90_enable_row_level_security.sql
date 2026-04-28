SET search_path TO pay, public;

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
ALTER TABLE pay.webhook_provider      ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_delivery      ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.provider_configs      ENABLE ROW LEVEL SECURITY;
ALTER TABLE pay.fraud_prevention      ENABLE ROW LEVEL SECURITY;

-- apps table is the registry itself — no tenant scoping needed.
-- bridge_admin bypasses RLS via BYPASSRLS role attribute.

-- ============================================================================
-- 2. Tenant isolation policies (bridge_app role, FOR ALL)
-- ============================================================================
-- Each policy restricts rows to current_setting('bridge.current_app_id').
-- The app MUST call SET LOCAL bridge.current_app_id = '...' at the start of
-- each transaction/request, otherwise queries return zero rows (fail-closed).

-- api_keys
CREATE POLICY tenant_isolation_api_keys ON pay.api_keys
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

-- subscriptions
CREATE POLICY tenant_isolation_subscriptions ON pay.subscriptions
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

-- payments
CREATE POLICY tenant_isolation_payments ON pay.payments
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

-- webhook_provider
CREATE POLICY tenant_isolation_webhook_provider ON pay.webhook_provider
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

-- webhook_delivery
CREATE POLICY tenant_isolation_webhook_delivery ON pay.webhook_delivery
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

-- provider_configs
CREATE POLICY tenant_isolation_provider_configs ON pay.provider_configs
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

-- fraud_prevention
CREATE POLICY tenant_isolation_fraud_prevention ON pay.fraud_prevention
    FOR ALL
    TO bridge_app
    USING (app_id = current_setting('bridge.current_app_id', true)::uuid)
    WITH CHECK (app_id = current_setting('bridge.current_app_id', true)::uuid);

-- ============================================================================
-- 3. Bootstrap SELECT policies (pre-tenant-context lookups)
-- ============================================================================
-- Some tables must be readable before bridge.current_app_id is set:
--   - api_keys:         auth middleware resolves API key → app_id
--   - provider_configs: provider config lookup during early request processing
--   - webhook_provider: idempotency checks during webhook ingress
--   - webhook_delivery: scheduler/retry logic before tenant context is established
-- These are SELECT-only; write paths remain guarded by the FOR ALL policies above.

DO $$
DECLARE
    current_table_owner text;
BEGIN
    -- api_keys bootstrap SELECT
    SELECT pg_catalog.pg_get_userbyid(c.relowner)
    INTO current_table_owner
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'pay'
      AND c.relname = 'api_keys'
      AND c.relkind = 'r';

    IF current_table_owner IS NOT NULL AND current_table_owner = current_user THEN
        IF NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_policies p
            WHERE p.schemaname = 'pay' AND p.tablename = 'api_keys'
              AND p.policyname = 'tenant_isolation_api_keys_bootstrap_select'
        ) THEN
            EXECUTE $policy$
                CREATE POLICY tenant_isolation_api_keys_bootstrap_select ON pay.api_keys
                    FOR SELECT
                    TO bridge_app
                    USING (
                        current_setting('bridge.current_app_id', true) IS NULL
                        OR app_id = current_setting('bridge.current_app_id', true)::uuid
                    )
            $policy$;
        END IF;
        EXECUTE $comment$
            COMMENT ON POLICY tenant_isolation_api_keys_bootstrap_select ON pay.api_keys IS
                'RLS bootstrap: allow SELECT on api_keys before bridge.current_app_id is set; write policies remain strict.'
        $comment$;
    END IF;

    -- provider_configs bootstrap SELECT
    SELECT pg_catalog.pg_get_userbyid(c.relowner)
    INTO current_table_owner
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'pay'
      AND c.relname = 'provider_configs'
      AND c.relkind = 'r';

    IF current_table_owner IS NOT NULL AND current_table_owner = current_user THEN
        IF NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_policies p
            WHERE p.schemaname = 'pay' AND p.tablename = 'provider_configs'
              AND p.policyname = 'tenant_isolation_provider_configs_bootstrap_select'
        ) THEN
            EXECUTE $policy$
                CREATE POLICY tenant_isolation_provider_configs_bootstrap_select ON pay.provider_configs
                    FOR SELECT
                    TO bridge_app
                    USING (
                        current_setting('bridge.current_app_id', true) IS NULL
                        OR app_id = current_setting('bridge.current_app_id', true)::uuid
                    )
            $policy$;
        END IF;
        EXECUTE $comment$
            COMMENT ON POLICY tenant_isolation_provider_configs_bootstrap_select ON pay.provider_configs IS
                'RLS bootstrap: allow SELECT on provider_configs before bridge.current_app_id is set; write policies remain strict.'
        $comment$;
    END IF;

    -- webhook_provider bootstrap SELECT
    SELECT pg_catalog.pg_get_userbyid(c.relowner)
    INTO current_table_owner
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'pay'
      AND c.relname = 'webhook_provider'
      AND c.relkind = 'r';

    IF current_table_owner IS NOT NULL AND current_table_owner = current_user THEN
        IF NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_policies p
            WHERE p.schemaname = 'pay' AND p.tablename = 'webhook_provider'
              AND p.policyname = 'tenant_isolation_webhook_provider_bootstrap_select'
        ) THEN
            EXECUTE $policy$
                CREATE POLICY tenant_isolation_webhook_provider_bootstrap_select ON pay.webhook_provider
                    FOR SELECT
                    TO bridge_app
                    USING (
                        current_setting('bridge.current_app_id', true) IS NULL
                        OR app_id = current_setting('bridge.current_app_id', true)::uuid
                    )
            $policy$;
        END IF;
        EXECUTE $comment$
            COMMENT ON POLICY tenant_isolation_webhook_provider_bootstrap_select ON pay.webhook_provider IS
                'RLS bootstrap: allow SELECT on webhook_provider before bridge.current_app_id is set; write policies remain strict.'
        $comment$;
    END IF;

    -- webhook_delivery bootstrap SELECT
    SELECT pg_catalog.pg_get_userbyid(c.relowner)
    INTO current_table_owner
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'pay'
      AND c.relname = 'webhook_delivery'
      AND c.relkind = 'r';

    IF current_table_owner IS NOT NULL AND current_table_owner = current_user THEN
        IF NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_policies p
            WHERE p.schemaname = 'pay' AND p.tablename = 'webhook_delivery'
              AND p.policyname = 'tenant_isolation_webhook_delivery_bootstrap_select'
        ) THEN
            EXECUTE $policy$
                CREATE POLICY tenant_isolation_webhook_delivery_bootstrap_select ON pay.webhook_delivery
                    FOR SELECT
                    TO bridge_app
                    USING (
                        current_setting('bridge.current_app_id', true) IS NULL
                        OR app_id = current_setting('bridge.current_app_id', true)::uuid
                    )
            $policy$;
        END IF;
        EXECUTE $comment$
            COMMENT ON POLICY tenant_isolation_webhook_delivery_bootstrap_select ON pay.webhook_delivery IS
                'RLS bootstrap: allow SELECT on webhook_delivery before bridge.current_app_id is set; write policies remain strict.'
        $comment$;
    END IF;
END
$$;

-- ============================================================================
-- 4. Comments (tenant isolation policies)
-- ============================================================================

COMMENT ON POLICY tenant_isolation_api_keys ON pay.api_keys IS
    'RLS: restrict api_keys access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_subscriptions ON pay.subscriptions IS
    'RLS: restrict subscriptions access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_payments ON pay.payments IS
    'RLS: restrict payments access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_webhook_provider ON pay.webhook_provider IS
    'RLS: restrict webhook_provider access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_webhook_delivery ON pay.webhook_delivery IS
    'RLS: restrict webhook_delivery access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_provider_configs ON pay.provider_configs IS
    'RLS: restrict provider_configs access to current app_id session variable.';
COMMENT ON POLICY tenant_isolation_fraud_prevention ON pay.fraud_prevention IS
    'RLS: restrict fraud_prevention access to current app_id session variable.';
