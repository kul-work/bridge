# Configuration Reference

This page is aligned with runtime config loading in `src/config.rs` plus additional environment reads in startup, admin auth, email, webhook, and Google Play paths.

Bridge configuration is split into two layers:

- **Process environment**: service-level settings for Bridge itself.
- **Database configuration**: per-app and per-provider settings in `pay.apps`, `pay.provider_configs`, and `pay.api_keys`.

Bridge should not use app-specific provider env vars such as `CREEM_API_KEY` or `GOOGLE_PACKAGE_NAME`. Those belong in `pay.provider_configs` so each Tyde app can have independent payment setup.

## Variables Loaded By `Config::from_env()` (`src/config.rs`)

### Core App

- `DATABASE_URL` (default: `postgresql://localhost/bridge`) - Runtime PostgreSQL connection string. In production this must be set explicitly, must parse as a PostgreSQL connection string, and should use the least-privilege `bridge_app` role.
- `ADMIN_DATABASE_URL` (default: unset) - Elevated PostgreSQL connection string used only to run migrations at startup. Required in production and in any environment where the runtime role is hardened, because `bridge_app` must not own migration-table or schema-change privileges. When unset, migrations run with `DATABASE_URL`; keep that fallback for local development only.
- `SERVER_ADDR` (default: `0.0.0.0`) - Bind address.
- `PORT` (default: `3000`) - Bind port. Must parse as `u16`.
- `LOGGING_LEVEL` (default: `info`) - Loaded into `Config`, but current tracing setup is controlled by `RUST_LOG`/default filters.
- `ENVIRONMENT` (default: `development`) - Used for production safeguards and default tracing filters. `production` and `prod` are treated as production.
- `MOCK_EXTERNAL_APIS` (default: `false`) - Enables local/test provider mocks and test-only webhook header overrides. Startup fails if this is `true` in production.
- `ENABLE_BACKGROUND_JOBS` (default: `true`) - Enables webhook retry, reconciliation, price step-up expiry, pause scheduler, and cleanup workers.

Boolean parsing accepts `1`, `true`, `yes`, `on`, `0`, `false`, `no`, and `off`.

## Additional Runtime Env Vars

These are read outside `Config::from_env()`.

### Tracing and Logs (`src/main.rs`)

- `RUST_LOG` (default depends on `ENVIRONMENT`) - Standard `tracing_subscriber::EnvFilter` value. Production default is `bridge=info,axum=info`; non-production default is `bridge=debug,axum=debug`.

Logs are also written to daily files under `logs/server.YYYY-MM-DD.log`.

### Admin Dashboard Auth (`src/middleware/admin_auth.rs`)

Admin routes under `/admin` require a Clerk session JWT from the configured Clerk instance.

- `ADMIN_CLERK_ORG_ID` (default: unset) - Optional. When set, the JWT's active organization must match this value (requires Clerk organizations/paid plan). When omitted, any valid JWT from the configured Clerk instance is accepted without org membership enforcement. Set this in production to restrict access to your internal Tyde org.
- `ADMIN_CLERK_FRONTEND_API` (default: unset) - Preferred Clerk issuer for admin JWT validation. In production, this URL must use public `https` when set.
- `CLERK_FRONTEND_API` (default: unset) - Fallback Clerk issuer when `ADMIN_CLERK_FRONTEND_API` is unset. In production, this URL must use public `https` when set.
- `CLERK_PUBLISHABLE_KEY` - Required in production for the admin dashboard. Also used to derive the Clerk issuer when both issuer URL vars are unset.
- `ADMIN_CLERK_AUTHORIZED_PARTIES` (default: unset) - Comma-separated allowed browser origins, for example `https://admin.tyde.app`. Required in production; admin JWTs must include an `azp` claim matching one of these origins. Production origins must use public `https`.
- `ADMIN_READ_RATE_LIMIT_PER_MINUTE` (default: `120`) - Per-admin-actor rate limit for admin `GET`/read requests.
- `ADMIN_MUTATION_RATE_LIMIT_PER_MINUTE` (default: `10`) - Per-admin-actor rate limit for admin mutating requests such as `POST` and `PATCH`.
- `ADMIN_AUTH_IP_LIMIT` (default: `10`) - Per-IP limit for failed or missing admin Clerk JWT attempts before JWT parsing. The rolling window is fixed at 60 seconds.
- `RATE_LIMIT_DISABLE` (default: `false`) - Set to `true` to completely disable all rate limiting middlewares (useful for local development, staging, or automated security scans).
- `BYPASS_ADMIN_AUTH` (default: `false`) - Set to `true` to bypass Clerk admin authentication in non-production environments (useful for automated security scanning of admin endpoints). This option is strictly rejected and disabled in production environments.

Issuer fallback order is:

1. `ADMIN_CLERK_FRONTEND_API`
2. `CLERK_FRONTEND_API`
3. Derived from `CLERK_PUBLISHABLE_KEY`

