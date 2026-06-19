SET search_path TO pay, public;

-- Runtime cleanup workers run without an app context. Keep direct table grants narrow
-- and make only these retention functions privileged.
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

REVOKE ALL ON FUNCTION pay.cleanup_old_webhook_provider() FROM PUBLIC;
REVOKE ALL ON FUNCTION pay.cleanup_purged_fraud_prevention() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() TO bridge_app, bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() TO bridge_app, bridge_admin;

COMMENT ON FUNCTION pay.cleanup_old_webhook_provider() IS 'Delete webhook_provider records older than 90 days. SECURITY DEFINER so the runtime worker does not need direct DELETE grants.';
COMMENT ON FUNCTION pay.cleanup_purged_fraud_prevention() IS 'Delete fraud_prevention records past should_purge_at. SECURITY DEFINER so the runtime worker does not need direct DELETE grants.';