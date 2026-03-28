SET search_path TO pay, public;

-- Allow bridge_app to resolve provider config before tenant context is set.
-- This is SELECT-only; write paths remain guarded by tenant_isolation_provider_configs.
DO $$
DECLARE
    current_table_owner text;
BEGIN
    SELECT pg_catalog.pg_get_userbyid(c.relowner)
    INTO current_table_owner
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'pay'
      AND c.relname = 'provider_configs'
      AND c.relkind = 'r';

    IF current_table_owner IS NULL THEN
        RAISE NOTICE 'Skipping migration 92: pay.provider_configs does not exist.';
        RETURN;
    END IF;

    IF current_table_owner <> current_user THEN
        RAISE NOTICE 'Skipping migration 92: current_user % is not owner (%) of pay.provider_configs.',
            current_user, current_table_owner;
        RETURN;
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_policies p
        WHERE p.schemaname = 'pay'
          AND p.tablename = 'provider_configs'
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
END
$$;
