SET search_path TO pay, public;

-- Bridge: Fraud Prevention & Purchase Token Tracking
-- Prevents re-enrollment, tracks obfuscated IDs for GDPR compliance, 90-day post-deletion retention.

CREATE TABLE fraud_prevention (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    -- Obfuscated IDs from providers (e.g., Google Play obfuscated_account_id)
    provider TEXT NOT NULL,
    provider_obfuscated_account_id TEXT,
    provider_obfuscated_profile_id TEXT,
    
    -- Associated subscription / user (before anonymization)
    external_user_id TEXT,                      -- opaque user ID (may be anonymized as "deleted_*")
    subscription_id TEXT,
    
    -- State tracking
    is_anonymized BOOLEAN DEFAULT false,        -- true after account deletion
    anonymized_at TIMESTAMPTZ,
    
    -- Retention countdown (90 days post-deletion before final purge)
    should_purge_at TIMESTAMPTZ,
    
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_fraud_prevention_app_id ON fraud_prevention(app_id);
CREATE INDEX idx_fraud_prevention_provider_obf_account ON fraud_prevention(app_id, provider, provider_obfuscated_account_id);
CREATE INDEX idx_fraud_prevention_is_anonymized ON fraud_prevention(app_id, is_anonymized) WHERE is_anonymized = true;
CREATE INDEX idx_fraud_prevention_purge_at ON fraud_prevention(should_purge_at) WHERE should_purge_at IS NOT NULL;
CREATE INDEX idx_fraud_prevention_created_at ON fraud_prevention(created_at);

COMMENT ON TABLE fraud_prevention IS 'GDPR compliance. Tracks obfuscated IDs and enrollment history. Supports 90-day post-deletion retention.';
COMMENT ON COLUMN fraud_prevention.provider_obfuscated_account_id IS 'Opaque account ID from provider (e.g., Google Play obfuscated_account_id) - never reusable by different user.';
COMMENT ON COLUMN fraud_prevention.is_anonymized IS 'True after user account deletion. external_user_id becomes "deleted_*".';
COMMENT ON COLUMN fraud_prevention.should_purge_at IS 'Date when this record should be permanently deleted (90 days post-anonymization).';
