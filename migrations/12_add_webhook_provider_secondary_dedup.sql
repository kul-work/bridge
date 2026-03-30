SET search_path TO pay, public;

-- §51: Secondary webhook deduplication index
-- Catches duplicate events by (provider, purchase_token, event_type) combination.
-- Partial index: only applies when purchase_token is present.
CREATE UNIQUE INDEX IF NOT EXISTS idx_webhook_provider_token_event_dedup
ON webhook_provider(app_id, provider, purchase_token, event_type)
WHERE purchase_token IS NOT NULL;

COMMENT ON INDEX idx_webhook_provider_token_event_dedup IS '§51 secondary dedup: prevents duplicate events for the same purchase_token + event_type per provider/app.';
