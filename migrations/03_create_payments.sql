SET search_path TO pay, public;

-- Bridge: payment records. product_id is opaque and app-owned.

CREATE TABLE payments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,

    external_user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_transaction_id TEXT NOT NULL,
    provider_purchase_token TEXT,

    subscription_id TEXT,
    product_id TEXT,

    amount_cents BIGINT,
    currency TEXT DEFAULT 'N/A',

    status TEXT NOT NULL,
    ack_required BOOLEAN NOT NULL DEFAULT false,

    acknowledged_at TIMESTAMPTZ,
    webhook_received_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ DEFAULT NOW(),

    CONSTRAINT uq_pay_app_provider_txnid UNIQUE (app_id, provider, provider_transaction_id)
);

CREATE INDEX idx_pay_app_user ON payments(app_id, external_user_id);
CREATE INDEX idx_pay_provider_txn_id ON payments(provider_transaction_id);
CREATE INDEX idx_pay_subscription_id ON payments(subscription_id);
CREATE INDEX idx_pay_provider_purchase_token
    ON payments(app_id, provider, provider_purchase_token)
    WHERE provider_purchase_token IS NOT NULL;
CREATE INDEX idx_pay_google_ack_required
    ON payments(app_id, provider, provider_purchase_token)
    WHERE provider = 'google_play'
      AND provider_purchase_token IS NOT NULL
      AND ack_required = true
      AND acknowledged_at IS NULL;

COMMENT ON TABLE payments IS 'Payment records. external_user_id and product_id are opaque; Bridge does not interpret them.';
COMMENT ON COLUMN payments.external_user_id IS 'Opaque user ID from the app.';
COMMENT ON COLUMN payments.product_id IS 'Opaque product identifier from the app.';
COMMENT ON COLUMN payments.provider_transaction_id IS 'Provider economic transaction/order id.';
COMMENT ON COLUMN payments.provider_purchase_token IS 'Provider lifecycle/API token used for acknowledgement or purchase lookup; not the economic transaction id.';
COMMENT ON COLUMN payments.ack_required IS 'True when this payment row represents a provider purchase that requires acknowledgement.';
COMMENT ON COLUMN payments.status IS 'Payment status: pending, success, failed, refunded.';
