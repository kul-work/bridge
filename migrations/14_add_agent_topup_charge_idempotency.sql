SET search_path TO pay, public;

-- Enforce idempotency for external charge-driven topups.
CREATE UNIQUE INDEX uq_agent_transactions_topup_charge
    ON agent_transactions(app_id, charge_id)
    WHERE request_type = 'topup' AND charge_id IS NOT NULL;
