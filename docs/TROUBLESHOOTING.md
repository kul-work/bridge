# Bridge Troubleshooting Runbook

Use this runbook during launch when the fastest useful answer is in backend logs. Search by `request_id` first for user-driven API requests, then add the safe identifiers listed in each section.

Never paste or search for raw API keys, webhook secrets/signatures, purchase tokens, provider payloads, or email credentials in shared logs or tickets.

## Core correlation fields

- `request_id`: accepted from or returned as `x-request-id` on HTTP requests.
- `app_id`: safe application identifier (associated with API keys).
- `external_user_id`: opaque mapping identifier from client apps.
- `subscription_id`, `product_id`, `payment_id`: safe payment/provider identifiers when present.
- `purchase_token` (diagnostic_hash): first 12 characters of the SHA-256 hash of the token.
- `provider`: payment or email provider, such as `google_play`, `creem`, or `resend`.
- `event_id`, `event_type`: webhook/callback event details.

## Alert-key dashboards and routing

Build Bridge dashboards from structured log fields, grouped by `alert_key`. Keep `signal_class` in every query so support/debug signals stay visible without paging.

Dashboard queries:

```text
alert_key="bridge.webhook.dead_lettered"
alert_key="bridge.callback.delivery_failed"
alert_key="bridge.reconciliation.drift_detected"
alert_key="bridge.reconciliation.job_failed"
alert_key="bridge.email.auth_or_permission_failed"
alert_key="bridge.db.readiness_failed"
alert_key="bridge.db.role_or_rls_failed"
```

Initial alert routing:

| `alert_key` | Route | Why |
|---|---|---|
| `bridge.webhook.dead_lettered` | Ticket | App callback delivery is stuck after all retries. |
| `bridge.reconciliation.drift_detected` | Audit | Provider truth corrected Bridge state; review for recurring drift. |
| `bridge.reconciliation.job_failed` | Ticket | Reconciliation self-healing did not run successfully. |
| `bridge.email.auth_or_permission_failed` | Ticket | Admin/lifecycle email provider credentials or permissions need operator action. |
| `bridge.db.readiness_failed` | Page | Bridge cannot prove database readiness or connect at startup. |
| `bridge.db.role_or_rls_failed` | Page | Runtime DB role, migration, or app-context setup is broken. |
| `bridge.callback.delivery_failed` | Dashboard only | Retryable callback failure before dead-lettering; useful for trend/debug, not paging. |

Do not route duplicate delivery, malformed/manual traffic, isolated provider noise, or retryable callback failures to paging from these baseline queries. Add spike or prolonged-failure thresholds only after staging or launch-window log volume gives a normal baseline.

## Health, readiness, and startup failures

Check:

- `GET /health` for liveness only.
- `GET /ready` for readiness, database connectivity, and configuration checks.
- Deployment logs around process startup and bind/listen.

Search for:

- `database pool connect failed`
- `database readiness check failed`
- `status: "not_ready"`
- `No configurations enabled`
- bind/listen errors from startup logs

Migration failures usually come from the deploy or SQLx migration command, not from normal app runtime. Check the migration job output for `sqlx migrate run`, the database role used for that job, and the schema version recorded.

## Auth and rate-limiting failures

For client API requests, get the returned `x-request-id` or reproduce and capture it from the response header.

Search for:

- `unauthorized` or `app_disabled`
- `rate_limit_exceeded`
- `HTTP request completed` with `status=401` or `status=429`

Check:

- The request has `Authorization: Bearer sk_hiha_xxxxx`.
- Hashed API key matches `api_keys.key_hash` and is enabled.
- App rate limits defined in `apps.api_rate_limit_rules` are not overly restrictive.

## Database and RLS failures

Search for:

- `database pool connect failed`
- SQLx timeout/acquire/query errors
- RLS/session-context setup failures (e.g., setting `bridge.current_app_id`)
- Database access issues in `pay.*` tables

Check:

- Runtime `DATABASE_URL` uses the app role, not the migration/admin role.
- Migrations were applied with a migration/admin role before the runtime deploy started.
- TLS settings match the database configuration (e.g., `sslmode`).

## Webhook Ingress failures

Bridge receives provider webhooks at `POST /webhooks/{token}/:provider`.

Search for:

