# Bridge Implementation Gap Analysis #2

Continuation of `gap_analysis_1.md`. More discrepancies found between the API contract, architecture docs, PRD, and the current implementation.

---

## 3. `get_subscription` DB Query Ignores `provider` AND `external_user_id` (Critical)

**Contract:** `GET /api/v1/subscriptions/:subscription_id?external_user_id=clerk_abc123&provider=google_play` — both `external_user_id` and `provider` are **required** query params. The DB constraint is `UNIQUE (app_id, external_user_id, subscription_id, provider)`.

**Implementation (`db/subscriptions.rs:25-39`):** The `get_subscription()` function queries only by `app_id + subscription_id`:
```sql
SELECT * FROM pay.subscriptions WHERE app_id = $1 AND subscription_id = $2
```
It ignores both `provider` and `external_user_id` at the query level. The handler (`handlers/subscriptions.rs:110-114`) calls this and then does a post-hoc `if sub.external_user_id != query.external_user_id` check — but `provider` is never checked at all.

**Problem:** If two users have the same `subscription_id` on different providers (the exact scenario the contract warns about), this query returns **whichever row the DB finds first** (non-deterministic with `fetch_optional` on multi-row result). Wrong user could see someone else's subscription.

---

## 4. `GetSubscriptionQuery` Missing `provider` Field (Contract Violation)

**Contract:** `provider` is a **required** query parameter on the single-subscription endpoint.

**Implementation (`handlers/subscriptions.rs:93-96`):**
```rust
pub struct GetSubscriptionQuery {
    pub external_user_id: String,
}
```
No `provider` field exists. The handler silently ignores this required parameter.

---

## 5. `verify-purchase` Missing `purchase_token` Field (Contract Violation)

**Contract:** `POST /api/v1/verify-purchase` requires a `purchase_token` field — it's the whole point of the endpoint (verify a mobile store token).

**Implementation (`handlers/verify_purchase.rs:13-17`):**
```rust
pub struct VerifyPurchaseRequest {
    pub external_user_id: String,
    pub provider: String,
    pub subscription_id: String,
}
```
No `purchase_token` field. The endpoint creates a subscription in "pending" status but **never actually verifies anything with the provider**. The `_provider_config` is loaded but unused. This is a stub, not a verification endpoint.

---

## 6. Checkout Missing `email`, `product_type`, and `idempotency_key` Fields

**Contract:** `POST /api/v1/checkout` requires `email` and supports optional `product_type` and `idempotency_key`.

**Implementation (`handlers/checkout.rs:13-18`):**
```rust
pub struct CheckoutRequest {
    pub external_user_id: String,
    pub provider: String,
    pub product_id: String,
}
```
Missing: `email`, `product_type`, `idempotency_key`. The handler also doesn't delegate to any provider (the `_provider_config` is loaded but unused) — it fabricates a fake `redirect_url` from the app's own URL + product_id, which is completely wrong. The checkout is supposed to create a session *with the provider*.

---

## 7. Checkout Returns 201, Contract Says 200

**Contract:** Checkout response is `200 OK`.

**Implementation (`handlers/checkout.rs:71`):** Returns `StatusCode::CREATED` (201).

---

## 8. Error Response Format Mismatch

**Contract:** All errors use:
```json
{"error": "error_code", "message": "Human-readable description"}
```
Error codes are lowercase snake_case: `bad_request`, `unauthorized`, `provider_not_configured`, etc.

**Implementation (`error.rs:9-14, 154-158`):**
```rust
pub struct ErrorResponse {
    pub error: String,    // contains the human-readable message
    pub code: String,     // SCREAMING_CASE like "VALIDATION_ERROR", "DB_ERROR"
    pub details: serde_json::Value,
}
```
Three problems:
1. The `error` field contains the message text, not the error code
2. The `code` field uses `SCREAMING_CASE` instead of `snake_case`
3. There's an extra `details` field not in the contract

---

## 9. Health Check Missing `version` Field

**Contract:**
```json
{"status": "healthy", "version": "1.0.0"}
```

**Implementation (`handlers/mod.rs:10-12`):**
```rust
axum::Json(serde_json::json!({"status": "healthy"}))
```
No `version` field.

---

## 10. Webhook Ingress Handlers Are Complete Stubs

