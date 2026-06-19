SET search_path TO pay, public;

-- Bridge: apps registry and app-to-Bridge API keys.

CREATE TABLE apps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug TEXT UNIQUE NOT NULL,
    display_name TEXT NOT NULL,

    -- Mobile store identifiers (reference only; actual config in provider_configs).
    google_package_name TEXT,
    apple_bundle_id TEXT,

    -- App callback.
    webhook_callback_url TEXT NOT NULL,
    webhook_callback_secret TEXT NOT NULL,

    -- Security and rate limiting.
    webhook_ingress_token UUID UNIQUE NOT NULL DEFAULT gen_random_uuid(),
    api_rate_limit_per_minute INT DEFAULT 120,
    api_rate_limit_rules JSONB,

    -- Settings.
    app_url TEXT,
    enabled BOOLEAN DEFAULT true,
    notes TEXT,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_apps_slug ON apps(slug);
CREATE INDEX idx_apps_enabled ON apps(enabled);
CREATE INDEX idx_apps_webhook_ingress_token ON apps(webhook_ingress_token);

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,

    key_prefix TEXT NOT NULL,
    key_hash TEXT NOT NULL,
    label TEXT,
    permissions TEXT[] DEFAULT '{}',

    last_used_at TIMESTAMPTZ,
    enabled BOOLEAN DEFAULT true,

    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_api_keys_app_id ON api_keys(app_id);
CREATE INDEX idx_api_keys_prefix ON api_keys(key_prefix);
CREATE INDEX idx_api_keys_enabled ON api_keys(enabled);

COMMENT ON TABLE apps IS 'Registered applications in Bridge. Provider credentials are stored separately in the provider_configs table.';
COMMENT ON COLUMN apps.webhook_ingress_token IS 'Auto-generated obfuscated token for webhook URLs. Provider signature verification is still required.';
COMMENT ON COLUMN apps.api_rate_limit_rules IS 'JSONB: per-endpoint overrides, e.g. {"checkout": 20, "subscription_queries": 100}';
COMMENT ON COLUMN apps.notes IS 'Internal notes about the app.';

COMMENT ON TABLE api_keys IS 'API keys for app-to-Bridge authentication. Keys are hashed; full key never stored.';
COMMENT ON COLUMN api_keys.key_prefix IS 'First 8 chars for identification. Helps humans identify which key without exposing full secret.';
COMMENT ON COLUMN api_keys.key_hash IS 'Hashed full key. Used for authentication verification.';
