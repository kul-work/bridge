-- Seed script for Bridge testing in CI environment
-- Run as superuser/postgres

SET search_path TO pay, public;

-- Clean up any existing apps/keys/configs (if any exist)
DELETE FROM pay.api_keys;
DELETE FROM pay.provider_configs;
DELETE FROM pay.apps;

-- 1. Insert App A (hiha)
INSERT INTO pay.apps (id, slug, display_name, webhook_callback_url, webhook_callback_secret, webhook_ingress_token, enabled)
VALUES (
  '43bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'hiha',
  'HiHa',
  'http://localhost:3000/webhooks/dummy',
  'dummy_secret',
  'fa0b93e3-32a1-49d4-ba2f-e421af5bf8e2'::uuid,
  true
);

-- 2. Insert App B (household)
INSERT INTO pay.apps (id, slug, display_name, webhook_callback_url, webhook_callback_secret, webhook_ingress_token, enabled)
VALUES (
  '23bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'household',
  'HouseHold',
  'http://localhost:3000/webhooks/dummy',
  'dummy_secret',
  'ea0b93e3-32a1-49d4-ba2f-e421af5bf8e2'::uuid,
  true
);

-- 3. Insert API Key for App A (hiha)
-- Key: sk_hiha_fnP2iRSNMZoNm0HWLNp2MWWIcxawt0fm
INSERT INTO pay.api_keys (app_id, key_prefix, key_hash, label, enabled)
VALUES (
  '43bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'sk_hiha_',
  '$2b$12$OQBHmWyzeBjverDDrkuAuOhQRrNtaK602snucHkdQReoTdvvFOeYa',
  'ci-hiha-key',
  true
);

-- 4. Insert API Key for App B (household)
-- Key: sk_haho_DxcO8Qk01fpD6V1Ov2R8P0Qs3CegemIV
INSERT INTO pay.api_keys (app_id, key_prefix, key_hash, label, enabled)
VALUES (
  '23bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'sk_haho_',
  '$2b$12$3mSLeYtIW0WseymEvShihO1mS.a1h8Emc9jhsazYNiEFNW9QpAr/y',
  'ci-haho-key',
  true
);

-- 5. Insert Provider Configs for App A (hiha)
INSERT INTO pay.provider_configs (app_id, provider, config, enabled)
VALUES (
  '43bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'creem',
  '{"webhook_secret": "whsec_1TSf4A155AEK1KLablY4w7", "verify_webhook_signature": true}'::jsonb,
  true
),
(
  '43bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'google_play',
  '{"package_name": "com.hiha.fe", "verify_audience": false, "service_account_json": "{}", "verify_webhook_signature": false}'::jsonb,
  true
);

-- 6. Insert Provider Configs for App B (household)
INSERT INTO pay.provider_configs (app_id, provider, config, enabled)
VALUES (
  '23bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'creem',
  '{"webhook_secret": "whsec_1TSf4A155AEK1KLablY4w7", "verify_webhook_signature": true}'::jsonb,
  true
),
(
  '23bd7125-87eb-4136-9605-6c5e524f1ab0'::uuid,
  'google_play',
  '{"package_name": "com.tyde.household", "verify_audience": false, "service_account_json": "{}", "verify_webhook_signature": false}'::jsonb,
  true
);
