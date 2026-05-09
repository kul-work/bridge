SET search_path TO pay, public;

-- Bridge: Provider Configurations
-- Stores per-app, per-provider settings (credentials, API keys, webhooks).
-- Decouples provider management from apps table schema.

CREATE TABLE provider_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    provider TEXT NOT NULL,                      -- 'google_play', 'creem', 'apple'
    
    -- Provider-specific configuration
    config JSONB NOT NULL,                       -- credentials, endpoints, product IDs, etc.
    
    -- Audit
    enabled BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    
    CONSTRAINT uq_provider_configs_app_provider UNIQUE (app_id, provider)
);

CREATE INDEX idx_provider_configs_app_id ON provider_configs(app_id);
CREATE INDEX idx_provider_configs_provider ON provider_configs(app_id, provider);
CREATE INDEX idx_provider_configs_enabled ON provider_configs(app_id, enabled) WHERE enabled = true;

COMMENT ON TABLE provider_configs IS 'Per-app, per-provider configuration (credentials, API keys, webhooks). Decouples provider setup from apps table schema.

Examples:

  Google Play:
  {
    "package_name": "com.example.app",
    "service_account_json": "certs/play-billing-xxxxx.json",
    "verify_webhook_signature": true,
    "verify_audience": false,
    "pub_sub_audience": "https://your-domain.com/webhooks/google"
  }

  Creem:
  {
    "api_key": "creem_live_xxxxx",
    "api_url": "https://api.creem.io/v1",
    "product_id": "premium_monthly",
    "offer_id": "offer_123",
    "otp_id": "otp_456",
    "webhook_secret": "whsec_xxxxx",
    "verify_webhook_signature": true
  }

  Apple (future):
  {
    "shared_secret": "apple_shared_secret_xxxxx",
    "key_id": "apple_key_id",
    "issuer_id": "apple_issuer_id",
    "private_key": "-----BEGIN PRIVATE KEY-----..."
  }';
COMMENT ON COLUMN provider_configs.enabled IS 'Controls whether this provider config is active for this app.';
COMMENT ON COLUMN provider_configs.provider IS 'Payment provider identifier: google_play, creem, apple, etc.';
COMMENT ON COLUMN provider_configs.config IS 'Provider-specific config as JSON.';
