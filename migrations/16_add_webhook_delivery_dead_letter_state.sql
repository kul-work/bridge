SET search_path TO pay, public;

-- Bridge: explicit terminal state for failed webhook callbacks

ALTER TABLE webhook_delivery
    ADD COLUMN dead_lettered BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN dead_lettered_at TIMESTAMPTZ,
    ADD COLUMN dead_letter_reason TEXT;

UPDATE webhook_delivery
SET dead_lettered = true,
    dead_lettered_at = COALESCE(updated_at, created_at, NOW()),
    dead_letter_reason = COALESCE(last_error, 'Retry limit exceeded')
WHERE forwarded = false
  AND forward_attempts >= 3
  AND dead_lettered = false;

CREATE INDEX idx_webhook_delivery_dead_lettered
    ON webhook_delivery(app_id, dead_lettered)
    WHERE dead_lettered = true;

COMMENT ON COLUMN webhook_delivery.dead_lettered IS 'Terminal state for webhook deliveries that exhausted retry attempts.';
COMMENT ON COLUMN webhook_delivery.dead_lettered_at IS 'Timestamp when the webhook delivery was marked dead-lettered.';
COMMENT ON COLUMN webhook_delivery.dead_letter_reason IS 'Reason recorded when the webhook delivery became dead-lettered.';
