SET search_path TO pay, public;

-- Durable lease used by the webhook retry worker when recovering provider
-- inbox rows that were stored but not processed before provider ACK.
ALTER TABLE webhook_provider
    ADD COLUMN recovery_claimed_at TIMESTAMPTZ;

CREATE INDEX idx_webhook_provider_recovery_claim
    ON webhook_provider(app_id, created_at, recovery_claimed_at)
    WHERE processed = false AND suppressed = false;

COMMENT ON COLUMN webhook_provider.recovery_claimed_at IS
    'When a background worker claimed this unprocessed provider webhook for inbox recovery.';
