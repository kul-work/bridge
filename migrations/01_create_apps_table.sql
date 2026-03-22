SET search_path TO pay, public;

-- Bridge: Apps Registry
-- Each registered application (e.g., hiha.app, future apps)
-- All sensitive credentials are encrypted at the application level using AES-GCM

CREATE TABLE apps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug TEXT UNIQUE NOT NULL,                  -- 'hiha', 'future-app'
    display_name TEXT NOT NULL,
    
    -- Mobile store identifiers (reference only; actual config in provider_configs)
    google_package_name TEXT,                   -- 'com.hiha.fe'
    apple_bundle_id TEXT,                       -- 'com.hiha.fe'
    
    -- App callback
    webhook_callback_url TEXT NOT NULL,         -- where Bridge forwards events to the app
    webhook_callback_secret TEXT NOT NULL,      -- HMAC secret for callback verification
    
    -- Security & Rate Limiting
    webhook_ingress_token UUID UNIQUE NOT NULL DEFAULT gen_random_uuid(),  -- secret path component for webhook URLs
    api_rate_limit_per_minute INT DEFAULT 120,  -- global fallback rate limit per-API-key
    api_rate_limit_rules JSONB,                 -- per-endpoint dynamic overrides (e.g., {"checkout": 20, "subscription_queries": 100})
    
    -- Settings
    app_url TEXT,                               -- app's public URL (used in checkout redirects)
    enabled BOOLEAN DEFAULT true,
    
    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_apps_slug ON apps(slug);
CREATE INDEX idx_apps_enabled ON apps(enabled);
CREATE INDEX idx_apps_webhook_ingress_token ON apps(webhook_ingress_token);

COMMENT ON TABLE apps IS 'Registered applications in Bridge. Provider credentials stored separately in provider_configs table (encrypted at application layer via AES-GCM).';
COMMENT ON COLUMN apps.webhook_ingress_token IS 'Obfuscated token for webhook URLs (not cryptographically sufficient alone; must pair with provider signature verification).';
COMMENT ON COLUMN apps.api_rate_limit_rules IS 'JSONB: per-endpoint overrides, e.g. {"checkout": 20, "subscription_queries": 100}';
