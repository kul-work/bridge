-- Bridge: Agent Credits System
-- Tracks virtual balance for automated agents. external_user_id is typically an email.
-- Completely decoupled from app's Clerk/Human users.

CREATE TABLE agent_credits (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    external_user_id TEXT NOT NULL,             -- opaque, from app (e.g., agent email)
    
    balance_cents INT NOT NULL DEFAULT 0,
    lifetime_spent_cents INT NOT NULL DEFAULT 0,
    
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    
    CONSTRAINT uq_agent_app_user UNIQUE (app_id, external_user_id)
);

CREATE INDEX idx_agent_credits_user ON agent_credits(app_id, external_user_id);

COMMENT ON TABLE agent_credits IS 'Agent credit balance for HTTP 402 micropayments. external_user_id typically an email address.';
COMMENT ON COLUMN agent_credits.external_user_id IS 'Opaque agent identifier from the app (typically email).';
COMMENT ON COLUMN agent_credits.balance_cents IS 'Current balance in cents.';
COMMENT ON COLUMN agent_credits.lifetime_spent_cents IS 'Cumulative amount spent (for analytics/billing).';

-- Agent transaction audit log
CREATE TABLE agent_transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    external_user_id TEXT NOT NULL,
    request_type TEXT NOT NULL,              -- 'topup', 'story', 'joke' (defined by app)
    amount_cents INT NOT NULL,               -- positive=topup, negative=deduction
    charge_id TEXT,                          -- Coinbase/Creem ID for topups
    
    status TEXT NOT NULL DEFAULT 'pending',  -- 'pending', 'completed', 'failed'
    
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_agent_transactions_user ON agent_transactions(app_id, external_user_id);

COMMENT ON TABLE agent_transactions IS 'Audit log for agent credit transactions (top-ups, charges, refunds).';
COMMENT ON COLUMN agent_transactions.charge_id IS 'External charge ID (e.g., Coinbase Commerce ID) for idempotency.';

-- Scoped payment tokens for agent 402 flow
-- One-time use, short-lived, tied to specific agent + endpoint + amount
CREATE TABLE agent_payment_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    external_user_id TEXT NOT NULL,
    endpoint TEXT NOT NULL,                     -- 'story', 'joke'
    amount_cents INT NOT NULL,
    nonce TEXT NOT NULL,
    
    used BOOLEAN NOT NULL DEFAULT FALSE,
    used_at TIMESTAMPTZ,
    
    created_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX idx_agent_tokens_unique_request ON agent_payment_tokens(app_id, external_user_id, endpoint, nonce);
CREATE INDEX idx_agent_tokens_user ON agent_payment_tokens(app_id, external_user_id);

COMMENT ON TABLE agent_payment_tokens IS 'Scoped, one-time-use tokens for agent HTTP 402 micropayments. 10-minute validity window.';
COMMENT ON COLUMN agent_payment_tokens.endpoint IS 'Which endpoint this token is for (e.g., "story", "joke"). Scoped for security.';
COMMENT ON COLUMN agent_payment_tokens.nonce IS 'Random string to prevent token prediction and collision detection.';
COMMENT ON COLUMN agent_payment_tokens.expires_at IS 'Token validity window (typically 10 minutes from creation).';