**Architecture doc:** Webhook ingress must: (1) resolve app from token, (2) verify provider signature, (3) dedup via `webhook_provider` table, (4) process and normalize event, (5) create `webhook_delivery` record.

**Implementation (`webhooks/ingress.rs`):** All four handlers (`handle_google_play`, `handle_creem`, `handle_lemonsqueezy`, `handle_coinbase`) are identical stubs — they log the token and return `200 OK`. No app resolution, no signature verification, no dedup, no processing. The `_db` and `_body` parameters are unused.

Additionally, the handlers return `StatusCode::OK` (200) but the contract specifies `204 No Content` for acknowledged webhooks.

---

## 11. Canonical Webhook Payload Missing Required Fields

**Contract callback payload** has 14+ fields including: `app_slug`, `product_id`, `amount_cents`, `auto_renewing`, `purchase_token`, `timestamp` (ISO 8601), `timestamp_epoch_ms`.

**Implementation (`webhooks/processor.rs:23-34`):**
```rust
pub struct CanonicalWebhookPayload {
    pub event_id: String,
    pub event_type: String,
    pub timestamp: i64,           // should be ISO 8601 string
    pub app_id: String,           // contract calls this app_slug
    pub subscription_id: Option<String>,
    pub external_user_id: Option<String>,
    pub status: Option<String>,
    pub provider: String,
    pub provider_event_id: String,  // not in contract
}
```
Missing: `app_slug` (uses `app_id` instead), `product_id`, `amount_cents`, `auto_renewing`, `purchase_token`, `timestamp` as ISO string, `timestamp_epoch_ms`, `current_period_end`. Additionally `external_user_id` is always `None` (hardcoded TODO comment).

---

## 12. Event Normalization Mapping Diverges from Architecture Doc

**Architecture doc** defines specific canonical event names per provider event.

**Implementation vs. Architecture discrepancies:**

| Provider Event | Architecture Doc Canonical | Code Canonical |
|---|---|---|
| `SUBSCRIPTION_PURCHASED` (Google) | `subscription.activated` | `subscription.renewed` |
| `SUBSCRIPTION_ON_HOLD` (Google) | `subscription.on_hold` | `subscription.paused` |
| `SUBSCRIPTION_RESTORED` (Google) | not listed (may be `recovered`) | `subscription.renewed` |
| `subscription.created` (Creem) | not the same as `subscription.renewed` | `subscription.renewed` |
| `subscription.active` / `subscription.paid` (Creem) | `subscription.activated` | not mapped |
| `subscription.trialing` (Creem) | `subscription.trial_started` | not mapped |
| `subscription.past_due` (Creem) | `subscription.grace_period` | not mapped |
| `subscription.paused` (Creem) | `subscription.paused` | not mapped |

Multiple Creem events from the architecture doc are completely absent from the code normalization.

---

## 13. Agent `charge` Handler Still Returns Hardcoded `amount_cents: 0`

**Contract:** Response should return `"amount_cents": 300` (the actual charged amount from the token).

**Implementation (`handlers/agent.rs:91`):** Despite the `db::agent::charge_agent()` function properly consuming the token and deducting the correct amount, the handler throws away that info and returns `"amount_cents": 0`.

The fix is trivial — the token amount is available in `charge_agent()` but not returned to the handler. The DB function only returns `new_balance` (i32), not the token amount.

---

## 14. `api_key_auth` Middleware Injects `AppAuth` but Agent Handlers Expect `App`

**Middleware (`handlers/api_key.rs:39`):** Inserts `AppAuth { app_id }` into request extensions.

**Agent handlers (`handlers/agent.rs`):** Extract `Extension(app): Extension<App>` — expecting the full `App` struct, not `AppAuth`.

This means agent routes will panic at runtime with a missing extension error. All other handlers correctly use `Extension(auth): Extension<AppAuth>`.

---

## 15. `list_subscriptions` Uses `offset` Pagination, Contract Uses Cursor-based

**Contract:** `GET /api/v1/subscriptions` uses cursor-based pagination with `after` parameter and returns `{"pagination": {"has_more": true, "after": "cursor_token"}}`.

**Implementation:** Uses `limit`/`offset` pagination and returns `{"total": N, "limit": N, "offset": N}` — completely different pagination model.

---

## 16. No Rate Limiting Implementation

