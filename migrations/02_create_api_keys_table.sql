SET search_path TO pay, public;

-- Bridge: API Keys
-- App-to-Bridge authentication. Each app gets one or more API keys.

CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    key_prefix TEXT NOT NULL,                   -- first 8 chars for identification (e.g. 'sk_hiha_')
    key_hash TEXT NOT NULL,                     -- bcrypt/argon2 hash of full key
    label TEXT,                                 -- 'production', 'staging', 'ci'
    permissions TEXT[] DEFAULT '{}',            -- future: granular permissions
    
    last_used_at TIMESTAMPTZ,
    enabled BOOLEAN DEFAULT true,
    
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_api_keys_app_id ON api_keys(app_id);
CREATE INDEX idx_api_keys_prefix ON api_keys(key_prefix);
CREATE INDEX idx_api_keys_enabled ON api_keys(enabled);

COMMENT ON TABLE api_keys IS 'API keys for app-to-Bridge authentication. Keys are hashed; full key never stored.';
COMMENT ON COLUMN api_keys.key_prefix IS 'First 8 chars for identification (e.g., "sk_hiha_"). Helps humans identify which key without exposing full secret.';
COMMENT ON COLUMN api_keys.key_hash IS 'Hashed full key (bcrypt/argon2). Used for authentication verification.';
