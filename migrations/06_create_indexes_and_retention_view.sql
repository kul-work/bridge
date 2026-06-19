SET search_path TO pay, public;

-- Additional operational indexes and retention monitoring view.

CREATE INDEX idx_subscriptions_event_ordering
    ON subscriptions(app_id, external_user_id, subscription_id, last_event_time DESC);

CREATE INDEX idx_subscriptions_active
    ON subscriptions(app_id, status)
    WHERE status IN ('active', 'trial', 'past_due');

CREATE INDEX idx_payments_pending
    ON payments(app_id, status)
    WHERE status = 'pending';

CREATE INDEX idx_payments_provider
    ON payments(app_id, provider, created_at DESC);

CREATE INDEX idx_webhook_provider_unprocessed
    ON webhook_provider(app_id, created_at)
    WHERE processed = false;

CREATE INDEX idx_fraud_prevention_provider_lookup
    ON fraud_prevention(app_id, provider, provider_obfuscated_account_id)
    WHERE is_anonymized = false;

CREATE OR REPLACE VIEW v_data_retention_stats AS
SELECT
    'webhook_provider' AS table_name,
    COUNT(*) AS record_count,
    MIN(created_at) AS oldest_record,
    (NOW() - MIN(created_at))::interval AS age_of_oldest,
    (SELECT COUNT(*) FROM webhook_provider WHERE created_at < NOW() - INTERVAL '90 days') AS records_pending_cleanup
FROM webhook_provider

UNION ALL

SELECT
    'fraud_prevention',
    COUNT(*),
    MIN(created_at),
    (NOW() - MIN(created_at))::interval,
    (SELECT COUNT(*) FROM fraud_prevention WHERE should_purge_at IS NOT NULL AND should_purge_at < NOW())
FROM fraud_prevention;

COMMENT ON VIEW v_data_retention_stats IS 'Shows data retention status and cleanup readiness. Useful for monitoring compliance.';