The admin dashboard uses `CLERK_PUBLISHABLE_KEY` to initialize the Clerk JS SDK for client-side sign-in.

In production, startup validation rejects missing `CLERK_PUBLISHABLE_KEY`, missing `ADMIN_CLERK_AUTHORIZED_PARTIES`, non-HTTPS admin issuer/origin URLs, and localhost/private/test hosts for admin issuer/origin URLs.

### Email (`src/main.rs`, `src/services/email.rs`)

- `EMAIL_PROVIDER` (default: `mock`) - Supported values: `mock`, `clerk`, `resend`.
- `CLERK_SECRET_KEY` - Required in production when `EMAIL_PROVIDER=clerk`.
- `RESEND_API_KEY` - Required in production when `EMAIL_PROVIDER=resend`.
- `APP_EMAIL_FROM` (default: `noreply@bridge.local`) - Resend sender address.
- `EMAIL_PROVIDER_DEFAULT_RATE_LIMIT_COOLDOWN_SECONDS` (default: `300`) - Resend cooldown after a `429` when `Retry-After` is missing or invalid.
- `EMAIL_PROVIDER_MAX_RATE_LIMIT_COOLDOWN_SECONDS` (default: `900`) - Upper bound for Resend `Retry-After` cooldown.

For non-production, missing provider credentials fall back to mock email. In production, missing required credentials fail startup.

### Admin Alert Emails (`src/webhooks/processor.rs`, `src/webhooks/scheduler.rs`)

- `ADMIN_ALERT_EMAIL` - Destination for dispute/reconciliation drift alerts.
- `TYDE_SUPPORT_EMAIL` - Fallback alert destination when `ADMIN_ALERT_EMAIL` is unset.

If neither is set, Bridge logs the alert and skips sending email.

### Google Play Test Fixtures

- `MOCK_GOOGLE_PURCHASE_RESPONSE` - Optional path to a JSON fixture used by Google Play mock verification/enrichment paths when `MOCK_EXTERNAL_APIS=true`.

### Google Webhook Controls

Older docs and `.env.sample` mention these process env vars:

- `GOOGLE_VERIFY_WEBHOOK_SIGNATURE`
- `GOOGLE_VERIFY_AUDIENCE`
- `GOOGLE_VERIFY_PUBSUB_IDENTITY`
- `GOOGLE_PUB_SUB_AUDIENCE`
- `GOOGLE_PUB_SUB_SERVICE_ACCOUNT_EMAIL`
- `GOOGLE_SKIP_RSA_VERIFICATION`

Keep Google provider controls in `pay.provider_configs.config` per app where possible. The active webhook ingress also supports process-wide overrides for Pub/Sub audience and service-account email as described below.

### Legacy Or Sample-Only Env Vars

These appear in older docs or `.env.sample`, but active runtime paths do not currently read them:

- `APP_EMAIL_SUPPORT` - Use `ADMIN_ALERT_EMAIL` or `TYDE_SUPPORT_EMAIL` for alert destinations.
- `RATE_LIMIT_CLEANUP_HOURS` - No active cleanup interval reads this env var.

## Database Configuration

Bridge onboarding is DB-driven. See [`DB_ONBOARDING.md`](./DB_ONBOARDING.md) for full SQL examples.

### `pay.apps`

Required fields:

- `slug` - Stable app identifier, for example `hiha`.
- `display_name` - Human-readable app name, for example `HiHa`.
- `webhook_callback_url` - App backend endpoint where Bridge forwards canonical payment events.
- `webhook_callback_secret` - HMAC secret Bridge uses for app callbacks.

Important optional fields:

- `app_url` - Public app URL used for checkout redirects.
- `api_rate_limit_per_minute` (default: `120`) - Per-API-key default limit.
- `api_rate_limit_rules` - JSON endpoint overrides.
- `enabled` (default: `true`) - Must be true for API-key auth and webhook token resolution.
- `google_package_name`, `apple_bundle_id` - Informational. Google runtime paths use `provider_configs.config.package_name`.

Default endpoint rate limit groups:

- `checkout`: `20`
- `verify_purchase`: `20`
- `subscription_queries`: `100`
- `subscription_mutations`: `10`
- `payment_history`: `100`
- `purchase_registration`: `20`
- `default`: `120`

`api_rate_limit_per_minute` caps the `default` group directly and caps other groups when it is lower than the group limit.

### `pay.provider_configs`

One enabled row is expected per `(app_id, provider)`.

#### Google Play

Provider name: `google_play`

Required `config` keys:

- `package_name` - Android package name, for example `app.hiha`.
- `service_account_json` - Path to the Google service account JSON file.
- `pub_sub_service_account_email` - Expected Pub/Sub authenticated push service account email. Required when Google webhook signature verification is enabled in non-mock mode; can be overridden process-wide with `GOOGLE_PUB_SUB_SERVICE_ACCOUNT_EMAIL`.

