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
CREATE INDEX idx_agent_credits_app_id ON agent_credits(app_id);

COMMENT ON TABLE agent_credits IS 'Agent credit balance for HTTP 402 micropayments. external_user_id typically an email address.';
COMMENT ON COLUMN agent_credits.external_user_id IS 'Opaque agent identifier from the app (typically email).';
COMMENT ON COLUMN agent_credits.balance_cents IS 'Current balance in cents.';
COMMENT ON COLUMN agent_credits.lifetime_spent_cents IS 'Cumulative amount spent (for analytics/billing).';

-- Trigger for updated_at
CREATE OR REPLACE FUNCTION update_agent_credits_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_update_agent_credits_updated_at
BEFORE UPDATE ON agent_credits
FOR EACH ROW
EXECUTE FUNCTION update_agent_credits_updated_at();

-- Agent transaction audit log
CREATE TABLE agent_transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_credits_id UUID NOT NULL REFERENCES agent_credits(id) ON DELETE CASCADE,
    
    transaction_type TEXT NOT NULL,             -- 'topup', 'charge', 'refund', 'adjustment'
    amount_cents INT NOT NULL,                  -- positive = topup, negative = deduction
    
    -- Reference to payment provider
    provider_charge_id TEXT,                    -- Coinbase Commerce charge ID, etc.
    
    -- Reference to agent request (if charge)
    endpoint TEXT,                              -- 'story', 'joke', etc.
    agent_token_id UUID,                        -- reference to agent_payment_tokens
    
    status TEXT NOT NULL DEFAULT 'pending',     -- 'pending', 'completed', 'failed', 'reversed'
    
    -- Reason for transaction
    description TEXT,
    
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Idempotency: prevent double-crediting same charge
CREATE UNIQUE INDEX idx_agent_transactions_charge_id
    ON agent_transactions(provider_charge_id)
    WHERE provider_charge_id IS NOT NULL AND status = 'completed';

CREATE INDEX idx_agent_transactions_credits_id ON agent_transactions(agent_credits_id);
CREATE INDEX idx_agent_transactions_status ON agent_transactions(status) WHERE status IN ('pending', 'failed');
CREATE INDEX idx_agent_transactions_type ON agent_transactions(transaction_type);
CREATE INDEX idx_agent_transactions_created_at ON agent_transactions(created_at);

COMMENT ON TABLE agent_transactions IS 'Audit log for agent credit transactions (top-ups, charges, refunds).';
COMMENT ON COLUMN agent_transactions.provider_charge_id IS 'External charge ID (e.g., Coinbase Commerce ID) for idempotency.';
COMMENT ON COLUMN agent_transactions.agent_token_id IS 'Reference to agent_payment_tokens for scoped 402 flows.';

-- Scoped payment tokens for agent 402 flow
-- One-time use, short-lived, tied to specific agent + endpoint + amount
CREATE TABLE agent_payment_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_credits_id UUID NOT NULL REFERENCES agent_credits(id) ON DELETE CASCADE,
    
    endpoint TEXT NOT NULL,                     -- 'story', 'joke', etc
    amount_cents INT NOT NULL,                  -- scoped amount
    nonce TEXT NOT NULL,                        -- random string for uniqueness
    
    used BOOLEAN NOT NULL DEFAULT FALSE,
    used_at TIMESTAMPTZ,
    
    created_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

-- Index for quick lookup by token ID + agent
CREATE INDEX idx_agent_payment_tokens_id_agent ON agent_payment_tokens(id, agent_credits_id);

-- Index for cleanup of expired tokens
CREATE INDEX idx_agent_payment_tokens_expires_at ON agent_payment_tokens(expires_at);

-- Prevent duplicate tokens (same agent + endpoint + nonce while unused)
CREATE UNIQUE INDEX idx_agent_payment_tokens_unique_request
    ON agent_payment_tokens(agent_credits_id, endpoint, nonce)
    WHERE used = FALSE;

CREATE INDEX idx_agent_payment_tokens_used ON agent_payment_tokens(used) WHERE used = FALSE;

COMMENT ON TABLE agent_payment_tokens IS 'Scoped, one-time-use tokens for agent HTTP 402 micropayments. 10-minute validity window.';
COMMENT ON COLUMN agent_payment_tokens.endpoint IS 'Which endpoint this token is for (e.g., "story", "joke"). Scoped for security.';
COMMENT ON COLUMN agent_payment_tokens.nonce IS 'Random string to prevent token prediction and collision detection.';
COMMENT ON COLUMN agent_payment_tokens.expires_at IS 'Token validity window (typically 10 minutes from creation).';
