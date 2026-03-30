# Bridge — Gap Analysis vs. Behavioral Specification

**Date**: 2026-03-30  
**Last Updated**: 2026-03-30  
**Status**: **Minor Gaps Only**  
**Target Doc**: `docs.notes/BEHAVIORAL_SPEC.md`

---

## 1. Executive Summary

This document tracks gaps between the **Bridge** implementation and the **BEHAVIORAL_SPEC.md**.

All critical gaps are now resolved. Rate limiting is wired with per-API-key + endpoint-specific overrides. Cancel/resume/portal call real provider APIs before updating local DB. Purchase registration creates pending subscription placeholders. All four background jobs are spawned (reconciliation with real provider polling, price step-up expiry, pause scheduler, webhook log cleanup). Webhook deduplication has secondary unique index on `(provider, purchase_token, event_type)`.

**Remaining work**: minor/partial gaps only (see below).

---

## 2. Feature-by-Feature Status

### ✅ Resolved

| Spec Section | Feature | Resolution |
| :--- | :--- | :--- |
| **§3** | Rate Limiting | Per-API-key middleware wired into router. Endpoint-group overrides from `apps.api_rate_limit_rules` JSONB. Hardcoded defaults per group (checkout 20, verify 20, queries 100, mutations 10, agent 60, etc.). |
| **§6** | Purchase Registration | Creates pending subscription placeholder via `upsert_pending_subscription` instead of manual grants. |
| **§8/§9** | Cancel/Resume — provider calls | `provider_api.rs` calls Creem/LemonSqueezy APIs before DB mutation. Google Play cancel logs pending. |
| **§10** | Billing Portal | Calls Creem `/customers/billing` API for real portal URLs. Requires `provider_customer_id`. |
| **§12** | Webhook Ingress — async processing | All 4 providers return 204 immediately, spawn `tokio::spawn` for processing + forwarding. |
| **§13-22** | Webhook Processing — state mutations | `processor.rs` routes by canonical event, calls `upsert_subscription_tx` / `update_subscription_status` / `record_payment_tx`. |
| **§18/§19** | Paused/Resumed status guards | Guards check current status before allowing transition (active/trial→paused, paused→active). |
| **§23-25** | Webhook Processing — payment events | Order created (pending), order failed, OTP purchased all record payments via atomic UPSERT. |
| **§27** | Webhook Processing — refunds | Payment status updated + linked subscription revoked. |
| **§46** | Reconciliation Job | Real provider polling via `provider_api::fetch_subscription_status` for Creem/LemonSqueezy. Drift detection updates DB. |
| **§47** | Price Step-Up Expiry Job | Spawned every 5 min. Auto-cancels subscriptions past consent deadline, clears flags. |
| **§48** | Pause Scheduler Job | Spawned every 25 min. Transitions scheduled pauses + cleans orphaned pending subscriptions (30 min). |
| **§49** | Webhook Log Cleanup Job | Spawned daily. Calls `cleanup_old_webhook_provider()`, `cleanup_expired_agent_tokens()`, `cleanup_purged_fraud_prevention()`. |
| **§50** | Payment Recording (DB) | Atomic UPSERT with fraud guard (`WHERE external_user_id = EXCLUDED.external_user_id`). |
| **§51** | Webhook Deduplication | Primary `(app_id, provider, provider_webhook_id)` + secondary partial unique index `(app_id, provider, purchase_token, event_type)`. Insert uses `ON CONFLICT DO NOTHING` for both. |
| **§52** | Subscription Store/Activate (DB) | Atomic UPSERT with `ON CONFLICT`, version increment, `last_event_time`, `COALESCE` for purchase_token. |
| **§53** | User Lookup Strategies | Full cascade: subscription_id → purchase_token (sub+payment) → Creem metadata → orphan guard. |

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

---

## 3. Implementation Roadmap (Priority)

1. **[PRIORITY 3] Remaining webhook events**: Add normalize mappings + handlers for §26, §28-§37 (OTP cancelled, pending purchase cancelled, disputes, Google Play-specific events).
2. **[PRIORITY 3] Security hardening**: Google Play JWT verification, `api_keys.enabled` check, `apps.enabled` → 403.
3. **[PRIORITY 3] Checkout improvements**: `idempotency_key`, `product_type`, wire non-Creem providers.
