SET search_path TO pay, public;

UPDATE subscriptions
SET status = 'trial'
WHERE status = 'trialing';

UPDATE subscriptions
SET status = 'cancelled'
WHERE status = 'scheduled_cancel';

UPDATE subscriptions
SET status = 'expired'
WHERE status = 'inactive';

ALTER TABLE subscriptions
    ADD CONSTRAINT subscriptions_status_check
    CHECK (status IN (
        'pending',
        'trial',
        'active',
        'past_due',
        'on_hold',
        'paused',
        'cancelled',
        'expired',
        'revoked',
        -- Set internally by link_replacement_subscriptions_tx, never by webhook normalization.
        'replaced'
    ));
