# Bridge — Gap Analysis vs. Behavioral Specification

**Date**: 2026-03-30  
**Last Updated**: 2026-03-30  
**Status**: **Minor Gaps Only**  
**Target Doc**: `docs.notes/BEHAVIORAL_SPEC.md`

---

## 1. Executive Summary

This document tracks gaps between the **Bridge** implementation and the **BEHAVIORAL_SPEC.md**.

All critical gaps identified in previous audits have been verified in the codebase and removed from this active tracking list. The remaining gaps are minor or provider-specific stubs. Rate limiting, webhook deduplication, background jobs, and atomic DB mutations are all fully operational.

---

## 2. Feature-by-Feature Status

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

