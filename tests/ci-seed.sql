-- Seed script for Bridge testing in CI environment
-- Run as superuser/postgres

SET search_path TO pay, public;

-- Clean up any existing apps/keys/configs (if any exist)
DELETE FROM pay.api_keys;
DELETE FROM pay.provider_configs;
DELETE FROM pay.apps;

-- High CI rate limits: keep middleware ON (API-01 asserts X-RateLimit headers)
-- but avoid 429s under the full bash suite volume.
-- Group overrides mirror src/middleware/rate_limit.rs default groups.
-- 1. Insert App A (hiha)
INSERT INTO pay.apps (
  id, slug, display_name, webhook_callback_url, webhook_callback_secret,
  webhook_ingress_token, enabled, api_rate_limit_per_minute, api_rate_limit_rules
)
VALUES (
  '43bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'hiha',
  'HiHa',
  'http://localhost:3000/webhooks/dummy',
  'dummy_secret',
  'fa0b93e3-32a1-49d4-ba2f-e421af5bf8e2'::uuid,
  true,
  100000,
  '{"checkout":100000,"verify_purchase":100000,"subscription_queries":100000,"subscription_mutations":100000,"payment_history":100000,"purchase_registration":100000,"default":100000}'::jsonb
);

-- 2. Insert App B (household)
INSERT INTO pay.apps (
  id, slug, display_name, webhook_callback_url, webhook_callback_secret,
  webhook_ingress_token, enabled, api_rate_limit_per_minute, api_rate_limit_rules
)
VALUES (
  '23bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'household',
  'HouseHold',
  'http://localhost:3000/webhooks/dummy',
  'dummy_secret',
  'ea0b93e3-32a1-49d4-ba2f-e421af5bf8e2'::uuid,
  true,
  100000,
  '{"checkout":100000,"verify_purchase":100000,"subscription_queries":100000,"subscription_mutations":100000,"payment_history":100000,"purchase_registration":100000,"default":100000}'::jsonb
);

-- 3. Insert API Key for App A (hiha)
-- Key: sk_hiha_ci_test_only_not_a_secret
INSERT INTO pay.api_keys (app_id, key_prefix, key_hash, label, enabled)
VALUES (
  '43bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'sk_hiha_',
  '$2b$12$H52M.o6FEMCBnHlTqN7kHO8wmNXoijG6821uR5iB4ENPghJmPjrMK',
  'ci-hiha-key',
  true
);

-- 4. Insert API Key for App B (household)
-- Key: sk_haho_ci_test_only_not_a_secret
INSERT INTO pay.api_keys (app_id, key_prefix, key_hash, label, enabled)
VALUES (
  '23bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'sk_haho_',
  '$2b$12$fwHYi4rNXaK5EPP0M8kGv.hz5z4A0kzexFPveCpJWb7NC8cuB/EFy',
  'ci-haho-key',
  true
);

-- 5. Insert Provider Configs for App A (hiha)
INSERT INTO pay.provider_configs (app_id, provider, config, enabled)
VALUES (
  '43bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'creem',
  '{"webhook_secret": "ci_test_webhook_secret_not_for_production", "verify_webhook_signature": true, "api_key": "ci_test_api_key_not_for_production"}'::jsonb,
  true
),
(
  '43bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'google_play',
  '{"package_name": "com.hiha.fe", "verify_audience": false, "service_account_json": "certs/ci-mock-google-sa.json", "verify_webhook_signature": false}'::jsonb,
  true
);

-- 6. Insert Provider Configs for App B (household)
INSERT INTO pay.provider_configs (app_id, provider, config, enabled)
VALUES (
  '23bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'creem',
  '{"webhook_secret": "ci_test_webhook_secret_not_for_production", "verify_webhook_signature": true, "api_key": "ci_test_api_key_not_for_production"}'::jsonb,
  true
),
(
  '23bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'google_play',
  '{"package_name": "com.example.household", "verify_audience": false, "service_account_json": "certs/ci-mock-google-sa.json", "verify_webhook_signature": false}'::jsonb,
  true
);
