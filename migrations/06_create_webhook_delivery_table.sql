-- Bridge: Webhook Delivery
-- Tracks forwarded callbacks to apps. Supports retry logic and audit.

CREATE TABLE webhook_delivery (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    webhook_log_id UUID NOT NULL REFERENCES webhook_log(id) ON DELETE CASCADE,
    
    -- Delivery attempt tracking
    forward_attempts INT NOT NULL DEFAULT 0,
    forwarded BOOLEAN DEFAULT false,
    forwarded_at TIMESTAMPTZ,
    
    -- HTTP response details
    last_http_status INT,
    last_error TEXT,
    
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_webhook_delivery_app_id ON webhook_delivery(app_id);
CREATE INDEX idx_webhook_delivery_log_id ON webhook_delivery(webhook_log_id);
CREATE INDEX idx_webhook_delivery_forwarded ON webhook_delivery(app_id, forwarded) WHERE forwarded = false;
CREATE INDEX idx_webhook_delivery_attempts ON webhook_delivery(app_id, forward_attempts) WHERE forward_attempts > 0;
CREATE INDEX idx_webhook_delivery_created_at ON webhook_delivery(created_at);

COMMENT ON TABLE webhook_delivery IS 'Tracks webhook delivery to apps. Supports retry logic (up to 3 strikes) and audit trail.';
COMMENT ON COLUMN webhook_delivery.forward_attempts IS 'Number of forward attempts. Max 3 before dead-lettering.';
COMMENT ON COLUMN webhook_delivery.last_http_status IS 'HTTP status from last delivery attempt (if any).';
COMMENT ON COLUMN webhook_delivery.last_error IS 'Error message from last delivery attempt (if any).';