**Contract & Architecture:** Detailed per-endpoint rate limits (checkout: 20/min, subscriptions: 100/min, etc.), per-IP rate limiting for unauthenticated requests (10/min), rate limit response headers (`X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`).

**Implementation:** Zero rate limiting exists anywhere in Bridge. No middleware, no in-memory store, no rate limit headers.

---

## 17. Admin Routes Have No Authentication

**Architecture doc:** Admin UI is "secured by Tyde's internal Clerk organization."

**Implementation (`main.rs:87-90`):** Admin routes (`/admin`, `/admin/apps`, etc.) are mounted directly on the root router with no middleware — completely unauthenticated. Anyone can view all apps and their webhook status.

---

## 18. No Reconciliation Background Job

**Architecture doc (Section 6.3):** A background job runs every 24 hours polling Google Play/Apple for active subscriptions, detecting drift, and triggering corrective callbacks.

**Implementation:** Only the webhook retry worker exists (`webhooks/scheduler.rs`). No reconciliation job.

---

## 19. `anonymize` Doesn't Cancel Subscriptions via Provider API

**Architecture doc (Section 10.3):** "Bridge instantly cancels any active auto-renewing subscriptions via Google Play/Apple/Provider APIs."

**Implementation (`db/users.rs:23-36`):** Sets `status = 'cancelled'` in the database but never calls the provider's cancel API. The user's subscription at Google/Apple/Creem would continue to auto-renew and charge money.

---

## 20. Webhook Forwarding Signature Format Mismatch

**Contract:** `X-Pay-Timestamp` contains a Unix epoch seconds integer (e.g., `1711000000`).

**Implementation (`webhooks/forwarding.rs:55`):** Uses `Utc::now().timestamp_millis()` — epoch *milliseconds*, not seconds. The HMAC signing message format (`payload.timestamp`) also concatenates with a dot, which isn't specified in the contract.

---

## 21. `topup` Handler Race Condition

**Implementation (`handlers/agent.rs:96-124`):** The `upsert_agent_credit` and `record_agent_transaction` calls happen in separate transactions. The credit is upserted first, then a new transaction is opened for the transaction record. If the server crashes between these two operations, the credit is added but no audit trail exists.

---

## 22. Missing `fraud_prevention` Table Operations

**Architecture doc (Section 3.10):** `fraud_prevention` table exists for purchase token → user binding validation to prevent token theft.

**Implementation:** The `anonymize_user` function references `pay.fraud_prevention` but no other code reads from or writes to this table. No fraud detection occurs during `verify-purchase` or webhook processing. The `409 fraud_detected` error code from the contract is never triggered.

---

## Summary

| # | Severity | Gap |
|---|---|---|
| 3 | 🔴 Critical | `get_subscription` DB query ignores `provider` + `external_user_id` |
| 4 | 🔴 Critical | `GetSubscriptionQuery` missing `provider` field |
| 5 | 🔴 Critical | `verify-purchase` missing `purchase_token`, never verifies |
| 6 | 🟠 High | Checkout missing `email`, `product_type`, `idempotency_key` |
| 7 | 🟡 Low | Checkout returns 201 instead of 200 |
| 8 | 🟠 High | Error response format completely wrong |
| 9 | 🟡 Low | Health check missing `version` |
| 10 | 🔴 Critical | All webhook ingress handlers are stubs |
| 11 | 🟠 High | Canonical webhook payload missing most required fields |
| 12 | 🟠 High | Event normalization diverges from architecture doc |
| 13 | 🟡 Medium | Agent charge returns hardcoded `amount_cents: 0` |
| 14 | 🔴 Critical | Agent handlers extract wrong extension type (runtime panic) |
| 15 | 🟡 Medium | Pagination model (offset vs cursor) doesn't match contract |
| 16 | 🟠 High | No rate limiting at all |
| 17 | 🟠 High | Admin routes completely unauthenticated |
| 18 | 🟡 Medium | No reconciliation background job |
| 19 | 🔴 Critical | Anonymize doesn't cancel via provider API (keeps charging) |
| 20 | 🟡 Low | Webhook timestamp millis vs seconds mismatch |
| 21 | 🟡 Medium | Topup has split-transaction race condition |
| 22 | 🟡 Medium | Fraud prevention table unused |
