SET search_path TO pay, public;

-- Replace the global purchase_token UNIQUE constraint with an app-scoped
-- partial unique index.
--
-- Migration 02 declared purchase_token as a column-level UNIQUE, which is
-- global across the whole table. That breaks app isolation: one app could
-- block another app from inserting the same token value (sandbox reuse,
-- staging/prod split, provider namespace collision), producing a
-- unique-constraint failure across a tenant boundary that RLS otherwise
-- isolates.
--
-- All lookups are already app-scoped (WHERE app_id = $1 AND purchase_token
-- = $2), and get_subscription_by_purchase_token relies on fetch_optional
-- (no LIMIT 1), so the enforced invariant is one-token-one-owner *within*
-- (app_id, purchase_token) -- not globally. Keeping provider out of the
-- key matches that assumption; including it would allow the same
-- (app_id, purchase_token) under two providers and make the fraud path
-- return multiple rows.
--
-- The partial filter (purchase_token IS NOT NULL) keeps the index small
-- and makes the NULL-tolerant intent explicit.

-- Drop the existing single-column UNIQUE constraint on purchase_token by
-- matching the column, not a hardcoded constraint name. PostgreSQL names a
-- column-level UNIQUE "<table>_<column>_key" by default, but resolving it
-- dynamically avoids a silent no-op (and an unfixed bug) if the name ever
-- differs.
DO $$
DECLARE
    cname text;
BEGIN
    SELECT con.conname INTO cname
    FROM pg_constraint con
    JOIN pg_attribute a
      ON a.attrelid = con.conrelid AND a.attnum = con.conkey[1]
    WHERE con.conrelid = 'pay.subscriptions'::regclass
      AND con.contype = 'u'
      AND array_length(con.conkey, 1) = 1
      AND a.attname = 'purchase_token';

    IF FOUND THEN
        EXECUTE format('ALTER TABLE pay.subscriptions DROP CONSTRAINT %I', cname);
    END IF;
END $$;

CREATE UNIQUE INDEX uq_subscriptions_app_purchase_token
    ON pay.subscriptions(app_id, purchase_token)
    WHERE purchase_token IS NOT NULL;

COMMENT ON COLUMN pay.subscriptions.purchase_token IS
    'One-token-one-owner for fraud prevention and restore purchases. Unique within (app_id, purchase_token), not globally, so app isolation holds.';
