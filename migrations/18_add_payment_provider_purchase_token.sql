SET search_path TO pay, public;

ALTER TABLE payments
    ADD COLUMN provider_purchase_token TEXT,
    ADD COLUMN ack_required BOOLEAN NOT NULL DEFAULT false;

UPDATE payments
SET provider_purchase_token = provider_transaction_id,
    ack_required = true
WHERE provider = 'google_play'
  AND provider_purchase_token IS NULL
  AND provider_transaction_id NOT LIKE 'GPA.%';

CREATE INDEX idx_pay_provider_purchase_token
    ON payments(app_id, provider, provider_purchase_token)
    WHERE provider_purchase_token IS NOT NULL;

CREATE INDEX idx_pay_google_ack_required
    ON payments(app_id, provider, provider_purchase_token)
    WHERE provider = 'google_play'
      AND provider_purchase_token IS NOT NULL
      AND ack_required = true
      AND acknowledged_at IS NULL;

COMMENT ON COLUMN payments.provider_purchase_token IS 'Provider lifecycle/API token used for acknowledgement or purchase lookup; not the economic transaction id.';
COMMENT ON COLUMN payments.ack_required IS 'True when this payment row represents a provider purchase that requires acknowledgement.';
