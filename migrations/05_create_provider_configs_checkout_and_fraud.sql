SET search_path TO pay, public;

-- Bridge support tables: provider configs, checkout idempotency, and fraud prevention.

CREATE TABLE provider_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,

    provider TEXT NOT NULL,
    config JSONB NOT NULL,

    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    CONSTRAINT uq_provider_configs_app_provider UNIQUE (app_id, provider)
);

CREATE INDEX idx_provider_configs_app_id ON provider_configs(app_id);
CREATE INDEX idx_provider_configs_provider ON provider_configs(app_id, provider);
CREATE INDEX idx_provider_configs_enabled ON provider_configs(app_id, enabled) WHERE enabled = true;

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

CREATE TABLE fraud_prevention (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,

    provider TEXT NOT NULL,
    provider_obfuscated_account_id TEXT,
    provider_obfuscated_profile_id TEXT,

    external_user_id TEXT,
    subscription_id TEXT,

    is_anonymized BOOLEAN DEFAULT false,
    anonymized_at TIMESTAMPTZ,
    should_purge_at TIMESTAMPTZ,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    CONSTRAINT unique_app_provider_obfuscated_account
        UNIQUE (app_id, provider, provider_obfuscated_account_id)
);

CREATE INDEX idx_fraud_prevention_app_id ON fraud_prevention(app_id);
CREATE INDEX idx_fraud_prevention_provider_obf_account ON fraud_prevention(app_id, provider, provider_obfuscated_account_id);
CREATE INDEX idx_fraud_prevention_is_anonymized ON fraud_prevention(app_id, is_anonymized) WHERE is_anonymized = true;
CREATE INDEX idx_fraud_prevention_purge_at ON fraud_prevention(should_purge_at) WHERE should_purge_at IS NOT NULL;
CREATE INDEX idx_fraud_prevention_created_at ON fraud_prevention(created_at);

COMMENT ON TABLE provider_configs IS 'Per-app, per-provider configuration. Decouples provider setup from apps table schema.';
COMMENT ON COLUMN provider_configs.enabled IS 'Controls whether this provider config is active for this app.';
COMMENT ON COLUMN provider_configs.provider IS 'Payment provider identifier: google_play, creem, apple, etc.';
COMMENT ON COLUMN provider_configs.config IS 'Provider-specific config as JSON.';

COMMENT ON TABLE checkout_idempotency IS 'Idempotency cache for checkout creation requests.';
COMMENT ON COLUMN checkout_idempotency.idempotency_key IS 'Client-provided idempotency key scoped to app_id.';

COMMENT ON TABLE fraud_prevention IS 'GDPR compliance. Tracks obfuscated IDs and enrollment history. Supports 90-day post-deletion retention.';
COMMENT ON COLUMN fraud_prevention.provider_obfuscated_account_id IS 'Opaque account ID from provider, such as Google Play obfuscated_account_id.';
COMMENT ON COLUMN fraud_prevention.is_anonymized IS 'True after user account deletion. external_user_id becomes deleted_*.';
COMMENT ON COLUMN fraud_prevention.should_purge_at IS 'Date when this record should be permanently deleted.';
