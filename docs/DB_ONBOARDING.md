# Bridge DB Onboarding — Add a New App

## Overview
Bridge onboarding is DB-driven right now (no admin endpoint for app creation/provider config/API keys), so adding a new app means creating records in:

1. `pay.apps`
2. `pay.provider_configs`
3. `pay.api_keys`

## 1) Create the app record in `pay.apps`

Required fields:
- `slug` (unique)
- `display_name`
- `webhook_callback_url`
- `webhook_callback_secret`

Important optional fields:
- `app_url` (used by Creem checkout success redirect)
- `api_rate_limit_per_minute` (default `120`)
- `api_rate_limit_rules` (JSON overrides)
- `enabled` (must be `true` for auth/webhook resolution)
- `google_package_name`, `apple_bundle_id` (currently informational)

```sql
BEGIN;

INSERT INTO pay.apps (
  slug,
  display_name,
  webhook_callback_url,
  webhook_callback_secret,
  app_url,
  api_rate_limit_per_minute,
  api_rate_limit_rules,
  enabled
)
VALUES (
  'myapp',
  'My App',
  'https://myapp.example.com/api/bridge/webhook',
  '{{STRONG_RANDOM_CALLBACK_SECRET}}',
  'https://myapp.example.com',
  120,
  '{"checkout":20,"verify_purchase":20,"subscription_queries":100,"subscription_mutations":10,"payment_history":100,"purchase_registration":20}'::jsonb,
  true
)
RETURNING id, slug, webhook_ingress_token;

COMMIT;
```

`webhook_ingress_token` is needed when configuring provider webhook URLs.

## 2) Create provider config rows in `pay.provider_configs`

One row per `(app_id, provider)`:
- `provider` must match exactly: `google_play`, `creem`
- `enabled` should be `true` for runtime use

### Google Play

Required keys in `config`:
- `package_name`
- `service_account_json`

Optional:
- `verify_webhook_signature` (defaults to true behavior when absent)

```sql
INSERT INTO pay.provider_configs (app_id, provider, config, enabled)
VALUES (
  '{{APP_ID}}'::uuid,
  'google_play',
  '{
    "package_name":"com.example.myapp",
    "service_account_json":"C:/secure/google-play-service-account.json",
    "verify_webhook_signature":true
  }'::jsonb,
  true
);
```

### Creem

Required:
- `api_key`
- `webhook_secret`

Depending on checkout mode, also needed:
- `product_id` (default path)
- `offer_id` (if `product_type=offer`)
- `otp_id` (if `product_type=otp`)

Optional:
- `verify_webhook_signature` (defaults to true behavior when absent)
- `api_url` (default fallback: `https://api.creem.io/v1`. **Note**: For sandbox/test environments, use `https://test-api.creem.io/v1`)

```sql
INSERT INTO pay.provider_configs (app_id, provider, config, enabled)
VALUES (
  '{{APP_ID}}'::uuid,
  'creem',
  '{
    "api_key":"creem_live_xxx",
    "api_url":"https://api.creem.io/v1",
    "product_id":"premium_monthly",
    "offer_id":"offer_123",
    "otp_id":"otp_456",
    "webhook_secret":"whsec_xxx",
    "verify_webhook_signature":true
  }'::jsonb,
  true
);
```

> [!TIP]
> **Dynamic Configuration on Neon/Render**:
> If you are onboarding on a cloud DB like Neon, you can build your provider configs dynamically in SQL (e.g. using `jsonb_build_object`) to automatically pull the generated `webhook_ingress_token` for your `pub_sub_audience` URL:
> ```sql
> INSERT INTO pay.provider_configs (app_id, provider, config, enabled)
> VALUES (
>   (SELECT id FROM pay.apps WHERE slug = 'household'),
>   'google_play',
>   (
>     SELECT jsonb_build_object(
>       'package_name', 'com.tyde.household',
>       'verify_audience', true,
>       'pub_sub_audience', 'https://pay.tydecode.com/webhooks/' || webhook_ingress_token || '/google_play',
>       'service_account_json', '/etc/secrets/play-billing-482519-28c007356bc6.json',
>       'verify_webhook_signature', true
>     )
>     FROM pay.apps WHERE slug = 'household'
>   ),
>   true
> );
> ```

## 3) Create API key row in `pay.api_keys`

Bridge auth logic expects:
- Bearer token raw key from app
- `key_prefix` = first 8 chars of raw key
- `key_hash` = bcrypt or argon2 hash of the full raw key
- `enabled = true`

```sql
INSERT INTO pay.api_keys (
  app_id,
  key_prefix,
  key_hash,
  label,
  permissions,
  enabled
)
VALUES (
  '{{APP_ID}}'::uuid,
  '{{FIRST_8_OF_RAW_KEY}}',
  '{{BCRYPT_OR_ARGON2_HASH_OF_RAW_KEY}}',
  'production',
  ARRAY[]::text[],
  true
);
```

## 4) Configure provider webhook endpoints

Use this route shape:

`https://pay.yourdomain.com/webhooks/{webhook_ingress_token}/{provider}`

Examples:
- `/webhooks/{token}/google_play`
- `/webhooks/{token}/creem`

## 5) DB validation checks

```sql
-- App row
SELECT id, slug, enabled, webhook_ingress_token
FROM pay.apps
WHERE slug = 'myapp';

-- Provider rows
SELECT provider, enabled, config
FROM pay.provider_configs
WHERE app_id = '{{APP_ID}}'::uuid
ORDER BY provider;

-- API keys
SELECT app_id, key_prefix, label, enabled
FROM pay.api_keys
WHERE app_id = '{{APP_ID}}'::uuid;
```

## 6) Runtime gotchas (important)

- `pay.apps.enabled=false` blocks API-key auth and webhook token app resolution.
- `pay.provider_configs.enabled=false` makes provider lookup fail for most handler paths.
- Provider name matching is exact and case-sensitive.
- `pay.apps.google_package_name` is not used as source-of-truth for Google verification; `provider_configs.config.package_name` is.
- Apple provider exists in schema/docs but is not wired in active runtime handlers yet.
- Active runtime paths read provider config keys directly from `provider_configs.config`.
