SET search_path TO pay, public;

-- Bridge: Checkout idempotency cache
-- Stores normalized request fingerprint and response payload for idempotent checkout creation.

CREATE TABLE checkout_idempotency (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    response_payload JSONB NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW(),

    CONSTRAINT uq_checkout_idempotency_app_key UNIQUE (app_id, idempotency_key)
);

CREATE INDEX idx_checkout_idempotency_created_at ON checkout_idempotency(created_at);

COMMENT ON TABLE checkout_idempotency IS 'Idempotency cache for checkout creation requests.';
COMMENT ON COLUMN checkout_idempotency.idempotency_key IS 'Client-provided idempotency key scoped to app_id.';
