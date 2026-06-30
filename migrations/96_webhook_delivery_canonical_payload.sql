SET search_path TO pay, public;

ALTER TABLE webhook_delivery
    ADD COLUMN canonical_payload JSONB;

COMMENT ON COLUMN webhook_delivery.canonical_payload IS
    'Canonical callback payload captured when provider processing succeeds. Retry workers must forward this immutable payload instead of rebuilding from current state.';
