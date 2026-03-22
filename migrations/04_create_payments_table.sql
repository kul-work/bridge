SET search_path TO pay, public;

-- Bridge: Payments
-- Payment records. product_id is opaque (Bridge does not interpret it).

CREATE TABLE payments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    external_user_id TEXT NOT NULL,             -- opaque, from app
    provider TEXT NOT NULL,
    provider_transaction_id TEXT NOT NULL,
    
    subscription_id TEXT,                       -- optional reference
    product_id TEXT,                            -- opaque, from app (e.g. 'premium_monthly', 'otp_lifetime')
    
    amount_cents INT NOT NULL,
    currency TEXT DEFAULT 'USD',
    
    status TEXT NOT NULL,                       -- 'pending', 'success', 'failed', 'refunded'
    
    acknowledged_at TIMESTAMPTZ,
    webhook_received_at TIMESTAMPTZ,
    
    created_at TIMESTAMPTZ DEFAULT NOW(),
    
    CONSTRAINT uq_pay_app_provider_txnid UNIQUE (app_id, provider, provider_transaction_id)
);

CREATE INDEX idx_pay_app_user ON payments(app_id, external_user_id);
CREATE INDEX idx_pay_provider_txn_id ON payments(provider_transaction_id);
CREATE INDEX idx_pay_subscription_id ON payments(subscription_id);

COMMENT ON TABLE payments IS 'Payment records. external_user_id and product_id are opaque; Bridge does not interpret them.';
COMMENT ON COLUMN payments.external_user_id IS 'Opaque user ID from the app.';
COMMENT ON COLUMN payments.product_id IS 'Opaque product identifier from the app (e.g., "premium_monthly", "otp_lifetime").';
COMMENT ON COLUMN payments.provider_transaction_id IS 'Unique transaction ID from the payment provider.';
COMMENT ON COLUMN payments.status IS 'Payment status: pending, success, failed, refunded.';
