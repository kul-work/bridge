# Bridge — Behavioral Specification

> Status: **Proposal / Under Review**  
> Source: Extracted from HiHa monolith codebase, filtered to Bridge-owned concerns only  
> Purpose: Step-by-step behavioral documentation for implementing Bridge (`pay.tydecode.com`)

---

## Document Purpose

This document captures **every behavioral action** that Bridge must perform. Each flow is a numbered sequence of discrete steps: guards, side effects, error paths, and DB mutations. Designed so an LLM can implement each flow independently without hallucinating missing behavior.

**Scope**: Only Bridge-owned concerns. App-specific logic (content generation, OpenAI, Clerk auth, user accounts, HTML pages) is excluded. Where Bridge interacts with apps, the boundary is explicitly marked.

**Key Adaptation**: The monolith stores subscription state directly in its own `users` table. In Bridge, that responsibility is replaced by **webhook callbacks to apps**. Every place the monolith updates `users.is_premium`, Bridge instead forwards a normalized event to the app's `webhook_callback_url`.

---

## Table of Contents

1. [Startup & Initialization](#1-startup--initialization)
2. [API Key Authentication](#2-api-key-authentication)
3. [Rate Limit System](#3-rate-limiting)
4. [Checkout Flow](#4-checkout-flow)
5. [Purchase Verification Flow (Mobile)](#5-purchase-verification-flow-mobile)
6. [Purchase Registration (Pre-Purchase)](#6-purchase-registration-pre-purchase)
7. [Subscription Queries](#7-subscription-queries)
8. [Subscription Cancellation](#8-subscription-cancellation)
9. [Subscription Resume](#9-subscription-resume)
10. [Billing Portal](#10-billing-portal)
11. [Payment History](#11-payment-history)
    - [Additional Subscription/User Endpoints](#111-additional-subscriptionuser-endpoints)
12. [Webhook Ingress (Provider → Bridge)](#12-webhook-ingress-provider--bridge)
13. [Webhook Processing — Subscription Activation](#13-webhook-processing--subscription-activation)
14. [Webhook Processing — Subscription Pending](#14-webhook-processing--subscription-pending)
15. [Webhook Processing — Grace Period](#15-webhook-processing--grace-period)
16. [Webhook Processing — Subscription Revoked](#16-webhook-processing--subscription-revoked)
17. [Webhook Processing — Subscription On Hold](#17-webhook-processing--subscription-on-hold)
18. [Webhook Processing — Subscription Paused](#18-webhook-processing--subscription-paused)
19. [Webhook Processing — Subscription Restarted](#19-webhook-processing--subscription-restarted)
20. [Webhook Processing — Cancellation Scheduled](#20-webhook-processing--cancellation-scheduled)
21. [Webhook Processing — Subscription Expired/Inactive](#21-webhook-processing--subscription-expiredinactive)
22. [Webhook Processing — Subscription Cancelled](#22-webhook-processing--subscription-cancelled)
23. [Webhook Processing — Order Created (Payment Pending)](#23-webhook-processing--order-created-payment-pending)
24. [Webhook Processing — Order Failed](#24-webhook-processing--order-failed)
25. [Webhook Processing — One-Time Product Purchased](#25-webhook-processing--one-time-product-purchased)
26. [Webhook Processing — One-Time Product Cancelled](#26-webhook-processing--one-time-product-cancelled)
27. [Webhook Processing — Purchase Voided (Refund)](#27-webhook-processing--purchase-voided-refund)
28. [Webhook Processing — Pending Purchase Cancelled](#28-webhook-processing--pending-purchase-cancelled)
29. [Webhook Processing — Dispute Created](#29-webhook-processing--dispute-created)
30. [Webhook Processing — Refund Created](#30-webhook-processing--refund-created)
31. [Webhook Processing — Subscription Updated](#31-webhook-processing--subscription-updated)
32. [Google Play-Specific: Price Step-Up Consent](#32-google-play-specific-price-step-up-consent)
33. [Google Play-Specific: Subscription Deferred](#33-google-play-specific-subscription-deferred)
34. [Google Play-Specific: Pause Scheduled](#34-google-play-specific-pause-scheduled)
35. [Google Play-Specific: Price Changed](#35-google-play-specific-price-changed)
36. [Google Play-Specific: Price Change Updated (Pending)](#36-google-play-specific-price-change-updated-pending)
37. [Google Play-Specific: Expired Voided](#37-google-play-specific-expired-voided)
38. [Webhook Callback Forwarding (Bridge → App)](#38-webhook-callback-forwarding-bridge--app)
39. [User Anonymization (GDPR)](#39-user-anonymization-gdpr)
40. [Data Export (GDPR)](#40-data-export-gdpr)
41. [Background Job: Reconciliation](#41-background-job-reconciliation)
42. [Background Job: Price Step-Up Expiry](#42-background-job-price-step-up-expiry)
43. [Background Job: Pause Scheduler](#43-background-job-pause-scheduler)
44. [Background Job: Webhook Provider Cleanup](#44-background-job-webhook-provider-cleanup)
45. [Payment Recording (DB Behavior)](#45-payment-recording-db-behavior)
46. [Webhook Deduplication (DB Behavior)](#46-webhook-deduplication-db-behavior)
47. [Subscription Store/Activate (DB Behavior)](#47-subscription-storeactivate-db-behavior)
48. [User Lookup Strategies (Webhook → User Resolution)](#48-user-lookup-strategies-webhook--user-resolution)
49. [Health Check](#49-health-check)

---

## 1. Startup & Initialization

**Trigger**: Application starts

1. Initialize tracing (stdout + daily rolling file log). Filter: `info` in production, `debug` in development.
2. Load config from environment variables for Bridge-owned runtime settings (port, database URL, environment, background job toggles, etc.). Provider credentials are loaded per-app from `pay.provider_configs` at request time.
3. **Production safeguard**: If `MOCK_EXTERNAL_APIS=true` in production → panic with error.
4. Connect to PostgreSQL (Bridge's own database, separate from any app DB).
5. No provider credentials are loaded at startup; provider config is fetched on demand for the active app/provider pair.
6. If `enable_background_jobs=true`: start background tasks (webhook retry, reconciliation, price step-up, pause scheduler, webhook log cleanup).
7. **Provider loading is dynamic, per-app**: Unlike the monolith (which loads providers at startup from env vars), Bridge loads provider credentials from `pay.provider_configs` on each request.
8. Build router:
   - **Public**: `GET /health`
   - **API** (`/api/v1/*`): all endpoints, authenticated via API key
   - **Webhook ingress** (`/webhooks/{token}/:provider`): per-app obfuscated paths, no auth (signature verification instead)
   - **Admin UI**: separate routes, secured by Tyde's internal Clerk instance
9. Serve on `0.0.0.0:{PORT}`.

---

## 2. API Key Authentication

**Applied to**: All `/api/v1/*` endpoints.

**Monolith equivalent**: Clerk JWT verification.  
**Bridge replacement**: API key from `Authorization: Bearer sk_hiha_xxxxx` header.

1. Extract `Authorization: Bearer <key>` header. Missing → `401 unauthorized`.
2. Extract `key_prefix` (first 8 chars, e.g., `sk_hiha_`).
3. Lookup in `api_keys` table by `key_prefix`. Not found → `401`.
4. Verify key hash (bcrypt/argon2 compare of full key against `key_hash`). Mismatch → `401`.
5. Check `api_keys.enabled = true`. Disabled → `401`.
6. Check `apps.enabled = true` (joined via `app_id`). Disabled → `403 app_disabled`.
7. Update `api_keys.last_used_at = NOW()`.
8. Resolve `app_id` and `api_key_id` from the key. All subsequent operations are scoped to this `app_id`.

---

## 3. Rate Limiting

### 3.1 Per-API-Key Rate Limiting

**Applied to**: All `/api/v1/*` endpoints (after auth).

1. Get `app.api_rate_limit_per_minute` (default 120) and `app.api_rate_limit_rules` (JSONB overrides).
2. Determine effective limit for this endpoint:
   - Check `api_rate_limit_rules` for endpoint-specific override (e.g., `{"checkout": 20, "subscription_queries": 100}`).
   - Fallback to `api_rate_limit_per_minute`.
3. Atomic UPSERT to in-memory rate limit store:
   - Bridge uses a thread-safe `static RateLimitStore` (HashMap + Mutex).
   - Identifiers are bucketed by `api:{api_key_id}:{group}`.
   - Window: 60 seconds (fixed).
4. If `current_usage >= limit` → `429 rate_limit_exceeded`.
5. Set response headers: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`.

### 3.2 Default Rate Limits Per Endpoint

| Endpoint Group | Default Limit |
|---|---|
| Checkout | 20 req/min |
| Verify purchase | 20 req/min |
| Subscription queries | 100 req/min |
| Subscription mutations (cancel/resume) | 10 req/min |
| Payment history | 100 req/min |
| Purchase registration | 20 req/min |
| Overall per API key | 120 req/min |

### 3.3 Per-IP Rate Limiting (Unauthenticated)

1. For failed auth attempts or unauthenticated requests: 10 req/min per IP.
2. Extract IP from `X-Forwarded-For` (first entry) → `X-Real-IP` → connection IP.

### 3.4 Exempt from Rate Limiting

- `GET /health`
- `/webhooks/{token}/:provider` (protected by obfuscated paths + signature verification instead)

---

## 4. Checkout Flow

**Endpoint**: `POST /api/v1/payment/checkout`
**Request body**:
```json
{
  "external_user_id": "clerk_abc123",
  "email": "user@example.com",
  "provider": "creem",
  "product_id": "premium_monthly",
  "product_type": "subscription",
  "idempotency_key": "uuid_v4"
}
```

1. Authenticate via API key → resolve `app_id`.
2. Check rate limit for `checkout` endpoint.
3. Validate required fields: `external_user_id`, `email`, `provider`, `product_id`.
4. Lookup provider credentials from `pay.provider_configs` for this `app_id` + `provider`. Not configured → `400 provider_not_configured`.
5. If `idempotency_key` provided: check cache/DB for existing response. If found → return cached response.
6. Initialize provider client with the config loaded for this app/provider pair.
7. Call provider API to create checkout session:
   - **Web providers** (Creem): provider returns a `redirect_url`.
   - **Mobile providers** (Google Play, Apple): provider returns `mobile_checkout_data` (SKU details for native SDK).
   - Pass `external_user_id` as metadata (for webhook resolution later).
   - Pass `email` as pass-through to provider API (never stored in Bridge DB).
   - Use app's `app_url` for success/cancel redirect URLs.
8. Cache response against `idempotency_key` if provided.
9. Return response with `checkout_id`, `redirect_url` (or null for mobile), `provider`.

**Note**: `amount_cents` is NOT in the request. Price authority is always the provider dashboard. Bridge records `amount_cents` later from provider webhooks/verification responses.

---

## 5. Purchase Verification Flow (Mobile)

**Endpoint**: `POST /api/v1/verify-purchase`  
**Request body**:
```json
{
  "external_user_id": "clerk_abc123",
  "provider": "google_play",
  "subscription_id": "premium_monthly",
  "purchase_token": "token_from_sdk",
  "product_type": "subscription"
}
```

1. Authenticate via API key → resolve `app_id`.
2. Check rate limit for `verify_purchase` endpoint.
3. Parse and validate: `external_user_id`, `provider`, `subscription_id`, `purchase_token` (non-empty), `product_type`.
4. Load provider credentials for this `app_id` + `provider`. Not found → `400 provider_not_configured`.
5. Determine `is_one_time_product`: true if `product_type == "one_time"`.
6. Call `provider.verify_token(purchase_token, subscription_id, product_type, external_user_id, Strict)`.
7. Handle verification result:
   - `LinkingRequired { obfuscated_account_id }` → return `200 { "status": "linking_required", "message": "..." }`.
     - Also calls `delete_pending_subscription()` to clean up placeholders for the current user.
   - `Success(details)` → continue.
8. **Resolved User Identification**:
   - If Google Play returns `resubscribe_obfuscated_account_id` → lookup original user by this ID.
   - If not found → return `LinkingRequired` with the resubscribe ID.
   - If hash of `external_user_id` != `obfuscated_account_id` (from provider) → return `LinkingRequired`.
9. **Fraud Check (Token Binding)**:
   - If `purchase_token` already exists in DB for a DIFFERENT user → return `LinkingRequired` (Google Play) or `409 FraudDetected` (others).
10. **Record payment** (all purchase types):
    - `transaction_id` = `purchase_token`.
    - `status` = `"pending"` if subscription status is Pending, else `"success"`.
    - `amount_cents` from provider response.
    - Atomic UPSERT to `payments` table (see [§45](#45-payment-recording-db-behavior)).
11. **If subscription**:
    - Begin DB transaction.
    - UPSERT to `subscriptions` table (see [§47](#47-subscription-storeactivate-db-behavior)).
    - Commit.
    - **Acknowledge with Google Play** (3-day rule):
      - Check `payments.acknowledged_at` for this `purchase_token`. If NULL:
        - Call `provider.acknowledge_purchase_idempotent(subscription_id, purchase_token, Subscription, external_user_id)`.
        - Set `payments.acknowledged_at = NOW()`.
    - **Forward callback to app**: `subscription.activated` event (see [§38](#38-webhook-callback-forwarding-bridge--app)).
12. **If OTP** (one-time product):
    - Record in `payments` table (already done in step 10).
    - Acknowledge with Google Play (same as step 11).
    - **Forward callback to app**: `purchase.one_time` event.
13. Return response:
    - `{ "status": "active", "subscription_id": "...", "current_period_end": "...", "auto_renewing": true, "amount_cents": 299, "is_new": true }`.
    - If status is `Pending` → return `200` with `status: "pending"` (app should retry later).

---

## 6. Purchase Registration (Pre-Purchase)

**Endpoint**: `POST /api/v1/purchase/register`  
**Request body**: `{ "external_user_id": "clerk_abc123", "provider": "google_play", "subscription_id": "premium_monthly" }`

1. Authenticate via API key → resolve `app_id`.
2. Check rate limit.
3. Validate required fields.
4. UPSERT to `subscriptions` table:
   - `app_id`, `external_user_id`, `subscription_id`, `provider`.
   - `status = "pending"`, `purchase_token = NULL`.
   - All other fields NULL.
5. Return `{ "status": "registered" }`.

**Purpose**: Creates a placeholder so webhooks can resolve the user before `verify_purchase` is called. Mobile purchase flow: register → Google Play SDK purchase → webhook arrives → verify_purchase.

---

## 7. Subscription Queries

### List All: `GET /api/v1/subscriptions?external_user_id=X&limit=20&after=cursor`

1. Authenticate → resolve `app_id`.
2. Check rate limit (`subscription_queries`).
3. Validate `external_user_id` (required).
4. Query `subscriptions` table: all rows for this `app_id` + `external_user_id`, ordered by `created_at DESC`.
5. Apply pagination: `limit` (default 20, max 100), cursor-based (`after` token).
6. Return subscriptions array with `pagination.has_more` and `pagination.after`.

### Get Specific: `GET /api/v1/subscriptions/:subscription_id?external_user_id=X&provider=Y`

1. Authenticate → resolve `app_id`.
2. Check rate limit.
3. Validate `external_user_id` (required) and `provider` (required for disambiguation).
4. Query `subscriptions` table: `WHERE app_id=$1 AND external_user_id=$2 AND subscription_id=$3 AND provider=$4`.
5. Not found → `404 subscription_not_found`.
6. Return full subscription details including provider-specific fields.

---

## 8. Subscription Cancellation

**Endpoint**: `POST /api/v1/subscriptions/:subscription_id/cancel?external_user_id=X&provider=Y`  
**Request body**: `{ "mode": "immediate"|"scheduled", "purchase_token": "..." }`

1. Authenticate → resolve `app_id`.
2. Check rate limit (`subscription_mutations`, 10/min).
3. Validate: `external_user_id`, `provider` (required query params).
4. Get subscription from DB for this `app_id` + `external_user_id` + `subscription_id` + `provider`. Not found → `404`.
5. Validate cancel mode: must be `"immediate"` or `"scheduled"`. Default: `"scheduled"`.
6. Load provider credentials from `apps` table.
7. **If Google Play**:
   - `purchase_token` required (from request body or DB lookup). Missing → `404`.
   - Call `provider.cancel_subscription_with_token(subscription_id, purchase_token)`.
8. **If other provider**:
   - Call `provider.cancel_subscription_with_mode(subscription_id, mode, on_execute)`.
9. **Update Bridge DB**:
   - If mode is `"immediate"`:
     - `UPDATE subscriptions SET status='cancelled', revoked_at=NOW(), revocation_reason='immediate_cancel'`.
   - If mode is `"scheduled"`:
     - `UPDATE subscriptions SET auto_renewing=false`.
     - Subscription remains active until `current_period_end`.
     - This is a soft transition: the row may still read as `active` until the effective end is reached.
10. **Forward callback to app**: `subscription.cancelled` event with `mode` info and `current_period_end` when available.
   - For scheduled cancels, `status` in the payload may still reflect the current DB lifecycle state. Apps should use `current_period_end` as the access cutoff.
11. Return `{ "status": "cancelled", "mode": "...", "subscription_id": "..." }`.

---

## 9. Subscription Resume

**Endpoint**: `POST /api/v1/subscriptions/:subscription_id/resume?external_user_id=X&provider=Y`

1. Authenticate → resolve `app_id`.
2. Check rate limit (`subscription_mutations`).
3. Get subscription from DB. Not found → `404`.
4. Load provider, call `provider.resume_subscription(subscription_id)`.
5. Update DB: `SET auto_renewing=true, status='active'`.
6. **Forward callback to app**: `subscription.resumed` event.
7. Return `{ "status": "active", "subscription_id": "..." }`.

---

## 10. Billing Portal

**Endpoint**: `POST /api/v1/subscriptions/:subscription_id/portal?external_user_id=X&provider=Y`

1. Authenticate → resolve `app_id`.
2. Check rate limit.
3. Get subscription from DB. Not found → `404`.
4. Get `provider_customer_id`. If NULL → `400` (provider doesn't support billing portal or customer not linked).
5. Load provider, call `provider.create_billing_portal(customer_id)`.
6. Return `{ "url": "..." }`.

---

## 11. Payment History

**Endpoint**: `GET /api/v1/payments?external_user_id=X&limit=20&after=cursor`

1. Authenticate → resolve `app_id`.
2. Check rate limit (`payment_history`).
3. Query `payments` table: `WHERE app_id=$1 AND external_user_id=$2`, ordered by `created_at DESC`.
4. Apply pagination.
5. Return array of `{ id, provider, provider_transaction_id, amount_cents, currency, status, subscription_id, created_at }`.

---

### 11.1 Additional Subscription/User Endpoints

These routed APIs are Bridge-owned behaviors and must be specified alongside the primary flows.

#### Acknowledge Subscription

**Endpoint**: `POST /api/v1/subscriptions/:subscription_id/acknowledge?external_user_id=X&provider=Y`

1. Authenticate -> resolve `app_id`.
2. Check rate limit (`subscription_mutations`).
3. Validate `external_user_id`, `provider`, and `subscription_id`.
4. Load the subscription scoped to `app_id` + `external_user_id` + `subscription_id` + `provider`. Not found -> `404`.
5. Require a known `purchase_token`. Missing -> `400 bad_request`.
6. Load provider config from `pay.provider_configs`.
7. Call provider acknowledgment API idempotently.
8. Mark the subscription/payment as acknowledged in Bridge DB.
9. Return acknowledgment status.

#### Price Step-Up Accept

**Endpoint**: `POST /api/v1/subscriptions/:subscription_id/price-step-up/accept?external_user_id=X&provider=google_play`

1. Authenticate -> resolve `app_id`.
2. Check rate limit (`subscription_mutations`).
3. Validate subscription belongs to this `app_id` + `external_user_id` + `provider`.
4. Only supported for Google Play subscriptions.
5. Clear stored price step-up consent flags/deadline in `subscriptions`.
6. Forward callback to app: `subscription.price_step_up` or equivalent informational event.
7. Return updated subscription status.

#### Price Step-Up Decline

**Endpoint**: `POST /api/v1/subscriptions/:subscription_id/price-step-up/decline?external_user_id=X&provider=google_play`

1. Authenticate -> resolve `app_id`.
2. Check rate limit (`subscription_mutations`).
3. Validate subscription belongs to this `app_id` + `external_user_id` + `provider`.
4. Only supported for Google Play subscriptions.
5. Cancel auto-renew / mark the price step-up as declined according to provider behavior.
6. Update Bridge DB with the declined/cancelled lifecycle state.
7. Forward callback to app: `subscription.cancelled` with price step-up decline reason.
8. Return updated subscription status.

#### Subscription Status Snapshot

**Endpoint**: `GET /api/v1/users/:external_user_id/subscription-status`

1. Authenticate -> resolve `app_id`.
2. Check rate limit (`subscription_queries`).
3. Query subscriptions for this `app_id` + `external_user_id`.
4. Compute app-facing premium/access snapshot from Bridge subscription lifecycle rows.
5. Return the snapshot without mutating state.

---

## 12. Webhook Ingress (Provider → Bridge)

**Endpoint**: `POST /webhooks/{token}/:provider`  
**No rate limiting. No API key auth.**

1. Extract `{token}` from URL path. This is the `webhook_ingress_token` (UUID).
2. Lookup `apps` table: `WHERE webhook_ingress_token = $1 AND enabled = true`. Not found → `404` (silent, no details).
3. Resolve `app_id` from the matched row.
4. Extract `:provider` from URL path (e.g., `google_play`, `creem`).
5. Load provider credentials from `pay.provider_configs` for this `app_id` + `provider`. Not configured or disabled → `400 provider_not_configured`.
6. Get signature header (provider-specific name):
   - Creem: `Webhook-Signature`
   - Google Play: `Authorization` (JWT Bearer)
7. Extract signature value. Missing → `400`.
8. Call `provider.verify_and_parse_webhook(body, signature, headers)`:
   - Verifies HMAC/JWT/RSA signature using provider-specific secret from `pay.provider_configs.config`.
   - Parses payload into normalized `WebhookEvent` struct.
   - On failure → `400` (webhook verification failed).
9. Get webhook ID: prefer `event.event_id` from payload, fallback to provider header. Missing both → `400`.
10. Parse raw body as JSON for audit storage.
11. **Atomic deduplication** (see [§46](#46-webhook-deduplication-db-behavior)):
    - `INSERT INTO webhook_provider (...) ON CONFLICT DO NOTHING`.
    - If duplicate → check status of existing webhook in `webhook_provider`.
    - If existing but not processed/delivered → **Resume processing/forwarding** in background.
    - Return `204 No Content` immediately.
12. **Return `204 No Content`** to provider immediately.
13. **Spawn async task** to process webhook event:
    - Route by `event.event_type` to appropriate handler (see §13-§37).
    - After processing: forward normalized event to app's `webhook_callback_url` (see [§38](#38-webhook-callback-forwarding-bridge--app)).

---

## 13. Webhook Processing — Subscription Activation

**Event types**: `order.completed`, `subscription.paid`, `subscription.created`, `subscription.recovered`

### Step 1: Resolve User

Resolve `external_user_id` from the webhook event (see [§48](#48-user-lookup-strategies-webhook--user-resolution)).

If no user found → log error, return (webhook discarded, provider won't retry since we already returned 204).

### Step 2: Enrich Webhook Event

If provider supports enrichment (Google Play):
- Call Google Play API with `purchase_token` to get `current_period_end`, `auto_renewing`, `amount_cents`, etc.
- Fill in missing fields on the `WebhookEvent`.

### Step 3: Persist (Single Transaction)

1. **Determine payment status**:
   - `"active"` → `"success"`, `"paid"` → `"success"`, `"trial"` → `"trial"`, `"expired"` → `"expired"`, `"cancelled"` → `"cancelled"`, else → `"pending"`.

2. **Determine if payment should be recorded**:
   - Creem: only record if `provider_transaction_id` is present AND `amount_cents > 0` (for `subscription.created`/`subscription.update`). Always record for other event types.
   - Other providers: always record.

3. **Call `apply_webhook_transition()`** (atomic DB update):
   - Uses `last_event_time` guard to prevent stale overwrites.
   - Updates specific fields based on event type (e.g., clearing grace period flags on activation).

4. **Post-commit: Resubscribe/Upgrade linking** (Google Play only):
   - If `purchase_token` present → detect upgrades/downgrades by checking existing active subscriptions for this user.
   - If old active subscription found with different `subscription_id` → mark it as `"replaced"`.

9. **Forward callback to app**: `subscription.activated` event (see [§38](#38-webhook-callback-forwarding-bridge--app)).

---

## 14. Webhook Processing — Subscription Pending

**Event type**: `subscription.pending`

1. Lookup `external_user_id` by `subscription_id` from `subscriptions` table.
2. If found:
   - **Google Play**: store pending state context with `event_time_millis` in subscription record.
   - Update `subscriptions.status = 'pending'`.
3. **Do NOT forward callback yet** — pending is an intermediate state. Wait for confirmed activation.

---

## 15. Webhook Processing — Grace Period

**Event type**: `subscription.grace_period`

1. Lookup `external_user_id` by `subscription_id`.
2. Begin transaction:
   ```sql
   UPDATE subscriptions SET status='past_due', google_grace_period_start=NOW(), google_grace_period_end=$grace_end
   WHERE app_id=$1 AND subscription_id=$2
   ```
3. Commit.
4. **Forward callback to app**: `subscription.grace_period` event.
   - App should retain user access during grace period.

---

## 16. Webhook Processing — Subscription Revoked

**Event type**: `subscription.revoked`

1. Lookup `external_user_id` by `subscription_id`. Not found → log error, return.
2. Extract `revocation_reason` from `cancel_reason` field (or event metadata).
3. **Idempotency check**: if payment already `"refunded"` in DB → skip.
4. Begin transaction:
   ```sql
   UPDATE subscriptions SET status='revoked', revoked_at=NOW(), revocation_reason=$reason
   WHERE app_id=$1 AND subscription_id=$2 AND status NOT IN ('expired','revoked')
   ```
5. Commit.
6. **Forward callback to app**: `subscription.revoked` event with `revocation_reason`.
   - App should revoke access immediately.

---

## 17. Webhook Processing — Subscription On Hold

**Event type**: `subscription.on_hold`

1. Lookup `external_user_id` by `subscription_id`.
2. Get `event_time_millis` (fallback to `now()`).
3. **Event-time guard**: only process if `event_time > subscription.last_event_time` (stale event protection).
4. Update: `SET status='on_hold', last_event_time=$event_time`.
5. **Forward callback to app**: `subscription.on_hold` event.
   - App should revoke access.

---

## 18. Webhook Processing — Subscription Paused

**Event type**: `subscription.paused`

1. Lookup `external_user_id` by `subscription_id`.
2. Begin transaction.
3. `SELECT ... FOR UPDATE` on subscription row (lock).
4. Guard: only process if current status is `"active"` or `"trial"`.
5. Guard: `event_time_millis > last_event_time` (stale event protection).
6. Update:
   ```sql
   SET status='paused', auto_renewing=false, google_paused_at=NOW(),
       google_subscription_state=4, last_event_time=$event_time
   ```
7. Commit.
8. **Forward callback to app**: `subscription.paused` event.
   - App should revoke access.

---

## 19. Webhook Processing — Subscription Restarted

**Event type**: `subscription.restarted`

1. Lookup `external_user_id` by `subscription_id`.
2. **Google Play**: enrich with `current_period_end` from Google API if missing.
3. Begin transaction, `SELECT ... FOR UPDATE`.
4. Guard: only if current status is `"paused"`.
5. Guard: `event_time > last_event_time`.
6. Update:
   ```sql
   SET status='active', auto_renewing=true, google_paused_at=NULL,
       google_subscription_state=0, last_event_time=$event_time
   ```
7. Commit.
8. **Forward callback to app**: `subscription.resumed` event.
   - App should grant access.

---

## 20. Webhook Processing — Cancellation Scheduled

**Event type**: `subscription.cancellation_scheduled`

1. Lookup `external_user_id` by `subscription_id`.
2. **Google Play**: store cancellation context with `event_time_millis`.
3. Update: `SET auto_renewing=false`. Subscription status remains active until the effective end.
4. **Forward callback to app**: `subscription.cancelled` event with `current_period_end`.
   - App should retain access until `current_period_end`.

---

## 21. Webhook Processing — Subscription Expired/Inactive

**Event type**: `subscription.expired`, `subscription.inactive`

1. Lookup `external_user_id` by `subscription_id`.
2. Begin transaction.
3. `UPDATE subscriptions SET status='expired' WHERE app_id=$1 AND subscription_id=$2`.
4. Commit.
5. **Forward callback to app**: `subscription.expired` event.
   - App should revoke access.

---

## 22. Webhook Processing — Subscription Cancelled

**Event type**: `subscription.cancelled`

1. Lookup user (see [§48](#48-user-lookup-strategies-webhook--user-resolution)).
2. Normalize status: `"trialing"→"trial"`, `"paid"/"active"/"completed"→"active"`, `"canceled"→"cancelled"`, etc.
3. **Google Play**: enrich with Google API (`current_period_end`, `auto_renewing`).
4. UPSERT subscription with the normalized lifecycle state from the provider event.
   - For scheduled cancellations, that may still be `active` until the effective end.
5. **Forward callback to app**: `subscription.cancelled` event with `current_period_end`.
   - App decides: retain access until `current_period_end` or revoke immediately.

---

## 23. Webhook Processing — Order Created (Payment Pending)

**Event type**: `order.created`

1. Lookup `external_user_id` by `subscription_id`. Not found → log warn, return.
2. Record payment with `status="pending"`:
   - Transaction ID: `subscription_id` for Creem, `webhook_id` for others.
3. **Forward callback to app**: `payment.pending` event (optional — app may ignore).

---

## 24. Webhook Processing — Order Failed

**Event type**: `order.failed`

1. Lookup `external_user_id` by `subscription_id`. Not found → log error, return (permanent failure, stop retries).
2. Record payment with `status="failed"`.
3. Set `payment_failure_notification=true` on subscription record (for admin/app awareness).
4. **Forward callback to app**: `payment.failed` event.
   - App handles user notification (email, in-app banner, etc.).

---

## 25. Webhook Processing — One-Time Product Purchased

**Event type**: `one_time_product.purchased`

### Google Play:
1. Lookup user by `purchase_token` in `payments` table.
2. Record payment (`status="success"`).
3. **Forward callback to app**: `purchase.one_time` event with `status="completed"`.

### Creem / Other:
1. Lookup user cascade: `metadata.user_id` → `purchase_token` in payments → email fallback.
2. Determine `transaction_id`: `purchase_token` → `subscription_id` → `event_id`.
3. Record payment (`status="success"`).
4. **Forward callback to app**: `purchase.one_time` event with `status="completed"`.
   - App grants whatever the OTP product means (e.g., lifetime premium).

---

## 26. Webhook Processing — One-Time Product Cancelled

**Event type**: `one_time_product.canceled`

1. Lookup user by `purchase_token` in `payments` table.
2. **Idempotency**: check if payment already `"refunded"` or `"canceled"` → skip.
3. Update payment status to `"cancelled"`.
4. **Forward callback to app**: `purchase.one_time` event with `status="cancelled"`.
   - App revokes whatever the OTP product granted.

---

## 27. Webhook Processing — Purchase Voided (Refund)

**Event type**: `purchase.voided`

### If `purchase_token` present:
1. **Idempotency**: `get_payment_status(token)`. If `"refunded"` → skip.
2. If payment exists → `update_payment_status(token, "refunded")`.
3. Try `get_subscription_by_token(token)`:
   - **Found (subscription)**: begin tx, `SET status='revoked', revocation_reason='REFUND'`, commit.
   - **Not found (OTP)**: lookup user cascade → mark payment as refunded.
4. **Forward callback to app**: `payment.refunded` event.
   - App revokes access.

### If no `purchase_token` but `subscription_id`:
- Lookup by subscription_id → revoke. Fallback to `metadata_user_id`.

### If only `metadata_user_id`:
- Forward `payment.refunded` event to app with the user identifier.

---

## 28. Webhook Processing — Pending Purchase Cancelled

**Event type**: `subscription.pending_purchase_canceled`

1. Lookup `external_user_id` by `subscription_id`.
2. Begin transaction.
3. `UPDATE subscriptions SET status='cancelled', revocation_reason='pending_purchase_canceled'`.
4. Commit.
5. **Forward callback to app**: `subscription.cancelled` event.

---

## 29. Webhook Processing — Dispute Created

**Event type**: `dispute.created`

1. Compose alert email with event details (event_id, amount, customer email).
2. Send to Bridge admin email / Tyde support.
3. No subscription status change.
4. **Forward callback to app**: a dispute notification so app can track it.

---

## 30. Webhook Processing — Refund Created

**Event type**: `refund.created`

1. Same processing as subscription cancelled handler.
2. **Forward callback to app**: `payment.refunded` event.

---

## 31. Webhook Processing — Subscription Updated

**Event type**: `subscription.update`

1. Lookup user (see [§48](#48-user-lookup-strategies-webhook--user-resolution)).
2. Normalize status: `"trialing"→"trial"`, `"paid"/"active"/"completed"→"active"`, `"canceled"→"cancelled"`, etc.
3. UPSERT subscription with normalized status.
4. **Forward callback to app**: appropriate event type based on new status.

---

## 32. Google Play-Specific: Price Step-Up Consent

**Event type**: `subscription.price_step_up_consent_updated`

1. Only process for `google_play` provider.
2. Lookup user by `subscription_id`.
3. Enrich with Google API: `new_price_cents`, `consent_status` (ACCEPTED/OUTSTANDING), `consent_deadline`.
4. If `consent_status == "pending"`:
   - Store consent requirement: `SET google_requires_price_step_up_consent=true, google_new_price_cents=$price, google_price_step_up_consent_deadline=$deadline`.
5. If accepted:
   - Clear consent flags.
6. **Forward callback to app**: `subscription.price_step_up` event with `new_price_cents`.

---

## 33. Google Play-Specific: Subscription Deferred

**Event type**: `subscription.deferred`

1. Lookup user by `subscription_id`.
2. If `current_period_end` (deferred_until) present:
   - `UPDATE subscriptions SET google_deferred_until=$deferred_until`.
3. **Forward callback to app**: informational event.

---

## 34. Google Play-Specific: Pause Scheduled

**Event type**: `subscription.pause_scheduled`

1. Lookup user by `subscription_id`.
2. If `current_period_end` missing → enrich from Google API.
3. Store: `UPDATE subscriptions SET google_pause_scheduled_at=$pause_date`.
4. Subscription remains `"active"` until pause date.
5. Background scheduler ([§43](#43-background-job-pause-scheduler)) handles actual transition.
6. **Forward callback to app**: informational event.

---

## 35. Google Play-Specific: Price Changed

**Event type**: `subscription.price_changed`

1. Lookup user by `subscription_id`.
2. Record payment with `status="price_changed"` (audit trail).
3. **Forward callback to app**: informational event with `new_price_cents`.

---

## 36. Google Play-Specific: Price Change Updated (Pending)

**Event type**: `subscription.price_change_updated`

1. Lookup user by `subscription_id`.
2. Log the pending price change.
3. **Forward callback to app**: informational event.

---

## 37. Google Play-Specific: Expired Voided

**Event type**: `subscription.expired_voided`

1. Log: "Subscription expired and voided".
2. No DB mutation. Informational only.
3. **Forward callback to app**: informational event (optional).

---

## 38. Webhook Callback Forwarding (Bridge → App)

**Triggered by**: Every webhook event handler after DB processing.

**This is a NEW behavior not in the monolith.** The monolith directly updates its own `users.is_premium`. Bridge instead forwards normalized events to apps.

### Forwarding Steps:

1. Load app's `webhook_callback_url` and `webhook_callback_secret` from `apps` table.
2. Build normalized callback payload:
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
3. Compute HMAC-SHA256 of raw JSON body using `webhook_callback_secret`.
4. Set headers:
   - `Content-Type: application/json`
   - `X-Pay-Signature: sha256=<hex_hmac>`
   - `X-Pay-Timestamp: <unix_epoch>`
   - `X-Pay-Event-Id: <event_id>`
5. POST to `webhook_callback_url`. Timeout: **10 seconds**.
6. If `2xx` response → mark as delivered in `webhook_delivery`.
7. If non-2xx or timeout:
   - Increment `forward_attempts` in `webhook_delivery`.
   - Log diagnostic payload (scrubbed/redacted: e.g., `purchase_token` partially masked).
   - **Retry policy**: background job retries up to 3 times (every 5 minutes).
   - After 3 failures → mark the `webhook_delivery` row as dead-lettered (visible in admin dashboard).

### Stale Event Guard (at forward time):

Before every forward attempt, re-check:
```
if webhook_provider.timestamp_epoch_ms < subscription.last_event_time →
    mark webhook_provider suppressed (reason: 'superseded_before_forward')
    skip forwarding
```

## 39. User Anonymization (GDPR)

**Endpoint**: `POST /api/v1/users/:external_user_id/anonymize`

**NEW behavior** (not in monolith — Bridge-specific).

1. Authenticate → resolve `app_id`.
2. Validate `external_user_id`.
3. **Cancel active subscriptions**: for each active subscription for this `app_id` + `external_user_id`:
   - Call provider API to cancel auto-renew.
   - Update subscription status in Bridge DB.
4. **Scramble identifiers**: replace `external_user_id` on all `subscriptions` and `payments` rows with hashed value (e.g., `deleted_<hash>`).
5. **Retain purchase tokens** (for "Restore Purchases" fraud prevention — per data retention policy).
6. **Retain payment records** (7-year tax/audit requirement — but with scrambled user ID).
7. Return `{ "anonymized": true, "subscriptions_cancelled": N }`.

**Note**: Bridge does NOT send separate callbacks for anonymization-triggered cancellations. The app already knows the outcome from this endpoint's synchronous response.

---

## 40. Data Export (GDPR)

**Endpoint**: `GET /api/v1/users/:external_user_id/data-export`

1. Authenticate → resolve `app_id`.
2. Query all Bridge data for this `app_id` + `external_user_id`:
   - `subscriptions` (all statuses)
   - `payments` (all records)
   - `webhook_provider` records related by subscription/payment identifiers, if within retention window
3. Return JSON export of all data.

---

## 41. Background Job: Reconciliation

**Frequency**: Every `RECONCILIATION_INTERVAL_MINUTES` (default 1440 = 24 hours).  
**Runs for**: Each enabled app with a supported provider config in `pay.provider_configs`.

1. For each enabled app in `apps`:
   - Load provider configs for that app from `pay.provider_configs`.
   - Query active subscriptions for each configured provider.
2. For each subscription:
   - Call the provider-specific status API using the provider config for that subscription.
   - Parse the returned period end / expiry data when available.
   - If the provider status differs from Bridge DB status:
     - Update the subscription to the provider-corrected status.
     - Insert an audit record in `webhook_provider`.
     - **Forward callback to app**: `reconciliation.drift_detected` event with `previous_status` and `corrected_status`.
     - Send admin alert email to Tyde support.
3. Errors on individual subscriptions are logged but don't fail the whole job.

---

## 42. Background Job: Price Step-Up Expiry

**Frequency**: Every 5 minutes (configurable).

1. Query: `SELECT * FROM subscriptions WHERE google_requires_price_step_up_consent=true AND google_price_step_up_consent_deadline < NOW()`.
2. For each: auto-cancel the subscription (same flow as cancellation).
3. **Forward callback to app**: `subscription.cancelled` with reason.

---

## 43. Background Job: Pause Scheduler

**Frequency**: Every 25 minutes (configurable).

### Pause Transition Check:
1. Query: `SELECT * FROM subscriptions WHERE google_pause_scheduled_at <= NOW() AND status != 'paused' LIMIT 100`.
2. For each:
   - `mark_subscription_paused()` in DB.
   - Sets `status='paused'`, `auto_renewing=false`, `google_paused_at=NOW()`.
   - **Forward callback to app**: `subscription.paused` event.

### Orphaned Pending Cleanup:
1. `DELETE FROM subscriptions WHERE status='pending' AND purchase_token IS NULL AND created_at < (NOW() - INTERVAL '30 minutes')`.
2. Cleans up `register_purchase` records where user never completed purchase.

---

## 44. Background Job: Webhook Provider Cleanup

**Frequency**: Daily (or configurable).

1. Delete old `webhook_provider` records older than the configured retention window, default 90 days.
2. Related `webhook_delivery` records are removed via foreign-key cascade.
3. Per data retention policy: raw webhook payloads are kept 90 days for debugging/reconciliation.

---

## 45. Payment Recording (DB Behavior)

**Atomic UPSERT with fraud detection.**

```sql
INSERT INTO payments (app_id, external_user_id, provider, provider_transaction_id, subscription_id, amount_cents, status, webhook_received_at)
VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
ON CONFLICT (app_id, provider, provider_transaction_id)
DO UPDATE SET
  status = EXCLUDED.status,
  subscription_id = COALESCE(EXCLUDED.subscription_id, payments.subscription_id),
  amount_cents = CASE WHEN EXCLUDED.amount_cents > 0 THEN EXCLUDED.amount_cents ELSE payments.amount_cents END,
  webhook_received_at = NOW()
WHERE payments.external_user_id = EXCLUDED.external_user_id  -- FRAUD GUARD
```

**Fraud detection**:
- If `ON CONFLICT` fires but `WHERE` clause prevents update (different `external_user_id`): `rows_affected=0` → `409 fraud_detected`.
- If unique constraint violation (23505) → `409 fraud_detected`.

---

## 46. Webhook Deduplication (DB Behavior)

```sql
INSERT INTO webhook_provider (app_id, provider, provider_webhook_id, event_type, payload, subscription_id, purchase_token, timestamp_epoch_ms)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT DO NOTHING
```

- `rows_affected=0` → duplicate/existing webhook. Fetch the existing `webhook_provider` row and decide whether to ignore, resume processing, or resume forwarding.
- `rows_affected=1` → new webhook. Process it asynchronously and mark `processed=true` only after handler completion.
- Primary dedupe is app-scoped: `(app_id, provider, provider_webhook_id)`.
- Do not dedupe by `(purchase_token, event_type)`: Google Play reuses purchase tokens across legitimate subscription renewals.
- Forwarding/retry/dead-letter state is tracked in `webhook_delivery`, not `webhook_provider`.

---

## 47. Subscription Store/Activate (DB Behavior)

UPSERT to `subscriptions` table:

```sql
INSERT INTO subscriptions (app_id, external_user_id, subscription_id, provider, status, current_period_end, purchase_token, ...)
VALUES (...)
ON CONFLICT (app_id, external_user_id, subscription_id, provider)
DO UPDATE SET
  status = EXCLUDED.status,
  current_period_end = EXCLUDED.current_period_end,
  purchase_token = COALESCE(EXCLUDED.purchase_token, subscriptions.purchase_token),
  auto_renewing = EXCLUDED.auto_renewing,
  payment_state = EXCLUDED.payment_state,
  ...
  version = subscriptions.version + 1,
  updated_at = NOW()
```

**Key constraints**:
- Unique on `(app_id, external_user_id, subscription_id, provider)`.
- `purchase_token` has a separate unique constraint (fraud: one token = one owner).
- `version` incremented on each update for optimistic concurrency.
- `last_event_time` used for stale event guards.

---

## 48. User Lookup Strategies (Webhook → User Resolution)

When a provider webhook arrives, Bridge resolves `external_user_id` via a cascade of strategies:

| Priority | Strategy | How | Used When |
|---|---|---|---|
| 1 (Google) | `purchase_token` | `SELECT external_user_id FROM subscriptions WHERE app_id=$1 AND purchase_token=$2` | Preferred for Google Play |
| 1 (Other) | `subscription_id` | `SELECT external_user_id FROM subscriptions WHERE app_id=$1 AND subscription_id=$2` | Renewal, existing subscription |
| 2A | `purchase_token` (subscription) | `SELECT external_user_id FROM subscriptions WHERE app_id=$1 AND purchase_token=$2` | New purchase after verify_purchase |
| 2B | `purchase_token` (payment) | `SELECT external_user_id FROM payments WHERE app_id=$1 AND provider_transaction_id=$2` | OTP webhook linking |
| 3 | `obfuscated_account_id` (Google Play) | Google API verify_token → extract obfuscated_id → `SELECT external_user_id FROM subscriptions WHERE app_id=$1 AND google_obfuscated_account_id=$2` | Resubscribe/Restore |
| 4 | `metadata.user_id` / `external_user_id` | From provider metadata/payload | Primary binding for some providers |
| 5 | Creem orphan guard | If Creem AND strategies 1-4 fail → log error, discard | Prevents accidental linking |
| 5 | `customer_email` | Not used in Bridge. Bridge has no user email table. Falls through to discard. | N/A in Bridge |

**If all strategies fail**: webhook is discarded (logged as error). Bridge has already returned 204 to the provider, so no retry impact.

**Key difference from monolith**: Strategy 5 (email fallback) does NOT exist in Bridge. Bridge does not store user emails — those are app-side. If strategies 1-4 fail, the webhook is orphaned and requires manual resolution via admin dashboard.

---

## 49. Health Check

**Endpoint**: `GET /health`  
**No auth. No rate limit.**

Returns:
```json
{
  "status": "healthy",
  "version": "1.0.0"
}
```

---

## Appendix A: Event Type to Handler Mapping

| Provider Event Type | Bridge Handler | Canonical Callback Event | Section |
|---|---|---|---|
| `order.completed` | subscription_activated | `subscription.activated` | §13 |
| `subscription.paid` | subscription_activated | `subscription.activated` | §13 |
| `subscription.created` | subscription_activated | `subscription.activated` | §13 |
| `subscription.recovered` | subscription_activated | `subscription.recovered` | §13 |
| `subscription.pending` | subscription_pending | (no callback) | §14 |
| `subscription.grace_period` | subscription_grace_period | `subscription.grace_period` | §15 |
| `subscription.revoked` | subscription_revoked | `subscription.revoked` | §16 |
| `subscription.on_hold` | subscription_on_hold | `subscription.on_hold` | §17 |
| `subscription.paused` | subscription_paused | `subscription.paused` | §18 |
| `subscription.restarted` | subscription_restarted | `subscription.resumed` | §19 |
| `subscription.cancellation_scheduled` | cancellation_scheduled | `subscription.cancelled` | §20 |
| `subscription.expired` | subscription_expired | `subscription.expired` | §21 |
| `subscription.inactive` | subscription_expired | `subscription.expired` | §21 |
| `subscription.cancelled` | subscription_cancelled | `subscription.cancelled` | §22 |
| `order.created` | order_created | `payment.pending` (optional) | §23 |
| `order.failed` | order_failed | `payment.failed` | §24 |
| `one_time_product.purchased` | otp_purchased | `purchase.one_time` | §25 |
| `one_time_product.canceled` | otp_canceled | `purchase.one_time` (cancelled) | §26 |
| `purchase.voided` | purchase_voided | `payment.refunded` | §27 |
| `subscription.pending_purchase_canceled` | pending_purchase_canceled | `subscription.cancelled` | §28 |
| `dispute.created` | dispute_created | (admin alert + app callback) | §29 |
| `refund.created` | refund_created | `payment.refunded` | §30 |
| `subscription.update` | subscription_updated | (varies by new status) | §31 |
| `subscription.price_step_up_consent_updated` | price_step_up | `subscription.price_step_up` | §32 |
| `subscription.deferred` | subscription_deferred | `subscription.deferred` | §33 |
| `subscription.pause_scheduled` | pause_scheduled | `subscription.pause_scheduled` | §34 |
| `subscription.price_changed` | price_changed | `subscription.price_changed` | §35 |
| `subscription.price_change_updated` | price_change_updated | `subscription.price_change_updated` | §36 |
| `subscription.expired_voided` | expired_voided | `subscription.expired_voided` | §37 |
| `google.test` | test_event (log only) | (none) | — |
| Unknown | log_unknown_event | (none) | — |

---

## Appendix B: Bridge Database Tables

| Table | Primary Key | Unique Constraints | Purpose |
|---|---|---|---|
| `apps` | UUID | `slug`, `webhook_ingress_token` | App registry, callback settings, rate limits |
| `api_keys` | UUID | `(app_id, key_hash)` | API key auth |
| `subscriptions` | UUID | `(app_id, external_user_id, subscription_id, provider)`, `(app_id, purchase_token)` | Subscription lifecycle |
| `payments` | UUID | `(app_id, provider, provider_transaction_id)` | Payment records |
| `provider_configs` | UUID | `(app_id, provider)` | Per-app provider credentials/settings |
| `webhook_provider` | UUID | `(app_id, provider, provider_webhook_id)` | Provider webhook dedup + audit |
| `webhook_delivery` | UUID | `(webhook_provider_id)` | Callback forwarding, retry, and dead-letter state |

---

## Appendix C: Error Codes

| HTTP Status | Error Code | When |
|---|---|---|
| 400 | `bad_request` | Invalid request body or parameters |
| 400 | `provider_not_configured` | Provider not set up for this app |
| 400 | `invalid_purchase_token` | Token verification failed |
| 401 | `unauthorized` | Missing or invalid API key |
| 403 | `app_disabled` | App is disabled in Bridge |
| 404 | `subscription_not_found` | Subscription doesn't exist for this app + user |
| 409 | `fraud_detected` | Purchase token already claimed by different user |
| 429 | `rate_limit_exceeded` | Too many requests for this API key |
| 500 | `internal_error` | Unexpected server error |
| 502 | `provider_error` | Payment provider returned an error |

---

## Appendix D: Key Behavioral Differences from Monolith

| Concern | Monolith (hiha) | Bridge |
|---|---|---|
| Auth | Clerk JWT per user | API key per app |
| User store | Own `users` table | No user table. Uses opaque `external_user_id`. |
| Premium status | `UPDATE users SET is_premium=...` | Forward callback event → app decides |
| Email notifications | Bridge sends directly | Bridge sends admin alerts only. App-facing notifications forwarded via callback. |
| Provider credentials | Environment variables | `pay.provider_configs` table (per-app) |
| Rate limiting | Per-user, DB-backed | Per-API-key, in-memory or DB-backed |
| Webhook endpoint | `/webhooks/:provider` | `/webhooks/{token}/:provider` (obfuscated per-app) |
| Webhook processing | Direct DB mutation | DB mutation + callback forwarding to app |
| Email lookup (strategy 5) | `SELECT clerk_id FROM users WHERE email=$1` | Not available. Bridge has no user emails. |
| Content generation | Inline (OpenAI) | Not in Bridge. App-side only. |
