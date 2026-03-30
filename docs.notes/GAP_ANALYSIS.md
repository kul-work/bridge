# Bridge — Gap Analysis vs. Behavioral Specification

**Date**: 2026-03-30  
**Last Updated**: 2026-03-30  
**Status**: **In Progress**  
**Target Doc**: `docs.notes/BEHAVIORAL_SPEC.md`

---

## 1. Executive Summary

This document tracks gaps between the **Bridge** implementation and the **BEHAVIORAL_SPEC.md**.

Recent work closed the three biggest gaps: webhook processing now performs real DB mutations (subscription state transitions, payment recording with fraud guard), user resolution cascade (§53) is implemented, and webhook ingress returns 204 immediately with async background processing.

**Remaining critical work**: provider API calls for cancel/resume, rate limiting, and missing background jobs.

---

## 2. Feature-by-Feature Status

### ✅ Resolved

| Spec Section | Feature | Resolution |
| :--- | :--- | :--- |
| **§12** | Webhook Ingress — async processing | All 4 providers now return 204 immediately, spawn `tokio::spawn` for processing + forwarding |
| **§13-22** | Webhook Processing — state mutations | `processor.rs` routes by canonical event, calls `upsert_subscription_tx` / `update_subscription_status` / `record_payment_tx` |
| **§23-25** | Webhook Processing — payment events | Order created (pending), order failed, OTP purchased all record payments via atomic UPSERT |
| **§27** | Webhook Processing — refunds | Payment status updated + linked subscription revoked |
| **§50** | Payment Recording (DB) | Atomic UPSERT with fraud guard (`WHERE external_user_id = EXCLUDED.external_user_id`) |
| **§52** | Subscription Store/Activate (DB) | Atomic UPSERT with `ON CONFLICT`, version increment, `last_event_time`, `COALESCE` for purchase_token |
| **§53** | User Lookup Strategies | Full cascade: subscription_id → purchase_token (sub+payment) → Creem metadata → orphan guard |
| **§18/§19** | Paused/Resumed status guards | Guards check current status before allowing transition (active/trial→paused, paused→active) |

### 🟡 Partial / Minor Gaps

| Spec Section | Feature | Gap Details |
| :--- | :--- | :--- |
| **§1** | Startup & Init | Missing production safeguard `MOCK_EXTERNAL_APIS` panic. |
| **§2** | API Auth | Missing `api_keys.enabled` check, `apps.enabled` → 403, `last_used_at` update. SHA256 instead of bcrypt/argon2. |
| **§4** | Checkout Flow | No `idempotency_key` or `product_type` fields. Non-Creem providers are stubbed. |
| **§5** | Purchase Verify | Missing Google Play "Acknowledgement" (3-day rule) and forward-to-app callback after verify. |
| **§7** | Subscription Queries | Uses `offset`-based pagination in some endpoints instead of cursor-based. |
| **§12** | Webhook Ingress | Google Play JWT signature verification is TODO (logged as warning). |
| **§38** | Forward To App | Missing stale event guard at forward time (`timestamp_epoch_ms < subscription.last_event_time`). |
| **§39-43** | Agent 402 Flows | Basic flows work. Coinbase webhook → agent topup integration not fully wired. |
| **Appendix A** | Event Mapping | `normalize_event_type` covers ~15 events; spec defines 35+ canonical mappings (§26, §28-§37 events missing). |
| **Appendix C** | Error Codes | Missing `404 subscription_not_found`, `403 app_disabled` as distinct error variants. |

### 🔴 Critical — Not Yet Implemented

| # | Spec Section | Feature | Gap Details |
| :--- | :--- | :--- | :--- |
| 1 | **§3** | Rate Limiting | Middleware exists but **never wired** into router. Per-API-key limits, endpoint overrides, per-IP limiting — all missing. |
| 2 | **§8/§9** | Cancel/Resume — provider calls | Only updates local DB. **Does not call provider APIs** (Google Play, Creem, etc.) to actually cancel/resume. |
| 3 | **§10** | Billing Portal | Returns hardcoded URLs. Should call provider API for real portal URLs. |
| 4 | **§6** | Purchase Registration | Implemented as "manual grants" instead of creating a pending subscription placeholder. |
| 5 | **§46** | Reconciliation Job | Scaffolded but `verify_subscription_status` always returns "active" — no real provider polling. |
| 6 | **§47** | Price Step-Up Expiry Job | Background job **not spawned**. |
| 7 | **§48** | Pause Scheduler Job | Background job **not spawned**. |
| 8 | **§49** | Webhook Log Cleanup Job | Background job **not spawned**. |
| 9 | **§51** | Webhook Deduplication | Uses `(app_id, provider, provider_webhook_id)`. Missing secondary unique `(provider, purchase_token, event_type)`. |

---

## 3. Implementation Roadmap (Priority)

1. **[PRIORITY 1] Rate Limiting (§3)**: Wire existing middleware into `main.rs`. Add per-API-key + endpoint-specific overrides.
2. **[PRIORITY 1] Cancel/Resume provider calls (§8/§9)**: Add actual provider API calls before DB mutation.
3. **[PRIORITY 2] Background Jobs (§47-§49)**: Spawn price step-up expiry, pause scheduler, webhook cleanup.
4. **[PRIORITY 2] Reconciliation (§46)**: Replace dummy `verify_subscription_status` with real provider API calls.
5. **[PRIORITY 3] Remaining webhook events**: Add normalize mappings + handlers for §26, §28-§37 (OTP cancelled, pending purchase cancelled, disputes, Google Play-specific events).
6. **[PRIORITY 3] Security hardening**: Google Play JWT verification, `api_keys.enabled` check, `apps.enabled` → 403.
7. **[PRIORITY 3] Checkout improvements**: `idempotency_key`, `product_type`, wire non-Creem providers.
