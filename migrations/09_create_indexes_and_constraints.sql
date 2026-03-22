-- Bridge: Additional Indexes and Constraints
-- Performance optimization for common queries

-- Subscriptions: Event-time ordering for deduplication & ordering
CREATE INDEX idx_subscriptions_event_ordering 
    ON subscriptions(app_id, external_user_id, subscription_id, last_event_time DESC);

-- Subscriptions: Active subscriptions (common query for reconciliation)
CREATE INDEX idx_subscriptions_active 
    ON subscriptions(app_id, status) 
    WHERE status IN ('active', 'trial', 'past_due');

-- Payments: Status tracking for reconciliation
CREATE INDEX idx_payments_pending
    ON payments(app_id, status)
    WHERE status = 'pending';

-- Payments: Provider-specific transaction queries
CREATE INDEX idx_payments_provider
    ON payments(app_id, provider, created_at DESC);

-- Webhook provider: Unprocessed webhooks for background job
CREATE INDEX idx_webhook_provider_unprocessed
    ON webhook_provider(app_id, created_at)
    WHERE processed = false;

-- Agent credits: Active agents with balance
CREATE INDEX idx_agent_credits_balance
    ON agent_credits(app_id, balance_cents)
    WHERE balance_cents > 0;

-- Agent transactions: By type and status for analytics
CREATE INDEX idx_agent_transactions_by_type_status
    ON agent_transactions(request_type, status, created_at DESC);

-- Agent payment tokens: Unused tokens for cleanup
CREATE INDEX idx_agent_payment_tokens_cleanup
    ON agent_payment_tokens(expires_at)
    WHERE used = FALSE;

-- Fraud prevention: Lookup by provider ID for restore purchases
CREATE INDEX idx_fraud_prevention_provider_lookup
    ON fraud_prevention(app_id, provider, provider_obfuscated_account_id)
    WHERE is_anonymized = false;
