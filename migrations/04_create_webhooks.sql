SET search_path TO pay, public;

-- Bridge: provider webhook ingress log and app callback delivery state.

CREATE TABLE webhook_provider (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,

    provider TEXT NOT NULL,
    provider_webhook_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    subscription_id TEXT,
    purchase_token TEXT,

    payload JSONB NOT NULL,

    processed BOOLEAN DEFAULT false,

    timestamp_epoch_ms BIGINT,
    suppressed BOOLEAN DEFAULT false,
    suppressed_reason TEXT,

    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE UNIQUE INDEX idx_webhook_provider_app_id ON webhook_provider(app_id, provider, provider_webhook_id);
CREATE UNIQUE INDEX idx_webhook_provider_id_app_id ON webhook_provider(id, app_id);

CREATE TABLE webhook_delivery (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    webhook_provider_id UUID NOT NULL,

    forward_attempts INT NOT NULL DEFAULT 0,
    forwarded BOOLEAN DEFAULT false,
    forwarded_at TIMESTAMPTZ,
    dead_lettered BOOLEAN NOT NULL DEFAULT false,
    dead_lettered_at TIMESTAMPTZ,
    dead_letter_reason TEXT,

    last_http_status INT,
    last_error TEXT,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    CONSTRAINT uq_webhook_delivery_provider UNIQUE (webhook_provider_id),
    CONSTRAINT chk_webhook_delivery_forward_attempts_non_negative CHECK (forward_attempts >= 0),
    CONSTRAINT fk_webhook_delivery_provider_and_app
        FOREIGN KEY (webhook_provider_id, app_id) REFERENCES webhook_provider(id, app_id) ON DELETE CASCADE
);

CREATE INDEX idx_webhook_delivery_app_id ON webhook_delivery(app_id);
CREATE INDEX idx_webhook_delivery_provider_id ON webhook_delivery(webhook_provider_id);
CREATE INDEX idx_webhook_delivery_forwarded ON webhook_delivery(app_id, forwarded) WHERE forwarded = false;
CREATE INDEX idx_webhook_delivery_attempts ON webhook_delivery(app_id, forward_attempts) WHERE forward_attempts > 0;
CREATE INDEX idx_webhook_delivery_created_at ON webhook_delivery(created_at);
CREATE INDEX idx_webhook_delivery_dead_lettered
    ON webhook_delivery(app_id, dead_lettered)
    WHERE dead_lettered = true;

COMMENT ON TABLE webhook_provider IS 'Incoming webhooks from payment providers. Deduplication, idempotent processing, and audit trail.';
COMMENT ON COLUMN webhook_provider.provider_webhook_id IS 'Unique event ID from the provider. Used for deduplication.';
COMMENT ON COLUMN webhook_provider.payload IS 'Full raw JSON payload from the provider for audit and debugging.';
COMMENT ON COLUMN webhook_provider.timestamp_epoch_ms IS 'Provider event timestamp in epoch milliseconds. Used for ordering, deduplication, and stale event detection.';
COMMENT ON COLUMN webhook_provider.suppressed_reason IS 'Why this webhook was suppressed: stale_ingress, superseded_before_forward, or duplicate.';

COMMENT ON TABLE webhook_delivery IS 'Tracks webhook delivery to apps. Supports retry logic and audit trail.';
COMMENT ON COLUMN webhook_delivery.forward_attempts IS 'Number of forward attempts. Max 3 before dead-lettering.';
COMMENT ON COLUMN webhook_delivery.dead_lettered IS 'Terminal state for webhook deliveries that exhausted retry attempts.';
COMMENT ON COLUMN webhook_delivery.dead_lettered_at IS 'Timestamp when the webhook delivery was marked dead-lettered.';
COMMENT ON COLUMN webhook_delivery.dead_letter_reason IS 'Reason recorded when the webhook delivery became dead-lettered.';
COMMENT ON COLUMN webhook_delivery.last_http_status IS 'HTTP status from last delivery attempt.';
COMMENT ON COLUMN webhook_delivery.last_error IS 'Error message from last delivery attempt.';
