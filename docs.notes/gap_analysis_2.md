# Bridge Implementation Gap Analysis #2

Continuation of `gap_analysis_1.md`. More discrepancies found between the API contract, architecture docs, PRD, and the current implementation.

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
| 12 | 🟡 Medium | Agent charge returns hardcoded `amount_cents: 0` |
| 13 | 🟡 Medium | Pagination model (offset vs cursor) doesn't match contract |
| 16 | 🟡 Medium | No reconciliation background job |
| 17 | 🟡 Low | Webhook timestamp millis vs seconds mismatch |
| 18 | 🟡 Medium | Topup has split-transaction race condition |
| 19 | 🟡 Medium | Fraud prevention table unused |
