# Bridge Implementation Gap Analysis #2

Continuation of `gap_analysis_1.md`. More discrepancies found between the API contract, architecture docs, PRD, and the current implementation.

---

## 7. Error Response Format Mismatch

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

## 8. Health Check Missing `version` Field

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

## 9. Webhook Ingress Handlers Are Complete Stubs

**Architecture doc:** Webhook ingress must: (1) resolve app from token, (2) verify provider signature, (3) dedup via `webhook_provider` table, (4) process and normalize event, (5) create `webhook_delivery` record.

**Implementation (`webhooks/ingress.rs`):** All four handlers (`handle_google_play`, `handle_creem`, `handle_lemonsqueezy`, `handle_coinbase`) are identical stubs — they log the token and return `200 OK`. No app resolution, no signature verification, no dedup, no processing. The `_db` and `_body` parameters are unused.

Additionally, the handlers return `StatusCode::OK` (200) but the contract specifies `204 No Content` for acknowledged webhooks.

---

## 10. Canonical Webhook Payload Missing Required Fields

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

## 11. Event Normalization Mapping Diverges from Architecture Doc

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

## 12. Agent `charge` Handler Still Returns Hardcoded `amount_cents: 0`

**Contract:** Response should return `"amount_cents": 300` (the actual charged amount from the token).

**Implementation (`handlers/agent.rs:91`):** Despite the `db::agent::charge_agent()` function properly consuming the token and deducting the correct amount, the handler throws away that info and returns `"amount_cents": 0`.

The fix is trivial — the token amount is available in `charge_agent()` but not returned to the handler. The DB function only returns `new_balance` (i32), not the token amount.

---

## 13. `list_subscriptions` Uses `offset` Pagination, Contract Uses Cursor-based

**Contract:** `GET /api/v1/subscriptions` uses cursor-based pagination with `after` parameter and returns `{"pagination": {"has_more": true, "after": "cursor_token"}}`.

**Implementation:** Uses `limit`/`offset` pagination and returns `{"total": N, "limit": N, "offset": N}` — completely different pagination model.

---

## 14. No Rate Limiting Implementation

**Contract & Architecture:** Detailed per-endpoint rate limits (checkout: 20/min, subscriptions: 100/min, etc.), per-IP rate limiting for unauthenticated requests (10/min), rate limit response headers (`X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`).

**Implementation:** Zero rate limiting exists anywhere in Bridge. No middleware, no in-memory store, no rate limit headers.

---

## 15. Admin Routes Have No Authentication

**Architecture doc:** Admin UI is "secured by Tyde's internal Clerk organization."

**Implementation (`main.rs:87-90`):** Admin routes (`/admin`, `/admin/apps`, etc.) are mounted directly on the root router with no middleware — completely unauthenticated. Anyone can view all apps and their webhook status.

---

## 16. No Reconciliation Background Job

**Architecture doc (Section 6.3):** A background job runs every 24 hours polling Google Play/Apple for active subscriptions, detecting drift, and triggering corrective callbacks.

**Implementation:** Only the webhook retry worker exists (`webhooks/scheduler.rs`). No reconciliation job.

---

## 17. Webhook Forwarding Signature Format Mismatch

**Contract:** `X-Pay-Timestamp` contains a Unix epoch seconds integer (e.g., `1711000000`).

**Implementation (`webhooks/forwarding.rs:55`):** Uses `Utc::now().timestamp_millis()` — epoch *milliseconds*, not seconds. The HMAC signing message format (`payload.timestamp`) also concatenates with a dot, which isn't specified in the contract.

---

## 18. `topup` Handler Race Condition

**Implementation (`handlers/agent.rs:96-124`):** The `upsert_agent_credit` and `record_agent_transaction` calls happen in separate transactions. The credit is upserted first, then a new transaction is opened for the transaction record. If the server crashes between these two operations, the credit is added but no audit trail exists.

---

## 19. Missing `fraud_prevention` Table Operations

**Architecture doc (Section 3.10):** `fraud_prevention` table exists for purchase token → user binding validation to prevent token theft.

**Implementation:** The `anonymize_user` function references `pay.fraud_prevention` but no other code reads from or writes to this table. No fraud detection occurs during `verify-purchase` or webhook processing. The `409 fraud_detected` error code from the contract is never triggered.

---

## Summary

| # | Severity | Gap |
|---|---|---|
| 7 | 🟠 High | Error response format completely wrong |
| 8 | 🟡 Low | Health check missing `version` |
| 9 | 🔴 Critical | All webhook ingress handlers are stubs |
| 10 | 🟠 High | Canonical webhook payload missing most required fields |
| 11 | 🟠 High | Event normalization diverges from architecture doc |
| 12 | 🟡 Medium | Agent charge returns hardcoded `amount_cents: 0` |
| 13 | 🟡 Medium | Pagination model (offset vs cursor) doesn't match contract |
| 14 | 🟠 High | No rate limiting at all |
| 15 | 🟠 High | Admin routes completely unauthenticated |
| 16 | 🟡 Medium | No reconciliation background job |
| 17 | 🟡 Low | Webhook timestamp millis vs seconds mismatch |
| 18 | 🟡 Medium | Topup has split-transaction race condition |
| 19 | 🟡 Medium | Fraud prevention table unused |
