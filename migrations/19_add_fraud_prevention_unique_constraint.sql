SET search_path TO pay, public;

-- Bridge: Add Unique Constraint to Fraud Prevention Table
-- 
-- Adds a unique constraint on (app_id, provider, provider_obfuscated_account_id)
-- to allow ON CONFLICT upserts when recording fraud prevention records.

ALTER TABLE pay.fraud_prevention
ADD CONSTRAINT unique_app_provider_obfuscated_account 
UNIQUE (app_id, provider, provider_obfuscated_account_id);
