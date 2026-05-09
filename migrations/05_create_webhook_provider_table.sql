SET search_path TO pay, public;

-- Bridge: Provider Webhooks
-- Incoming webhooks from payment providers (Google Play, Creem, Apple, etc.).
-- Deduplication, audit trail, and forwarding status.

CREATE TABLE webhook_provider (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    provider TEXT NOT NULL,
    provider_webhook_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    subscription_id TEXT,
    purchase_token TEXT,
    
    -- Full payload (raw JSON from provider)
    payload JSONB NOT NULL,
    
    -- Processing state
    processed BOOLEAN DEFAULT false,
    
    -- Stale event suppression
    timestamp_epoch_ms BIGINT,             -- provider event time, used for high-water-mark comparison against subscriptions.last_event_time
    suppressed BOOLEAN DEFAULT false,
    suppressed_reason TEXT,                 -- 'stale_ingress', 'superseded_before_forward'
    
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_webhook_provider_app_id ON webhook_provider(app_id, provider, provider_webhook_id);
CREATE UNIQUE INDEX idx_webhook_provider_id_app_id ON webhook_provider(id, app_id);
COMMENT ON TABLE webhook_provider IS 'Incoming webhooks from payment providers (Google Play, Creem, Apple). Deduplication, idempotent processing, and audit trail. Full raw payloads stored for short-term debugging (90 days).';
COMMENT ON COLUMN webhook_provider.provider_webhook_id IS 'Unique event ID from the provider. Used for deduplication.';
COMMENT ON COLUMN webhook_provider.payload IS 'Full raw JSON payload from the provider (for audit and debugging).';
COMMENT ON COLUMN webhook_provider.timestamp_epoch_ms IS 'Provider event timestamp in epoch milliseconds. Used for ordering, deduplication, and stale event detection.';
COMMENT ON COLUMN webhook_provider.suppressed_reason IS 'Why this webhook was suppressed: stale_ingress, superseded_before_forward, or duplicate.';