Optional `config` keys:

- `verify_webhook_signature` (default behavior: `true`) - Verifies Google Pub/Sub JWT signatures for RTDN webhooks.
- `pub_sub_audience` - Expected Pub/Sub JWT audience when `GOOGLE_VERIFY_AUDIENCE=true`; can be overridden process-wide with `GOOGLE_PUB_SUB_AUDIENCE`.
- `verify_pubsub_identity` - Verifies the JWT `email` claim matches the configured Pub/Sub push service account and `email_verified=true`. Non-mock mode forces this on whenever Google webhook signature verification is enabled; `false` is honored only for local/mock testing. Can be overridden process-wide with `GOOGLE_VERIFY_PUBSUB_IDENTITY`.

When Pub/Sub identity verification is enabled, the Pub/Sub JWT must be Google-signed and must include both `email` matching the configured push service account and `email_verified=true`.

Example:

```json
{
  "package_name": "app.hiha",
  "service_account_json": "C:/secure/hiha-google-play-service-account.json",
  "pub_sub_service_account_email": "pubsub-push@your-project.iam.gserviceaccount.com",
  "pub_sub_audience": "https://api.yourdomain.com/webhooks/google",
  "verify_webhook_signature": true
}
```

#### Creem

Provider name: `creem`

Required `config` keys:

- `api_key` - Creem API key.
- `webhook_secret` - HMAC secret used to verify Creem webhook payloads.

Optional `config` keys:

- `api_url` (default: `https://api.creem.com`)
- `offer_id` - Used when checkout request has `product_type=offer`.
- `otp_id` - Used when checkout request has `product_type=otp`.
- `verify_webhook_signature` (default behavior: `true`)
- `connect_timeout_secs` (default: `5`, must be greater than `0`)
- `request_timeout_secs` (default: `25`, must be greater than `0`)

For normal checkout, Bridge uses the request `product_id`. For `product_type=offer` or `product_type=otp`, it uses `offer_id` or `otp_id` for the provider request while preserving the requested app product in metadata.

Example:

```json
{
  "api_key": "creem_live_xxx",
  "api_url": "https://api.creem.com",
  "offer_id": "offer_hiha_premium_monthly",
  "otp_id": "otp_hiha_lifetime",
  "webhook_secret": "whsec_xxx",
  "verify_webhook_signature": true
}
```

### `pay.api_keys`

App backends call Bridge with:

```http
Authorization: Bearer sk_app_...
```

Bridge stores:

- `app_id`
- `key_prefix` - First 8 characters of the raw key.
- `key_hash` - bcrypt or argon2 hash of the full raw key.
- `label`
- `permissions`
- `enabled`

The raw API key is never stored.

## Webhook URLs

Provider webhooks use the app's generated `webhook_ingress_token`:

```text
https://pay.yourdomain.com/webhooks/{webhook_ingress_token}/{provider}
```

Examples:

- `https://pay.yourdomain.com/webhooks/{token}/google_play`
- `https://pay.yourdomain.com/webhooks/{token}/creem`

Provider signature verification must remain enabled in production. The token is an obfuscated routing secret, not sufficient authentication by itself.

## HiHa As A Sample App

For HiHa, Bridge should be configured as an app entry, not hard-coded through global provider env vars:

- `pay.apps.slug`: `hiha`
- `pay.apps.display_name`: `HiHa`
- `pay.apps.webhook_callback_url`: HiHa backend callback endpoint
- `pay.apps.app_url`: HiHa public app URL
- `pay.provider_configs.provider`: `google_play` and/or `creem`
- HiHa backend receives a Bridge API key and calls Bridge under `/api/v1/*`
- HiHa Frontend never talks to Bridge directly

Payment status changes flow:

```text
Provider -> Bridge /webhooks/{token}/{provider} -> HiHa backend callback
```

## Local Development Baseline

Minimal `.env` for local Bridge:

```env
ENVIRONMENT=development
DATABASE_URL=postgresql://user:password@localhost/appgen
PORT=3000
ENABLE_BACKGROUND_JOBS=true
MOCK_EXTERNAL_APIS=false
EMAIL_PROVIDER=mock
```

For admin UI locally, also configure Clerk admin auth:

```env
ADMIN_CLERK_ORG_ID=org_your_internal_tyde_org
CLERK_FRONTEND_API=https://your-clerk-instance.clerk.accounts.dev
CLERK_PUBLISHABLE_KEY=pk_test_xxx
ADMIN_CLERK_AUTHORIZED_PARTIES=https://admin.tyde.app
```

For local provider simulation:

```env
MOCK_EXTERNAL_APIS=true
MOCK_GOOGLE_PURCHASE_RESPONSE=C:/share/tyde/bridge/scratch/google-purchase-fixture.json
```

Never run production with `MOCK_EXTERNAL_APIS=true`; startup rejects it.
