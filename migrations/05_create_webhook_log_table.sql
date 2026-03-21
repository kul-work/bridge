-- Bridge: Webhook Log
-- Idempotent processing, full payload logging, deduplication, and audit trail.

CREATE TABLE webhook_log (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    webhook_token UUID NOT NULL,                -- which webhook endpoint received this
    provider TEXT NOT NULL,                     -- 'google_play', 'creem', 'apple', 'lemonsqueezy'
    provider_event_id TEXT NOT NULL,            -- provider's unique event ID
    
    -- Full payload (raw JSON from provider)
    payload JSONB NOT NULL,
    
    -- Event metadata
    timestamp_epoch_ms BIGINT NOT NULL,         -- provider event timestamp (epoch milliseconds)
    event_type TEXT,                            -- inferred event type (e.g., 'subscription.activated')
    
    -- Processing state
    processed BOOLEAN DEFAULT false,
    suppressed BOOLEAN DEFAULT false,           -- true if stale ingress or superseded before forward
    suppression_reason TEXT,                    -- 'stale_ingress', 'superseded_before_forward', 'duplicate'
    
    external_user_id TEXT,                      -- extracted from payload (for quick filtering)
    subscription_id TEXT,                       -- extracted from payload
    
    created_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- Uniqueness: one event per provider per app
    CONSTRAINT uq_webhook_provider_event UNIQUE (app_id, provider, provider_event_id)
);

CREATE INDEX idx_webhook_app_id ON webhook_log(app_id);
CREATE INDEX idx_webhook_app_user ON webhook_log(app_id, external_user_id) WHERE external_user_id IS NOT NULL;
CREATE INDEX idx_webhook_app_sub ON webhook_log(app_id, subscription_id) WHERE subscription_id IS NOT NULL;
CREATE INDEX idx_webhook_processed ON webhook_log(app_id, processed) WHERE processed = false;
CREATE INDEX idx_webhook_suppressed ON webhook_log(app_id, suppressed) WHERE suppressed = true;
CREATE INDEX idx_webhook_timestamp ON webhook_log(app_id, timestamp_epoch_ms);
CREATE INDEX idx_webhook_created_at ON webhook_log(created_at);

COMMENT ON TABLE webhook_log IS 'Webhook deduplication, idempotent processing, and audit trail. Full raw payloads stored for short-term debugging (90 days).';
COMMENT ON COLUMN webhook_log.webhook_token IS 'The obfuscated webhook_ingress_token from apps table that this webhook arrived on.';
COMMENT ON COLUMN webhook_log.provider_event_id IS 'Unique event ID from the provider. Used for deduplication.';
COMMENT ON COLUMN webhook_log.payload IS 'Full raw JSON payload from the provider (for audit and debugging).';
COMMENT ON COLUMN webhook_log.timestamp_epoch_ms IS 'Provider event timestamp in epoch milliseconds. Used for ordering, deduplication, and stale event detection.';
COMMENT ON COLUMN webhook_log.suppression_reason IS 'Why this webhook was suppressed: stale_ingress, superseded_before_forward, or duplicate.';
