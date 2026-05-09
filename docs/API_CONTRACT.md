# Bridge API Contract

> Status: Implemented contract, aligned with the current Bridge codebase.
> Base URL: `https://pay.tydecode.com`
> Public app API prefix: `/api/v1`

This document describes the app-facing Bridge API implemented by the Rust/Axum service. Bridge is the payment boundary for Tyde apps: apps authenticate with Bridge API keys, Bridge talks to providers, and Bridge forwards normalized payment events back to each app callback URL.

## Authentication

All app-facing `/api/v1/*` endpoints require API key authentication:

```http
Authorization: Bearer sk_hiha_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

The API key resolves the app. Request bodies do not include `app_id`; Bridge scopes all reads and writes to the authenticated app.

Webhook callbacks from Bridge to apps are signed:

```http
X-Pay-Signature: sha256=<hex-encoded HMAC-SHA256 of raw JSON body>
X-Pay-Timestamp: 1711000000
X-Pay-Event-Id: evt_uuid
Content-Type: application/json
```

## Current Provider Support

Implemented public API support:

| Provider | Checkout | Verify purchase | Cancel | Resume | Billing portal | Webhook ingress |
|---|---:|---:|---:|---:|---:|---:|
| `creem` | yes | no | yes | yes | yes | yes |
| `google_play` | yes | yes | yes | no | no | yes |
| `apple` | checkout metadata only | not implemented | no | no | no | no |

Unsupported provider operations return `validation_error`, `provider_not_configured`, `config_error`, or `provider_error` depending on where the failure happens.

## Provider Disambiguation

Bridge treats `(app_id, external_user_id, subscription_id, provider)` as the subscription identity. Apps must pass `provider` when reading or mutating one specific subscription.

Examples:

```http
GET /api/v1/subscriptions?external_user_id=user123
GET /api/v1/subscriptions/premium_monthly?external_user_id=user123&provider=google_play
POST /api/v1/subscriptions/premium_monthly/cancel?external_user_id=user123&provider=creem
```

Apps must not assume a user has only one subscription per product ID across providers.

## Health

### `GET /health`

No API key required.

Response `200`:

```json
{
  "status": "healthy",
  "version": "0.3.0"
}
```

`version` is the compiled Cargo package version.

## Checkout

### `POST /api/v1/checkout`

Creates provider checkout data.

Request:

```json
{
  "external_user_id": "clerk_abc123",
  "email": "user@example.com",
  "provider": "creem",
  "product_id": "premium_monthly",
  "product_type": "subscription",
  "idempotency_key": "uuid-or-client-retry-key"
}
```

| Field | Required | Notes |
|---|---:|---|
| `external_user_id` | yes | Opaque app user ID. |
| `email` | yes | Valid email string. |
| `provider` | yes | `creem`, `google_play`, or `apple`. Normalized to lowercase. |
| `product_id` | yes | App/provider product identifier. |
| `product_type` | no | Passed through for mobile checkout data. Creem also recognizes `offer` and `otp` as config aliases. |
| `idempotency_key` | no | If reused with the same payload, returns the cached response. If reused with a different payload, returns `validation_error`. |

Creem response `201`:

```json
{
  "checkout_id": "creem_session_id",
  "provider": "creem",
  "redirect_url": "https://checkout.creem.io/...",
  "mobile_checkout_data": null
}
```

Google Play response `201`:

```json
{
  "checkout_id": "generated_uuid",
  "provider": "google_play",
  "redirect_url": null,
  "mobile_checkout_data": {
    "provider": "google_play",
    "platform": "android",
    "package_name": "app.package.name",
    "external_user_id": "clerk_abc123",
    "email": "user@example.com",
    "product_id": "premium_monthly",
    "sku": "premium_monthly",
    "product_type": "subscription"
  }
}
```

Apple response `201` has the same shape as Google Play, with `provider: "apple"`, `platform: "ios"`, and `bundle_id`.

## Purchase Verification

### `POST /api/v1/verify-purchase`

Verifies a mobile purchase token and records the subscription or one-time purchase. Currently implemented for `google_play`; `apple` returns `validation_error`.

Request:

```json
{
  "external_user_id": "clerk_abc123",
  "provider": "google_play",
  "subscription_id": "premium_monthly",
  "purchase_token": "token_from_google_play",
  "product_type": "subscription"
}
```

| Field | Required | Notes |
|---|---:|---|
| `external_user_id` | yes | Opaque app user ID. |
| `provider` | yes | Currently `google_play` for successful verification. |
| `subscription_id` | yes | Product/subscription ID. Used for one-time products too. |
| `purchase_token` | yes | Provider purchase token. |
| `product_type` | yes | `subscription`, `sub`, `subs`, `one_time`, `one-time`, or `inapp`. |

Response `200`:

```json
{
  "status": "active",
  "subscription_id": "premium_monthly",
  "current_period_end": "2026-04-18T00:00:00+00:00",
  "auto_renewing": true,
  "amount_cents": 299,
  "is_new": true
}
```

Linking-required response `200`:

```json
{
  "status": "linking_required",
  "subscription_id": "premium_monthly",
  "current_period_end": null,
  "auto_renewing": null,
  "amount_cents": null,
  "is_new": false,
  "message": "This purchase belongs to a different Google Play account and must be linked first",
  "obfuscated_account_id": "google_obfuscated_account_hash"
}
```

Successful non-pending verification also emits a Bridge-to-app callback with `event_type` `subscription.activated` for subscriptions or `purchase.one_time` for one-time products.

## Subscriptions

### `GET /api/v1/subscriptions`

Lists subscriptions for a user in the authenticated app.

Query parameters:

| Field | Required | Notes |
|---|---:|---|
| `external_user_id` | yes | Opaque app user ID. |
| `limit` | no | Default `20`, max `100`. |
| `after` | no | Opaque base64 cursor from the previous response. |

Response `200`:

```json
{
  "subscriptions": [
    {
      "id": "subscription_uuid",
      "subscription_id": "premium_monthly",
      "provider": "google_play",
      "status": "active",
      "current_period_end": "2026-04-18T00:00:00+00:00",
      "auto_renewing": true,
      "payment_failure_notification": false,
      "payment_state": 1,
      "cancel_reason": null,
      "provider_customer_id": null,
      "cancellation_initiated_at": null,
      "revocation_reason": null,
      "revoked_at": null,
      "google_requires_price_step_up_consent": false,
      "google_price_step_up_consent_deadline": null,
      "google_new_price_cents": null,
      "google_pause_scheduled_at": null,
      "google_paused_at": null,
      "google_deferred_until": null,
      "last_event_time": 1711000700000
    }
  ],
  "pagination": {
    "has_more": false,
    "after": null
  }
}
```

### `GET /api/v1/subscriptions/:subscription_id`

Gets one subscription for a user and provider.

Query parameters:

| Field | Required |
|---|---:|
| `external_user_id` | yes |
| `provider` | yes |

Response `200` is the same subscription detail shape as list results, except it does not include `payment_failure_notification` or `last_event_time`.

### `GET /api/v1/users/:external_user_id/subscription-status`

Returns Bridge's selected premium snapshot for an app user.

Response `200`:

```json
{
  "is_premium": true,
  "subscription_id": "premium_monthly",
  "provider": "google_play",
  "status": "active",
  "current_period_end": "2026-04-18T00:00:00+00:00",
  "auto_renewing": true,
  "payment_failure_notification": false,
  "revoked_at": null,
  "revocation_reason": null,
  "google_requires_price_step_up_consent": false,
  "google_new_price_cents": null,
  "google_price_step_up_consent_deadline": null,
  "google_pause_scheduled_at": null,
  "google_deferred_until": null,
  "last_event_time": 1711000700000
}
```

`is_premium` is true when any subscription is `active`, `trial`, or `past_due`.

## Subscription Actions

### `POST /api/v1/subscriptions/:subscription_id/cancel`

Query parameters:

| Field | Required |
|---|---:|
| `external_user_id` | yes |
| `provider` | yes |

Optional body:

```json
{
  "mode": "scheduled",
  "purchase_token": "google_play_purchase_token"
}
```

`mode` must be `scheduled` or `immediate`; default is `scheduled`. Google Play requires a purchase token, either in the body or already stored on the subscription.

Response `200`:

```json
{
  "status": "cancelled",
  "mode": "scheduled",
  "subscription_id": "premium_monthly"
}
```

### `POST /api/v1/subscriptions/:subscription_id/resume`

Query parameters:

| Field | Required |
|---|---:|
| `external_user_id` | yes |
| `provider` | yes |

Currently supported by Creem.

Response `200`:

```json
{
  "status": "active",
  "subscription_id": "premium_monthly"
}
```

### `POST /api/v1/subscriptions/:subscription_id/acknowledge`

Marks the subscription payment as acknowledged and clears the payment failure notification flag.

Request:

```json
{
  "external_user_id": "clerk_abc123"
}
```

Response `200`:

```json
{
  "success": true,
  "message": "Subscription acknowledged"
}
```

### `POST /api/v1/subscriptions/:subscription_id/portal`

Creates a provider billing portal URL. Currently supported by Creem.

Query parameters:

| Field | Required |
|---|---:|
| `external_user_id` | yes |
| `provider` | yes |

Response `200`:

```json
{
  "url": "https://billing.creem.io/..."
}
```

### `POST /api/v1/subscriptions/:subscription_id/price-step-up/accept`

Google Play only.

Request:

```json
{
  "external_user_id": "clerk_abc123"
}
```

Response `200`:

```json
{
  "accepted": true,
  "new_price_cents": 499
}
```

### `POST /api/v1/subscriptions/:subscription_id/price-step-up/decline`

Google Play only. Declining schedules cancellation through the provider.

Request:

```json
{
  "external_user_id": "clerk_abc123"
}
```

Response `200`:

```json
{
  "declined": true,
  "cancellation_effective_at": "2026-04-18T00:00:00+00:00"
}
```

## Payments

### `GET /api/v1/payments`

Lists payment history for a user.

Query parameters:

| Field | Required | Notes |
|---|---:|---|
| `external_user_id` | yes | Opaque app user ID. |
| `limit` | no | Default `20`, max `100`. |
| `after` | no | Opaque base64 cursor from the previous response. |

Response `200`:

```json
{
  "payments": [
    {
      "id": "payment_uuid",
      "external_user_id": "clerk_abc123",
      "subscription_id": "premium_monthly",
      "provider": "google_play",
      "provider_transaction_id": "purchase_token_or_provider_transaction_id",
      "amount_cents": 299,
      "currency": "USD",
      "status": "success",
      "created_at": "2026-03-18T10:00:00+00:00"
    }
  ],
  "total": 1,
  "limit": 20,
  "pagination": {
    "has_more": false,
    "after": null
  }
}
```

### `POST /api/v1/purchase/register`

Creates a pending subscription placeholder. This is not a manual grant endpoint.

Request:

```json
{
  "external_user_id": "clerk_abc123",
  "subscription_id": "premium_monthly",
  "provider": "google_play",
  "reason": "client_started_purchase"
}
```

Response `200`:

```json
{
  "status": "registered"
}
```

## User Data

### `POST /api/v1/users/:external_user_id/anonymize`

Cancels active subscriptions through Bridge records and anonymizes stored payment identity for the authenticated app.

Request:

```json
{
  "reason": "user_requested_deletion"
}
```

Response `200`:

```json
{
  "anonymized": true,
  "subscriptions_cancelled": 1,
  "payments_anonymized": 12,
  "new_anonymous_id": "deleted_hash_ab7b92f..."
}
```

If no records are found, the current implementation returns `400 validation_error` with message `User not found`.

### `GET /api/v1/users/:external_user_id/data-export`

Returns all Bridge-held data for an app user, including webhook records associated with that user's subscription IDs and payment tokens.

Response `200`:

```json
{
  "external_user_id": "clerk_abc123",
  "export_date": "2026-03-18T12:00:00Z",
  "subscriptions": [],
  "payments": [],
  "webhook_records": []
}
```

## Provider Webhook Ingress

Provider webhook endpoints are not under `/api/v1`.

```http
POST /webhooks/:token/google_play
POST /webhooks/:token/creem
```

`:token` is the app's webhook ingress token. Bridge resolves the app from that token, stores/deduplicates the provider webhook, processes it asynchronously, and forwards a normalized callback to the app when possible.

Google Play ingress can verify Pub/Sub Authorization JWTs when provider config enables `verify_webhook_signature`. Creem ingress verifies HMAC signatures when provider config enables `verify_webhook_signature`; accepted signature header names are `Webhook-Signature`, `creem-signature`, and `x-signature`.

Ingress responses are `204 No Content` for accepted events and duplicates that can be ignored or resumed. Invalid token paths return `404`.

## Bridge-to-App Callback Payload

Normalized callbacks use this payload shape:

```json
{
  "event_id": "evt_uuid",
  "event_type": "subscription.activated",
  "timestamp": "2026-03-18T10:05:00+00:00",
  "timestamp_epoch_ms": 1711000700000,
  "app_slug": "hiha",
  "product_id": "premium_monthly",
  "subscription_id": "premium_monthly",
  "external_user_id": "clerk_abc123",
  "amount_cents": 299,
  "new_price_cents": null,
  "auto_renewing": true,
  "purchase_token": "token_xxx",
  "current_period_end": "2026-04-18T00:00:00+00:00",
  "status": "active",
  "provider": "google_play",
  "provider_event_id": "provider_event_id",
  "previous_status": null,
  "corrected_status": null,
  "reconciliation_source": null,
  "revocation_reason": null,
  "cancellation_mode": null,
  "google_price_step_up_consent_deadline": null,
  "google_pause_scheduled_at": null,
  "google_deferred_until": null
}
```

Common callback event types emitted by the current code include:

| Event type |
|---|
| `subscription.activated` |
| `subscription.renewed` |
| `subscription.trial_started` |
| `subscription.expired` |
| `subscription.cancelled` |
| `subscription.revoked` |
| `subscription.paused` |
| `subscription.resumed` |
| `subscription.grace_period` |
| `subscription.on_hold` |
| `subscription.recovered` |
| `subscription.price_step_up` |
| `payment.pending` |
| `payment.failed` |
| `payment.refunded` |
| `payment.partially_refunded` |
| `purchase.one_time` |
| `dispute.created` |
| `reconciliation.drift_detected` |

Bridge expects any `2xx` response from the app within 10 seconds. Failed deliveries are retried by the background worker up to 3 attempts, then dead-lettered. Delivery is at-least-once; apps must use `X-Pay-Event-Id` or `event_id` for idempotency and `timestamp_epoch_ms` to ignore stale events.

## Admin Endpoints

Admin endpoints are mounted under `/admin` and use separate admin bearer auth:

```http
GET /admin/
GET /admin/apps
GET /admin/apps/:app_id/webhooks
POST /admin/webhooks/:webhook_id/retry
```

`POST /admin/webhooks/:webhook_id/retry` currently validates the webhook ID and returns `200`; the handler contains a TODO for queueing the retry.

## Errors

Errors use this JSON shape:

```json
{
  "error": "validation_error",
  "message": "external_user_id is required"
}
```

Implemented error codes:

| HTTP status | Error code |
|---:|---|
| `400` | `validation_error` |
| `400` | `bad_request` |
| `400` | `webhook_error` |
| `400` | `provider_not_configured` |
| `401` | `unauthorized` |
| `403` | `app_disabled` |
| `404` | `subscription_not_found` |
| `409` | `fraud_detected` |
| `429` | `rate_limit_exceeded` |
| `500` | `database_error` |
| `500` | `internal_server_error` |
| `500` | `config_error` |
| `502` | `provider_error` |

## Rate Limiting

Rate limiting is in-memory per process.

Protected API requests are limited per API key and endpoint group:

| Group | Default limit |
|---|---:|
| Checkout | 20 req/min |
| Verify purchase | 20 req/min |
| Subscription queries | 100 req/min |
| Subscription mutations | 10 req/min |
| Payment history | 100 req/min |
| Purchase registration | 20 req/min |
| Default | 120 req/min |

App-level `api_rate_limit_per_minute` caps group limits. `api_rate_limit_rules` can override group defaults, but overrides are still capped by the app-level limit.

Successful protected API responses include:

```http
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 95
X-RateLimit-Reset: 1711000060
```

Missing or failed API-key requests are limited per client IP at 10 requests/minute. Provider webhook ingress is not part of the `/api/v1` API-key rate limiter.
