-- Bridge: Subscriptions
-- Source of truth for subscription lifecycle. external_user_id is opaque (e.g., clerk_id from app).

CREATE TABLE subscriptions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    
    external_user_id TEXT NOT NULL,             -- opaque, from the app (e.g. clerk_id)
    subscription_id TEXT NOT NULL,
    provider TEXT NOT NULL,                     -- 'google_play', 'creem', 'apple', 'lemonsqueezy'
    
    purchase_token TEXT UNIQUE,                 -- one-token-one-owner: required for restore purchases & fraud prevention
    
    status TEXT NOT NULL DEFAULT 'pending',     -- pending, active, trial, past_due, cancelled, expired, revoked, paused, on_hold
    current_period_end TIMESTAMPTZ,
    auto_renewing BOOLEAN,
    payment_state INT,
    cancel_reason INT,
    provider_customer_id TEXT,
    
    -- Cancellation / Revocation
    cancellation_initiated_at TIMESTAMPTZ,
    revocation_reason TEXT,
    revoked_at TIMESTAMPTZ,
    
    -- Google Play specific (prefixed)
    google_subscription_state INT,
    google_obfuscated_account_id TEXT,
    google_obfuscated_profile_id TEXT,
    google_linked_purchase_token TEXT,
    google_previous_subscription_id TEXT,
    google_grace_period_start TIMESTAMPTZ,
    google_grace_period_end TIMESTAMPTZ,
    google_cancellation_context TEXT,
    google_cancellation_feedback TEXT,
    google_initial_committed_payments INT,
    google_remaining_committed_payments INT,
    google_pending_cancellation BOOLEAN DEFAULT false,
    google_pending_cancellation_at TIMESTAMPTZ,
    google_deferred_until TIMESTAMPTZ,
    google_prepaid_ack_deadline TIMESTAMPTZ,
    google_prepaid_allow_extend_after TIMESTAMPTZ,
    google_prepaid_linked_purchase_token TEXT,
    google_requires_price_step_up_consent BOOLEAN DEFAULT false,
    google_price_step_up_consent_status TEXT,
    google_price_step_up_consent_deadline TIMESTAMPTZ,
    google_new_price_cents INT,
    google_pause_scheduled_at TIMESTAMPTZ,
    google_pause_scheduled_reason TEXT,
    google_paused_at TIMESTAMPTZ,
    google_is_manual_resume BOOLEAN,
    
    -- Apple specific (future, prefixed)
    apple_original_transaction_id TEXT,
    apple_web_order_line_item_id TEXT,
    apple_environment TEXT,
    
    -- Concurrency
    version INT NOT NULL DEFAULT 1,
    last_event_time BIGINT NOT NULL DEFAULT 0, -- epoch milliseconds, derived from webhook callback timestamp for event ordering and deduplication
    
    -- Timestamps
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    
    -- Constraints
    CONSTRAINT uq_sub_app_user_sub_provider UNIQUE (app_id, external_user_id, subscription_id, provider)
);

CREATE INDEX idx_subs_app_id ON subscriptions(app_id);
CREATE INDEX idx_subs_app_user ON subscriptions(app_id, external_user_id);
CREATE INDEX idx_subs_status ON subscriptions(app_id, status) WHERE status = 'active';
CREATE INDEX idx_subs_provider_status ON subscriptions(app_id, provider, status);
CREATE INDEX idx_subs_event_time ON subscriptions(app_id, external_user_id, subscription_id, last_event_time);
CREATE INDEX idx_subs_purchase_token ON subscriptions(purchase_token) WHERE purchase_token IS NOT NULL;
CREATE INDEX idx_subs_period_end ON subscriptions(app_id, current_period_end) WHERE current_period_end IS NOT NULL;

COMMENT ON TABLE subscriptions IS 'Source of truth for subscription lifecycle. external_user_id is opaque; Bridge does not interpret it.';
COMMENT ON COLUMN subscriptions.external_user_id IS 'Opaque user ID from the app (e.g., Clerk user ID).';
COMMENT ON COLUMN subscriptions.purchase_token IS 'One-token-one-owner for fraud prevention and restore purchases.';
COMMENT ON COLUMN subscriptions.last_event_time IS 'Epoch milliseconds from provider event timestamp. Used for ordering, deduplication, and stale event detection.';
COMMENT ON COLUMN subscriptions.version IS 'Optimistic locking for concurrent updates.';
COMMENT ON COLUMN subscriptions.google_subscription_state IS 'Raw subscription_state from Google API v3 (0-6 enum).';
