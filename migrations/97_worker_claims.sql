SET search_path TO pay, public;

ALTER TABLE webhook_delivery
    ADD COLUMN claim_token UUID,
    ADD COLUMN claimed_by TEXT,
    ADD COLUMN claimed_until TIMESTAMPTZ,
    ADD COLUMN next_attempt_at TIMESTAMPTZ DEFAULT NOW();

CREATE INDEX idx_webhook_delivery_claimable
    ON webhook_delivery(app_id, next_attempt_at, created_at)
    WHERE forwarded = false
      AND dead_lettered = false
      AND forward_attempts < 3;

COMMENT ON COLUMN webhook_delivery.claim_token IS
    'Fencing token for the worker currently allowed to complete this delivery attempt.';
COMMENT ON COLUMN webhook_delivery.claimed_by IS
    'Diagnostic worker identifier for the active webhook delivery claim.';
COMMENT ON COLUMN webhook_delivery.claimed_until IS
    'Lease expiry for the active webhook delivery claim.';
COMMENT ON COLUMN webhook_delivery.next_attempt_at IS
    'Durable retry-not-before timestamp for failed callback delivery attempts.';

ALTER TABLE subscriptions
    ADD COLUMN scheduled_job_claim_token UUID,
    ADD COLUMN scheduled_job_claimed_by TEXT,
    ADD COLUMN scheduled_job_claimed_until TIMESTAMPTZ,
    ADD COLUMN scheduled_job_claim_kind TEXT;

CREATE INDEX idx_subscriptions_price_step_up_claimable
    ON subscriptions(app_id, google_price_step_up_consent_deadline, scheduled_job_claimed_until)
    WHERE google_requires_price_step_up_consent = true
      AND google_price_step_up_consent_deadline IS NOT NULL;

COMMENT ON COLUMN subscriptions.scheduled_job_claim_token IS
    'Fencing token for the worker currently allowed to complete a scheduler transition.';
COMMENT ON COLUMN subscriptions.scheduled_job_claimed_by IS
    'Diagnostic worker identifier for the active scheduler claim.';
COMMENT ON COLUMN subscriptions.scheduled_job_claimed_until IS
    'Lease expiry for the active scheduler claim.';
COMMENT ON COLUMN subscriptions.scheduled_job_claim_kind IS
    'Scheduler job kind currently claiming this subscription row.';

ALTER TABLE payments
    ADD COLUMN ack_claim_token UUID,
    ADD COLUMN ack_claimed_by TEXT,
    ADD COLUMN ack_claimed_until TIMESTAMPTZ;

CREATE INDEX idx_pay_google_ack_claimable
    ON payments(app_id, provider, ack_claimed_until, created_at)
    WHERE provider = 'google_play'
      AND provider_purchase_token IS NOT NULL
      AND ack_required = true
      AND acknowledged_at IS NULL;

COMMENT ON COLUMN payments.ack_claim_token IS
    'Fencing token for the worker currently allowed to complete a provider acknowledgement.';
COMMENT ON COLUMN payments.ack_claimed_by IS
    'Diagnostic worker identifier for the active provider acknowledgement claim.';
COMMENT ON COLUMN payments.ack_claimed_until IS
    'Lease expiry for the active provider acknowledgement claim.';
