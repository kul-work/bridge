SET search_path TO pay, public;

ALTER TABLE subscriptions
    ADD COLUMN IF NOT EXISTS payment_failure_notification BOOLEAN NOT NULL DEFAULT false;

COMMENT ON COLUMN subscriptions.payment_failure_notification IS
    'Marks subscriptions that have had a payment failure webhook for admin/app follow-up.';
