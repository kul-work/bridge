# Bridge — Gap Analysis vs. Behavioral Specification

**Date**: 2026-03-30  
**Status**: **Draft / Audit Results**  
**Target Doc**: `docs.notes/BEHAVIORAL_SPEC.md`

---

## 1. Executive Summary

This document performs a gap analysis between the current **Bridge** microservice implementation and the **BEHAVIORAL_SPEC.md**. 

The implementation currently provides a solid foundation for routing and basic database interactions. However, it contains **critical functional gaps** where it updates local state (e.g., cancelling a subscription in the DB) without communicating with external payment providers (Google Play, Creem, etc.). Additionally, the **webhook processing system** is a skeleton that lacks the complex state transitions required by the specification.

---

## 2. Feature-by-Feature Gap Table

| Spec Section | Feature | Status | Gap Details |
| :--- | :--- | :--- | :--- |
| **§1** | Startup & Init | 🟡 Partial | Missing production safeguard `MOCK_EXTERNAL_APIS` panic. |
| **§2** | API Auth | 🟡 Partial | Missing `apps.enabled` check and `last_used_at` updates. Hashing strategy differs. |
| **§3** | Rate Limiting | 🔴 Missing | Completely unimplemented (both per-API-key and per-IP). |
| **§4** | Checkout Flow | 🔴 Missing | No `idempotency_key` or `product_type`. Non-Creem providers are stubbed. |
| **§5** | Purchase Verify | 🟡 Partial | Missing Google Play "Acknowledgement" (3-day rule) and forward-to-app callback. |
| **§6** | Purchase Register | 🔴 Broken | Implemented as "manual grants" instead of the required "pending placeholder" behavior. |
| **§7-11** | Queries & History | 🟡 Partial | Uses `offset`-based pagination instead of cursor-based (`after`). |
| **§8-10** | Actions (Cancel/Resume) | 🔴 Critical | **Does not call provider APIs**. Only updates local Bridge DB. |
| **§12** | Webhook Ingress | 🔴 Critical | No Google Play JWT signature verification. Processing is synchronous. |
| **§13-37**| Webhook Processing | 🔴 Critical | Skeleton only. No DB mutations (subscriptions/payments) for actual events. |
| **§38** | Forward To App | 🟡 Partial | HMAC message format and retry intervals differ from spec. |
| **§39-43**| Agent 402 Flows | 🟡 Partial | Basic balance/token/charge exists, but integration with webhooks is missing. |
| **§46-49**| Background Jobs | 🔴 Missing | Reconciliation is a dummy skeleton. Price step-up, pause, and cleanup jobs missing. |

---

## 3. Top Critical Deficiencies

### 🔴 Critical A: Provider API Synchronization
The most significant gap is that **Bridge is not currently acting as a bridge** for mutations. 
- When an app calls `/cancel` or `/resume`, Bridge updates its own database but **fails to notify the provider** (e.g., Google Play API). 
- This results in "split-brain" where Bridge thinks a subscription is cancelled, but the provider continues to charge the user.

### 🔴 Critical B: Webhook Processor Logic
The specification outlines ~25 discrete webhook scenarios (Activation, Grace Period, Hold, Pause, etc.). 
- The current `processor.rs` only "normalizes" the event name. 
- It **does not update the subscription state** or record the payment in the DB, effectively rendering the webhook system non-functional for state management.

### 🔴 Critical C: User Resolution Strategy (§53)
The spec requires complex user resolution (lookup by `subscription_id`, `purchase_token`, or `metadata`). 
- The current implementation lacks this logic, meaning webhooks from providers (which often don't include the consumer's `external_user_id`) cannot be mapped back to the correct user.

---

## 4. Infrastructure & Security Gaps

### Rate Limiting (§3)
The spec requires a sophisticated rate-limiting system:
- Per-API-Key: Default 120/min with endpoint-specific overrides (e.g., Checkout: 20/min).
- Per-IP: 10/min for unauthenticated requests.
- **Current status**: No rate limiting is implemented.

### Pagination (§7, §11)
The spec explicitly requests **cursor-based pagination** using an `after` token (base64 of the cursor).
- **Current status**: Most endpoints use traditional `offset/limit` pagination, which is less performant and prone to drift.

### Webhook Signature Verification (§12)
- **Google Play**: Marked as `TODO`. Verification against Google's public certificates is required for production.
- **HMAC**: Currently verifies using `app.webhook_callback_secret` instead of provider-specific secrets from `provider_configs`.

---

## 5. Background Job Gaps

| Job Name | Spec Ref | Current Status |
| :--- | :--- | :--- |
| **Webhook Retry** | §38.7 | Implemented, but uses different retry intervals (0, 5, 10 min). |
| **Reconciliation** | §46 | Started but **dummy implementation**. Always returns "active". |
| **Price Step-Up Expiry** | §47 | **Missing**. Required for Google Play price change flows. |
| **Pause Scheduler** | §48 | **Missing**. Required for Google Play subscription pausing. |
| **Webhook Log Cleanup** | §49 | **Missing**. Required for DB maintenance. |

---

## 6. Implementation Roadmap (Priority)

1. **[PRIORITY 1] Logic Synchronization**: Implement the actual provider API calls in `handlers/subscriptions_actions.rs`.
2. **[PRIORITY 1] Webhook Processor**: Implement the DB mutation logic for subscription state transitions in `webhooks/processor.rs`.
3. **[PRIORITY 2] User Resolution**: Implement the multi-strategy user lookup to correctly route webhooks to users.
4. **[PRIORITY 2] Security**: Implement Google Play JWT verification and HMAC provider-secret verification.
5. **[PRIORITY 3] Infrastructure**: Implement the Rate Limiting middleware and move to true cursor pagination.