- `Webhook signature verification failed`
- `Webhook uniqueness check failed` or `webhook_log` deduplication errors
- `JSON payload validation failed`
- `webhook_log` primary key constraint failures

Useful fields:

- `provider_webhook_id`
- `provider`
- `event_type`
- `app_id`

Common interpretations:
- Invalid inbound signature: compare provider webhook secrets configured in `provider_configs`. Do not log the signature.
- Duplicate webhook: the webhook was already logged in `webhook_log`. This is expected idempotency behavior.

## Provider RPC and client failures

Bridge calls provider APIs (Google Play Developer API, Creem API).

Search for:

- `Provider API failure` or timeout errors
- `Failed to parse provider response`
- `invalid_purchase_token` or `provider_error`

Useful fields:

- `provider`
- `app_id`
- `subscription_id`
- `purchase_token_hash`

Check Google Play / Creem Developer console and API service account credentials stored in `provider_configs`. Raw provider response bodies and raw purchase tokens must not be logged on failure.

## Subscription Lifecycle transitions

Search for:

- `Subscription state transition`
- `Stale event suppressed`
- `last_event_time` comparison logs

Useful fields:

- `app_id`
- `subscription_id`
- `external_user_id`
- `old_state` -> `new_state`

If a subscription did not update, check if a newer event had already processed (stale events are suppressed using high-water mark timestamp comparison).

## Webhook Sub-delivery & retry delivery issues

Bridge forwards callbacks to client apps at their configured `webhook_callback_url`.

Search for:

- `Webhook delivery attempt failed`
- `dead_lettered`
- `X-Pay-Signature` signature checking logs

Useful fields:

- `app_id`
- `webhook_id`
- `attempt` (1 of 3, etc.)
- `status_code`

If a callback delivery fails continuously, it will exhaust all 3 attempts (using exponential backoff) and be marked `dead_lettered`. Verify client app webhook signature verification configuration (`webhook_callback_secret`) and log targets.

## Background worker execution failures

Search for:

- `Reconciliation job failed`
- `Price step-up worker error`
- `Pause scheduler worker error`
- `Cleanup worker error`

Check:
- `ENABLE_BACKGROUND_JOBS=true` in environment variables.
- Reconciliation polling API quotas are not exhausted.

## Quick launch-window routine

1. Confirm `/health` and `/ready`.
2. For user issues, capture `x-request-id` and search `HTTP request completed`.
3. Check `webhook_delivery` table for any queued or `dead_lettered` callbacks.
4. For webhook signature issues, verify app callback secret and provider configs.
5. If the issue is provider-specific, search logs with the purchase token's `diagnostic_hash`.

## Security Scanning (OWASP ZAP)

If you are running an automated security scanner like OWASP ZAP, you may encounter issues such as rate-limit blocks (HTTP 429) or orphaned browser processes on Windows.

### Troubleshooting ZAP Scans on Windows

1. **Rate Limiting (HTTP 429)**
   ZAP sends request bursts that exceed the default unauthenticated rate limit (10 requests/min).
   - Set `RATE_LIMIT_DISABLE=true` in `.env` to bypass all rate limit checks during the scan.

2. **Orphaned Firefox / Geckodriver Processes**
   When using ZAP's active browser scanning on Windows, it might spawn and orphan multiple browser and driver instances. Run these commands to clean up the environment:
   ```cmd
   taskkill /F /IM firefox.exe
   taskkill /F /IM geckodriver.exe
   ```

3. **Scanning Protected Endpoints**
   Configure ZAP's **Replacer** tool to inject a test API key header to authenticate requests under `/api/v1`:
   - **Type**: `Request Header (will add if not present)`
   - **Match**: `Authorization`
   - **Replacement**: `Bearer <dev_api_key>` (e.g. `sk_hiha_fnP2iRSNMZoNm0HWLNp2MWWIcxawt0fm` from dev/test config)

4. **Scanning Admin Endpoints (/admin)**
   Clerk authentication requires interactive loading of external scripts which can fail or expire during scanning.
   - Set `BYPASS_ADMIN_AUTH=true` in `.env` in non-production environments to completely bypass Clerk validation and mock the admin session. This allows ZAP to crawl and scan all admin endpoints directly. **Note**: This setting is strictly rejected and disabled in production.
