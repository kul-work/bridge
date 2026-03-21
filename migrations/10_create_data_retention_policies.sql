-- Bridge: Data Retention Policies
-- Enforces GDPR/CCPA compliance and prevents unchecked database bloat

-- Cleanup function for webhook_log (90-day retention)
CREATE OR REPLACE FUNCTION cleanup_old_webhook_logs()
RETURNS void AS $$
BEGIN
    DELETE FROM webhook_log
    WHERE created_at < NOW() - INTERVAL '90 days';
END;
$$ LANGUAGE plpgsql;

-- Cleanup function for fraud_prevention (90 days post-deletion)
CREATE OR REPLACE FUNCTION cleanup_purged_fraud_prevention()
RETURNS void AS $$
BEGIN
    DELETE FROM fraud_prevention
    WHERE should_purge_at IS NOT NULL
    AND should_purge_at < NOW();
END;
$$ LANGUAGE plpgsql;

-- Cleanup function for expired agent payment tokens
CREATE OR REPLACE FUNCTION cleanup_expired_agent_tokens()
RETURNS void AS $$
BEGIN
    DELETE FROM agent_payment_tokens
    WHERE expires_at < NOW()
    AND used = FALSE;
END;
$$ LANGUAGE plpgsql;

-- View for data retention status
CREATE OR REPLACE VIEW v_data_retention_stats AS
SELECT
    'webhook_log' AS table_name,
    COUNT(*) AS record_count,
    MIN(created_at) AS oldest_record,
    (NOW() - MIN(created_at))::interval AS age_of_oldest,
    (SELECT COUNT(*) FROM webhook_log WHERE created_at < NOW() - INTERVAL '90 days') AS records_pending_cleanup
FROM webhook_log

UNION ALL

SELECT
    'fraud_prevention',
    COUNT(*),
    MIN(created_at),
    (NOW() - MIN(created_at))::interval,
    (SELECT COUNT(*) FROM fraud_prevention WHERE should_purge_at IS NOT NULL AND should_purge_at < NOW())
FROM fraud_prevention

UNION ALL

SELECT
    'agent_payment_tokens',
    COUNT(*),
    MIN(created_at),
    (NOW() - MIN(created_at))::interval,
    (SELECT COUNT(*) FROM agent_payment_tokens WHERE expires_at < NOW() AND used = FALSE)
FROM agent_payment_tokens;

COMMENT ON VIEW v_data_retention_stats IS 'Shows data retention status and cleanup readiness. Useful for monitoring compliance.';

-- Comment on retention policies
COMMENT ON FUNCTION cleanup_old_webhook_logs() IS 'Delete webhook_log records older than 90 days. Call from background job daily.';
COMMENT ON FUNCTION cleanup_purged_fraud_prevention() IS 'Delete fraud_prevention records past their should_purge_at deadline. Call from background job daily.';
COMMENT ON FUNCTION cleanup_expired_agent_tokens() IS 'Delete expired agent payment tokens (past expires_at, unused). Call from background job hourly.';
