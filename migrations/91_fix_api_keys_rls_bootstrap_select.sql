SET search_path TO pay, public;

-- Allow bridge_app to resolve API key -> app_id before tenant context is known.
-- This is SELECT-only; write paths remain guarded by tenant_isolation_api_keys.
DO $$
DECLARE
    current_table_owner text;
BEGIN
    SELECT pg_catalog.pg_get_userbyid(c.relowner)
    INTO current_table_owner
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = 'pay'
      AND c.relname = 'api_keys'
      AND c.relkind = 'r';

    IF current_table_owner IS NULL THEN
        RAISE NOTICE 'Skipping migration 91: pay.api_keys does not exist.';
        RETURN;
    END IF;

    IF current_table_owner <> current_user THEN
        RAISE NOTICE 'Skipping migration 91: current_user % is not owner (%) of pay.api_keys.',
            current_user, current_table_owner;
        RETURN;
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_policies p
        WHERE p.schemaname = 'pay'
          AND p.tablename = 'api_keys'
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
END
$$;
