# Bridge — API Contract Proposal

> Status: **Proposal / Under Review**
> Base URL: `https://pay.tydecode.com` (Bridge)

---

## Authentication

All API calls from apps use API key authentication:

```
Authorization: Bearer sk_hiha_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

The API key identifies the app. All operations are scoped to that app — no `app_id` needed in request bodies.

Callbacks from Bridge to apps use HMAC-SHA256 signature:

```
X-Pay-Signature: sha256=<hex-encoded HMAC of raw body using app's webhook_callback_secret>
X-Pay-Timestamp: 1711000000
```

---

## Provider Disambiguation & Multi-Provider Subscriptions

### Problem: Multiple Subscriptions with Same ID

A single user can have the same `subscription_id` (e.g., `premium_monthly`) active across **multiple providers**:
- `premium_monthly` on Google Play (mobile)
- `premium_monthly` on Creem (web)
- `premium_monthly` on Apple (future)

The database schema enforces uniqueness via `UNIQUE (app_id, external_user_id, subscription_id, provider)`, so all combinations are allowed.

### Solution: Always Specify Provider

When querying or managing a subscription, **always include the `provider` parameter**:

- `GET /api/v1/subscriptions?external_user_id=user123` → returns **all subscriptions** for all providers
- `GET /api/v1/subscriptions/premium_monthly?external_user_id=user123&provider=google_play` → returns **exactly one** subscription (if it exists)
- `POST /api/v1/subscriptions/premium_monthly/cancel?external_user_id=user123&provider=creem` → cancels **only the Creem subscription**, not the Google Play one

**Apps must NOT assume a single subscription per user/product combination.**

---

## Endpoints

### Health

#### `GET /health`

No auth required. No rate limiting.

**Response** `200`:
```json
{
  "status": "healthy",
  "version": "1.0.0"
}
```

---

### Checkout

#### `POST /api/v1/checkout`

Create a checkout session with a payment provider. Bridge orchestrates the session and returns redirect/purchase info to the app.

**Request**:
```json
{
  "external_user_id": "clerk_abc123",
  "email": "user@example.com",
  "provider": "google_play",
  "product_id": "premium_monthly",
  "product_type": "subscription",
  "idempotency_key": "uuid_v4_for_retry_safety"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `external_user_id` | string | yes | Opaque user ID from the app |
| `email` | string | yes | User email (needed by some providers for checkout) |
| `provider` | string | yes | `google_play`, `apple`, `creem`, `lemonsqueezy` |
| `product_id` | string | yes | Opaque product identifier from the app |
| `product_type` | string | no | `subscription` or `one_time`. Default: `subscription` |
| `idempotency_key` | string (UUID) | no | Optional idempotency key to prevent duplicate checkout sessions on retries. If provided, subsequent requests with the same key return the cached response. |

> **Note on pricing**: `amount_cents` is intentionally absent. The price authority is always the provider (Google Play Console, Creem dashboard, LemonSqueezy dashboard, etc.). Bridge populates `amount_cents` in the `payments` table from the provider's webhook or verification response — never from the app.

**Note on Idempotency Implementation**: The `idempotency_key` prevents duplicate checkout sessions on network retries. Implementation can use:
- In-memory cache (short-lived), or
- Redis (if distributed), or
- Query existing `webhook_log` for duplicate `app_id + external_user_id + product_id` as natural deduplication

No new database table required.

**Response** `200` — Web provider (Creem, LemonSqueezy, Coinbase):
```json
{
  "checkout_id": "chk_xxxxx",
  "redirect_url": "https://checkout.creem.io/...",
  "provider": "creem"
}
```

**Response** `200` — Mobile provider (Google Play, Apple):
```json
{
  "checkout_id": "chk_xxxxx",
  "redirect_url": null,
  "provider": "google_play",
  "mobile_checkout_data": {
    "sku_details": {...},
    "product_id": "premium_monthly",
    "product_type": "subs"
  }
}
```

**Note on Provider-Specific Responses**:
- **Web providers** (Creem, LemonSqueezy, Coinbase): Return a `redirect_url` for browser-based checkout. App redirects user to this URL.
- **Mobile providers** (Google Play, Apple): Return `null` for `redirect_url`. Instead, return provider-specific data (e.g., `mobile_checkout_data`) that the mobile app passes to the native billing SDK (`BillingClient.launchBillingFlow()` for Google Play, SKPaymentQueue for Apple).
- Apps MUST inspect `provider` field and handle responses accordingly (web redirect vs. mobile SDK flow).

**Response** `400` — invalid provider or missing credentials for app:
```json
{
  "error": "provider_not_configured",
  "message": "Provider 'apple' is not configured for this app"
}
```

---

### Purchase Verification (Mobile)

#### `POST /api/v1/verify-purchase`

Verify a mobile purchase token (Google Play / Apple) and activate the subscription.

**Request**:
```json
{
  "external_user_id": "clerk_abc123",
  "provider": "google_play",
  "subscription_id": "premium_monthly",
  "purchase_token": "token_from_google_play_sdk",
  "product_type": "subscription"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `external_user_id` | string | yes | Opaque user ID |
| `provider` | string | yes | `google_play` or `apple` |
| `subscription_id` | string | yes | Product/subscription ID from the store |
| `purchase_token` | string | yes | Purchase token from the mobile SDK |
| `product_type` | string | no | `subscription` or `one_time`. Default: `subscription` |

**Response** `200` — verified and activated:
```json
{
  "status": "active",
  "subscription_id": "premium_monthly",
  "current_period_end": "2026-04-18T00:00:00Z",
  "auto_renewing": true,
  "amount_cents": 299,
  "is_new": true
}
```

**Response** `200` — linking required (token belongs to different user):
```json
{
  "status": "linking_required",
  "message": "Purchase token is associated with a different account"
}
```

**Response** `400` — invalid token:
```json
{
  "error": "invalid_purchase_token",
  "message": "Purchase token verification failed with provider"
}
```

---

### Subscription Status

#### `GET /api/v1/subscriptions?external_user_id=clerk_abc123&limit=10&after=cursor`

Get all subscriptions for a user within this app.

**Query Parameters**:
| Field | Type | Required | Description |
|---|---|---|---|
| `external_user_id` | string | yes | Opaque user ID |
| `limit` | int | no | Max results per page. Default: 20, max: 100 |
| `after` | string | no | Cursor for pagination (opaque token from previous response) |

**Response** `200`:
```json
{
  "subscriptions": [
    {
      "subscription_id": "premium_monthly",
      "provider": "google_play",
      "status": "active",
      "current_period_end": "2026-04-18T00:00:00Z",
      "auto_renewing": true,
      "created_at": "2026-03-18T10:00:00Z"
    }
  ],
  "pagination": {
    "has_more": true,
    "after": "cursor_token_for_next_page"
  }
}
```

---

#### `GET /api/v1/subscriptions/:subscription_id?external_user_id=clerk_abc123&provider=google_play`

Get a specific subscription.

**Query Parameters**:
| Field | Type | Required | Description |
|---|---|---|---|
| `external_user_id` | string | yes | Opaque user ID |
| `provider` | string | yes | `google_play`, `apple`, `creem`, or `lemonsqueezy`. **Required to disambiguate** when a user has the same `subscription_id` across multiple providers (e.g., premium_monthly on both Google Play and Creem). |

**Response** `200`:
```json
{
  "subscription_id": "premium_monthly",
  "provider": "google_play",
  "status": "active",
  "current_period_end": "2026-04-18T00:00:00Z",
  "auto_renewing": true,
  "payment_state": 1,
  "google_subscription_state": 0,
  "google_pause_scheduled_at": null,
  "google_requires_price_step_up_consent": false,
  "created_at": "2026-03-18T10:00:00Z",
  "updated_at": "2026-03-18T10:00:00Z"
}
```

---

### Subscription Management

#### `POST /api/v1/subscriptions/:subscription_id/cancel?external_user_id=clerk_abc123&provider=google_play`

Cancel a subscription.

**Query Parameters**:
| Field | Type | Required | Description |
|---|---|---|---|
| `external_user_id` | string | yes | Opaque user ID |
| `provider` | string | yes | `google_play`, `apple`, `creem`, or `lemonsqueezy`. **Required to disambiguate** which provider's subscription to cancel (see "Provider Disambiguation"). |

**Request Body**:
```json
{
  "mode": "end_of_period",
  "purchase_token": "token_xxx"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `mode` | string | no | `immediate` or `end_of_period`. Default: provider-specific |
| `purchase_token` | string | no | Required for Google Play |

**Response** `200`:
```json
{
  "status": "cancelled",
  "effective_at": "2026-04-18T00:00:00Z"
}
```

---

#### `POST /api/v1/subscriptions/:subscription_id/resume?external_user_id=clerk_abc123&provider=google_play`

Resume a paused/cancelled subscription (if provider supports it).

**Query Parameters**:
| Field | Type | Required | Description |
|---|---|---|---|
| `external_user_id` | string | yes | Opaque user ID |
| `provider` | string | yes | `google_play`, `apple`, `creem`, or `lemonsqueezy`. **Required to disambiguate** which provider's subscription to resume. |

**Request Body**: Empty `{}`

**Response** `200`:
```json
{
  "status": "active"
}
```

---

#### `POST /api/v1/subscriptions/:subscription_id/acknowledge`

Acknowledge a purchase with the provider (Google Play 3-day requirement).

**Request**:
```json
{
  "external_user_id": "clerk_abc123",
  "purchase_token": "token_xxx",
  "purchase_type": "subscription"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `purchase_type` | string | no | `subscription` or `one_time`. Default: `subscription` |

**Response** `200`:
```json
{
  "acknowledged": true
}
```

---

### Price Step-Up Consent (Google Play)

#### `POST /api/v1/subscriptions/:subscription_id/price-step-up/accept`

**Request**:
```json
{
  "external_user_id": "clerk_abc123"
}
```

**Response** `200`:
```json
{
  "accepted": true,
  "new_price_cents": 499
}
```

---

#### `POST /api/v1/subscriptions/:subscription_id/price-step-up/decline`

**Request**:
```json
{
  "external_user_id": "clerk_abc123"
}
```

**Response** `200`:
```json
{
  "declined": true,
  "cancellation_effective_at": "2026-04-18T00:00:00Z"
}
```

---

### Billing Portal

#### `POST /api/v1/subscriptions/:subscription_id/portal`

Get a billing management URL (if provider supports it — Creem, LemonSqueezy).

**Request**:
```json
{
  "external_user_id": "clerk_abc123"
}
```

**Response** `200`:
```json
{
  "portal_url": "https://billing.creem.io/manage/..."
}
```

---

### Payment History

#### `GET /api/v1/payments?external_user_id=clerk_abc123&limit=50&after=txn_uuid`

Get payment history for a user.

**Response** `200`:
```json
{
  "payments": [
    {
      "id": "txn_uuid",
      "provider": "google_play",
      "product_id": "premium_monthly",
      "amount_cents": 299,
      "currency": "USD",
      "status": "success",
      "created_at": "2026-03-18T10:00:00Z"
    }
  ]
}
```

---

### Purchase Registration (Server-Side)

#### `POST /api/v1/purchases/register`

Register a purchase directly (for server-side payment flows or manual grants).

**Request**:
```json
{
  "external_user_id": "clerk_abc123",
  "provider": "manual",
  "product_id": "premium_lifetime",
  "product_type": "one_time",
  "amount_cents": 0,
  "transaction_id": "manual_grant_20260318"
}
```

**Response** `200`:
```json
{
  "registered": true,
  "transaction_id": "manual_grant_20260318"
}
```

---

### User Data & Compliance (GDPR)

#### `POST /api/v1/users/:external_user_id/anonymize`

Anonymize a user's data to comply with GDPR/CCPA "Right to be Forgotten" and Apple/Google account deletion requirements.
When a client app deletes a user, it must call this endpoint to sever the user's identity from the payment gateway.
This action will immediately cancel any active subscriptions to prevent future charges, clear mobile provider links (obfuscated account/profile IDs), and hash the `external_user_id` on all historical payment records.

**Request**:
```json
{
  "reason": "user_requested_deletion"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `reason` | string | no | Optional reason for the deletion/anonymization (e.g., `user_requested_deletion`, `fraud_ban`) |

**Response** `200`:
```json
{
  "anonymized": true,
  "subscriptions_cancelled": 1,
  "payments_anonymized": 12,
  "new_anonymous_id": "deleted_hash_ab7b92f..."
}
```

**Response** `404` — no records found:
```json
{
  "error": "user_not_found",
  "message": "No subscription or payment records found for this external_user_id"
}
```

---

#### `GET /api/v1/users/:external_user_id/data-export`

Data Subject Access Request (DSAR) export to comply with GDPR Article 15. Returns a JSON export of all data `pay.tydecode.com` holds on the requested user. The client app is responsible for forwarding the user's request to this endpoint and aggregating the result with its own app-level data before returning it to the user.

**Request**:
```http
GET /api/v1/users/clerk_abc123/data-export
```

**Response** `200`:
```json
{
  "external_user_id": "clerk_abc123",
  "export_date": "2026-03-18T12:00:00Z",
  "subscriptions": [
    {
      "subscription_id": "premium_monthly",
      "provider": "google_play",
      "status": "active"
    }
  ],
  "payments": [
    {
      "id": "txn_uuid",
      "amount_cents": 299,
      "currency": "USD",
      "created_at": "2026-03-18T10:00:00Z"
    }
  ]
}
```

---

## Agent Micropayments (402)

### `GET /api/v1/agent/balance`

Get an agent's current balance and lifetime spend stats.

**Query Parameters**:
| Field | Type | Required | Description |
|---|---|---|---|
| `external_user_id` | string | yes | Opaque user ID (e.g., agent email) |

**Response** `200`:
```json
{
  "external_user_id": "agent@example.com",
  "balance_cents": 1500,
  "lifetime_spent_cents": 3000
}
```

**Response** `404` — agent not found:
```json
{
  "external_user_id": "agent@example.com",
  "balance_cents": 0,
  "lifetime_spent_cents": 0
}
```

---

### `POST /api/v1/agent/token`

Generate a scoped, short-lived (10-minute) one-time payment token. 
This is used to gate concurrent deductions and lock prices before content generation execution.

**Request**:
```json
{
  "external_user_id": "agent@example.com",
  "endpoint": "story",
  "amount_cents": 300,
  "nonce": "unique_request_guid"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `external_user_id` | string | yes | Opaque user ID |
| `endpoint` | string | yes | Target endpoint (e.g., `story`, `joke`) |
| `amount_cents` | int | yes | Amount to charge |
| `nonce` | string | yes | Client-generated nonce to prevent duplicate token creation |

**Response** `201`:
```json
{
  "token_id": "uuid_v4_token",
  "amount_cents": 300,
  "expires_at": "2026-03-19T21:10:00Z"
}
```

---

### `POST /api/v1/agent/charge`

Atomically consume a token and deduct the locked amount from the agent's balance in a single transaction.

**Request**:
```json
{
  "external_user_id": "agent@example.com",
  "token_id": "uuid_v4_token"
}
```

**Response** `200`:
```json
{
  "charged": true,
  "amount_cents": 300,
  "new_balance_cents": 1200
}
```

**Response** `400` — token invalid, expired, or balance insufficient:
```json
{
  "error": "insufficient_funds",
  "message": "Token cannot be consumed (expired, already used, or balance insufficient)"
}
```

---

### `POST /api/v1/agent/topup`

Directly credit an agent's balance (e.g., for response to external flow, support adjustments, or internal tooling).
*(Standard crypto top-ups trigger automatically via Coinbase Commerce webhook ingress).*

**Request**:
```json
{
  "external_user_id": "agent@example.com",
  "amount_cents": 1000,
  "charge_id": "optional_reference_id"
}
```

**Response** `200`:
```json
{
  "credited": true,
  "new_balance_cents": 2500
}
```

---

## Webhook Ingress (Providers → Pay)

### `POST /webhooks/{webhook_ingress_token}/:provider`

Receives webhooks from payment providers. Each app gets a unique `webhook_ingress_token` (UUID v4) — the URL is unguessable by random clients.

Examples:
- `POST /webhooks/a1b2c3d4-e5f6-7890-abcd-ef1234567890/google_play` — Google Play RTDN for hiha
- `POST /webhooks/a1b2c3d4-e5f6-7890-abcd-ef1234567890/creem` — Creem webhooks for hiha
- `POST /webhooks/f9e8d7c6-b5a4-3210-fedc-ba9876543210/apple` — Apple Server Notifications for future-app

Bridge resolves the app from the `webhook_ingress_token`. **Security Note**: This obfuscated path is merely the first defense. Bridge MUST explicitly verify the payload using provider-specific cryptographic authentication (HMAC signature, JWT, etc.) with credentials stored in the `apps` table to prevent impersonation.

**Not rate limited.** Protection relies on obfuscated paths, explicit provider signature verification, and webhook deduplication.

**Response**: `204 No Content` (acknowledged) or `400`/`401` (rejected).

---

## Webhook Callbacks (Bridge → Apps)

After processing a provider webhook, Bridge forwards a **normalized event** to the app's `webhook_callback_url`.

### Callback Format

```
POST <app.webhook_callback_url>
Content-Type: application/json
X-Pay-Signature: sha256=<HMAC of body>
X-Pay-Timestamp: 1711000000
X-Pay-Event-Id: evt_uuid
```

### Callback Payload

**Subscription Event Example**:
```json
{
  "event_id": "evt_uuid",
  "event_type": "subscription.activated",
  "app_slug": "hiha",
  "external_user_id": "clerk_abc123",
  "provider": "google_play",
  "subscription_id": "premium_monthly",
  "product_id": "premium_monthly",
  "status": "active",
  "current_period_end": "2026-04-18T00:00:00Z",
  "amount_cents": 299,
  "auto_renewing": true,
  "purchase_token": "token_xxx",
  "timestamp": "2026-03-18T10:05:00Z",
  "timestamp_epoch_ms": 1711000700000
}
```

**One-Time Purchase Event Example**:
```json
{
  "event_id": "evt_uuid",
  "event_type": "purchase.one_time",
  "app_slug": "hiha",
  "external_user_id": "clerk_abc123",
  "provider": "creem",
  "product_id": "otp_product_id",
  "status": "completed",
  "current_period_end": null,
  "amount_cents": 9999,
  "auto_renewing": false,
  "purchase_token": "token_xxx",
  "timestamp": "2026-03-18T10:05:00Z",
  "timestamp_epoch_ms": 1711000700000
}
```

**Callback Payload Fields**:
| Field | Type | Required | Description |
|---|---|---|---|
| `event_id` | string (UUID) | ✅ | Unique event ID for idempotency |
| `event_type` | string | ✅ | One of the event types from the table above |
| `app_slug` | string | ✅ | App identifier (e.g., "hiha") |
| `external_user_id` | string | ✅ | Opaque user ID from the app |
| `provider` | string | ✅ | Payment provider (google_play, creem, apple, lemonsqueezy) |
| `subscription_id` | string | ✅ | Product/subscription ID; same for both subscription and one-time events |
| `product_id` | string | ✅ | **App-defined product identifier** — required for app to activate correct product tier (e.g., "premium_monthly", "otp_lifetime") |
| `status` | string | ✅ | Subscription status (active, trial, past_due, on_hold, paused, cancelled, revoked, expired, completed) |
| `current_period_end` | string \| null | ❌ | ISO 8601 end date. **Null for one-time purchases.** |
| `amount_cents` | int | ✅ | Transaction amount in cents |
| `auto_renewing` | bool | ❌ | Whether subscription auto-renews. Null/false for one-time purchases. |
| `purchase_token` | string | ❌ | Purchase token (for recovery/restore logic) |
| `timestamp` | string | ✅ | ISO 8601 format for human readability |
| `timestamp_epoch_ms` | number | ✅ | Unix epoch milliseconds (used internally for ordering, deduplication, and event sequencing) |
| `revocation_reason` | string | ❌ | Present if status="revoked"; value: "refund", "chargeback", "fraud", etc. |
| `new_price_cents` | int | ❌ | Present if event_type="subscription.price_step_up"; new price after increase |
| `previous_status` | string | ❌ | Present if event_type="reconciliation.drift_detected"; the status pay had before correction |
| `corrected_status` | string | ❌ | Present if event_type="reconciliation.drift_detected"; the status corrected to (from provider API poll) |
| `reconciliation_source` | string | ❌ | Present if event_type="reconciliation.drift_detected"; value: "google_play", "apple", etc. — provider that was polled |

**Key Notes on One-Time Purchases**:
- `event_type` = `purchase.one_time`
- `status` = `completed` (not subscription statuses)
- `current_period_end` = `null` (no renewal)
- `auto_renewing` = `false` (or omitted)
- No further webhook events for this purchase (unless refunded via `payment.refunded`)

### Event Types

| Event | When | Payload Details |
|---|---|---|
| `subscription.activated` | New subscription or renewal confirmed | subscription_id, status (active/trial), current_period_end |
| `subscription.renewed` | Subscription renewed for a new period | subscription_id, status, current_period_end |
| `subscription.trial_started` | Free trial activated (no payment yet) | subscription_id, status="trial", current_period_end (trial end date) |
| `subscription.expired` | Subscription period ended without renewal | subscription_id, status="expired" |
| `subscription.cancelled` | User or system cancelled (may still have access until period end) | subscription_id, status="cancelled", current_period_end |
| `subscription.revoked` | Access revoked immediately (refund, chargeback, fraud) | subscription_id, status="revoked", revocation_reason |
| `subscription.paused` | Subscription paused | subscription_id, status="paused" |
| `subscription.resumed` | Subscription resumed from pause | subscription_id, status="active", current_period_end |
| `subscription.grace_period` | Payment failed, grace period started (user retains access) | subscription_id, status="past_due", current_period_end |
| `subscription.on_hold` | Account hold after grace period expired (access revoked) | subscription_id, status="on_hold" |
| `subscription.recovered` | Payment recovered from grace/hold state | subscription_id, status="active", current_period_end |
| `subscription.price_step_up` | Price change requires user consent (Google Play) | subscription_id, status (unchanged), new_price_cents |
| `payment.succeeded` | Payment received | subscription_id, product_id, amount_cents |
| `payment.failed` | Payment attempt failed | subscription_id, product_id, amount_cents, failure_reason |
| `payment.refunded` | Payment refunded | subscription_id, product_id, amount_cents |
| `purchase.one_time` | One-time purchase completed | subscription_id (product_id), status="completed", current_period_end=null |
| `reconciliation.drift_detected` | Reconciliation found state mismatch, corrected | subscription_id, previous_status, corrected_status, reconciliation_source |

**Reconciliation Drift Event Example**:
```json
{
  "event_id": "evt_uuid",
  "event_type": "reconciliation.drift_detected",
  "app_slug": "hiha",
  "external_user_id": "clerk_abc123",
  "provider": "google_play",
  "subscription_id": "premium_monthly",
  "product_id": "premium_monthly",
  "previous_status": "active",
  "corrected_status": "expired",
  "reconciliation_source": "google_play",
  "status": "expired",
  "current_period_end": "2026-03-15T00:00:00Z",
  "amount_cents": 299,
  "timestamp": "2026-03-18T12:00:00Z",
  "timestamp_epoch_ms": 1711016400000
}
```

### Callback Delivery

- Bridge expects `2xx` response from the app within **10 seconds**
- On failure: Implement a simple 3-strike retry rule. A background job (e.g., every 5 mins) will retry failed deliveries up to 3 times before naturally "dead-lettering" them.
- Failed callbacks visible in admin dashboard or via DB query.
- No strict ordering guarantees (at-least-once delivery).
- `X-Pay-Event-Id` and `timestamp` enable app-side idempotency and discarding of stale events.

### App Response

**Success** — `200`:
```json
{
  "received": true
}
```

**Failure** — any non-2xx: Bridge will retry.

---

## API Versioning & Deprecation

### Current Version: `/api/v1/`

All endpoints are scoped under `/api/v1/`. This allows Bridge to introduce breaking changes in a future `/api/v2/` without disrupting existing apps.

### Backward Compatibility Guarantee

Within `/api/v1/`:
- **Additive changes** (new optional fields, new event types) — no warning required
- **Breaking changes** (removing fields, changing semantics, renaming endpoints) — **NOT allowed** without proper deprecation

### Deprecation Process (if needed in the future)

1. **Announcement Phase** (2 weeks):
   - `Deprecation-Sunset: RFC7231 date` header on all responses
   - Webhook events include `api_version_deprecated` warning field
   - Dashboard alerts for affected apps

2. **Grace Period** (4 weeks):
   - Old endpoint still works
   - Errors/warnings logged per app

3. **Cutover** (sunset date):
   - Calls to old endpoint return `410 Gone` or redirect to new endpoint

### Migration Path

Apps should:
1. Monitor response headers for `Deprecation-Sunset`
2. Test against new `/api/v2/` endpoints in staging (once available)
3. Update integration before cutover date

---

## Error Format

All errors follow a consistent structure:

```json
{
  "error": "error_code",
  "message": "Human-readable description"
}
```

### Common Error Codes

| HTTP Status | Error Code | Description |
|---|---|---|
| `400` | `bad_request` | Invalid request body or parameters |
| `400` | `provider_not_configured` | Provider not set up for this app |
| `400` | `invalid_purchase_token` | Token verification failed |
| `401` | `unauthorized` | Missing or invalid API key |
| `403` | `app_disabled` | App is disabled in Bridge |
| `404` | `subscription_not_found` | Subscription doesn't exist for this app + user |
| `409` | `fraud_detected` | Purchase token already claimed by different user |
| `429` | `rate_limit_exceeded` | Too many requests for this API key |
| `500` | `internal_error` | Unexpected server error |
| `502` | `provider_error` | Payment provider returned an error |

---

## Rate Limiting

Per API key, enforced by Bridge on `/api/v1/*` endpoints only.

| Endpoint group | Limit |
|---|---|
| Checkout | 20 req/min |
| Verify purchase | 20 req/min |
| Subscription queries | 100 req/min |
| Subscription mutations (cancel/resume) | 10 req/min |
| Payment history | 100 req/min |
| Purchase registration | 20 req/min |

Default overall limit per API key: `api_rate_limit_per_minute` from `apps` table (default: 120). Per-endpoint limits above are applied within that budget utilizing dynamic overrides from the `api_rate_limit_rules` JSONB config.

Unauthenticated / failed auth requests: 10 req/min per IP.

**Exempt from rate limiting:**
- `GET /health`
- `/webhooks/{token}/:provider` (protected by obfuscated paths + signature verification instead)

Rate limit headers returned on all API responses:

```
X-RateLimit-Limit: 100
X-RateLimit-Remaining: 95
X-RateLimit-Reset: 1711000060
```

