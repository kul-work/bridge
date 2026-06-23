SET search_path TO pay, public;

CREATE OR REPLACE FUNCTION pay.count_enabled_provider_configs_bootstrap()
RETURNS BIGINT
LANGUAGE sql
SECURITY DEFINER
SET search_path = pay, pg_temp
STABLE
AS $$
    SELECT COUNT(*)
    FROM pay.provider_configs
    WHERE enabled = true
$$;

REVOKE ALL ON FUNCTION pay.count_enabled_provider_configs_bootstrap() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pay.count_enabled_provider_configs_bootstrap() TO bridge_app;
