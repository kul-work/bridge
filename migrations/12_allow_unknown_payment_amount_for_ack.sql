SET search_path TO pay, public;

ALTER TABLE payments ALTER COLUMN amount_cents DROP NOT NULL;
