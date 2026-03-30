# Bridge - Gap Analysis vs. Behavioral Specification

**Date**: 2026-03-30  
**Last Updated**: 2026-03-30  
**Status**: **Major Gaps Present**  
**Target Doc**: `docs.notes/BEHAVIORAL_SPEC.md`

---

## 1. Executive Summary

This audit was rerun against the current codebase (`src/`, `migrations/`) and compared to `docs.notes/BEHAVIORAL_SPEC.md` (sections 1-54 + appendices).

Current state is **not** "minor gaps only". Core flows exist (auth middleware, checkout, verify, webhook ingress, forwarding, scheduler jobs, agent endpoints), but there are still significant specification deltas in security, callback correctness, and full behavior coverage.

Main risk areas:

- API key auth does not follow spec hardening model (hash verification strategy, app-enabled gate, `last_used_at` update).
- Webhook verification is incomplete for Google Play and uses callback secret instead of provider secret in ingress handlers.
- Forwarding pipeline misses stale-event guard and strict retry semantics from section 38.
- Event normalization/handlers cover only a subset of sections 13-37.
- GDPR endpoints are wired with wrong extractor type (`Extension<App>` on API-key routes), so they are likely broken at runtime.

---

## 2. Findings by Severity

### Critical / High

| Spec Section | Feature | Current Gap |
| :--- | :--- | :--- |
| **§2** | API Key Authentication | Uses SHA256 equality check against stored hash, not bcrypt/argon2 compare of full key; no `apps.enabled` check -> `403 app_disabled`; no `api_keys.last_used_at` update. |
| **§12** | Webhook Ingress Verification | Google Play JWT signature verification is TODO-only warning. Ingress signature checks use `apps.webhook_callback_secret` instead of provider-specific webhook secrets from provider config. |
| **§38** | Callback Forwarding Guard | Missing stale-event suppression at forward time (`timestamp_epoch_ms < subscription.last_event_time` -> suppress + skip). |
| **§5** | Verify Purchase (Mobile) | Missing Google Play acknowledgement flow and callback forward after successful verify. |
| **§44 / §45** | GDPR Endpoints | `users` handlers require `Extension<App>` but API middleware injects `Extension<AppAuth>`; anonymize/data-export path is not aligned with active auth context and is likely runtime-failing. |

### Medium

| Spec Section | Feature | Current Gap |
| :--- | :--- | :--- |
| **§1** | Startup & Init | Missing production safeguard (`MOCK_EXTERNAL_APIS=true` panic in prod). Background workers always start (no `enable_background_jobs` gate). |
| **§4** | Checkout | Request model omits `product_type` and `idempotency_key`; no idempotent response cache; non-Creem providers still stubbed in endpoint implementation. |
| **§7 / §11** | Query Pagination | Subscription and payment list APIs are offset-based under the hood (subscription cursor is base64-encoded offset), not true keyset/cursor semantics as spec intent describes. |
| **§8 / §9** | Cancel / Resume | Cancel flow does not support `mode` (`immediate` vs `scheduled`) behavior from spec and does not forward callbacks. |
| **§39-43** | Agent 402 + Coinbase | Base balance/token/charge exists, but Coinbase webhook -> agent topup flow from §§42-43 is not wired (processor logs and exits for confirmed charge). |
| **Appendix A** | Event Mapping Completeness | `normalize_event_type` and handler match arms cover only a subset of canonical events; many sections §26 and §§28-37 are missing. |
| **Appendix C** | Error Surface | Distinct errors such as `subscription_not_found` / `app_disabled` are not consistently returned as specified. |

### Low

| Spec Section | Feature | Current Gap |
| :--- | :--- | :--- |
| **§38** | Forwarding Protocol | Signature format uses `HMAC(payload + "." + timestamp)` rather than strict "HMAC over raw JSON body" statement in spec text. |
| **§46-48** | Background Jobs | Job skeletons run and mutate DB, but reconciliation and scheduling flows do not consistently emit spec-defined callback events/admin alerts. |

---

## 3. Coverage Snapshot (Spec vs Implementation)

| Area | Status | Notes |
| :--- | :--- | :--- |
| **Implemented (mostly aligned)** | Partial | §3 base API-key rate limiting, §6 pending registration, §10 billing portal route, §49 cleanup framework, §50/§51/§52 DB primitives, §54 health. |
| **Implemented (partially aligned)** | Partial | §§4-5, 7-9, 11-12, 13-25, 38-43, 46-48, 53. |
| **Not yet implemented / not evidenced in code paths** | Gap | Large portions of §§26-37 and complete behavior detail for §§42-43 spec expectations. |

---

## 4. Priority Fix Order

1. **Security + ingress correctness**: §§2, 12, 38 (auth hardening, provider signature correctness, stale-forward guard).
2. **Mobile purchase lifecycle correctness**: §5 (ack + forward), §4 idempotency.
3. **Broken API paths**: §§44-45 handler auth wiring.
4. **Event surface completion**: Appendix A / §§26-37 mappings and handlers.
5. **Behavior polish**: error code normalization (Appendix C), cursor semantics, callback coverage in scheduled jobs.
