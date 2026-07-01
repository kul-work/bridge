SET search_path TO pay, public;

ALTER TABLE payments ALTER COLUMN amount_cents TYPE bigint;
ALTER TABLE subscriptions ALTER COLUMN google_new_price_cents TYPE bigint;
