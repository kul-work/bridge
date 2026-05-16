SET search_path TO pay, public;

-- Google Play reuses purchase tokens across renewals, so token+event is not a
-- valid webhook idempotency key. Keep provider_webhook_id as the dedup key.
DROP INDEX IF EXISTS pay.idx_webhook_provider_token_event_dedup;
