# Bridge - Gap Analysis vs. Behavioral Specification

**Date**: 2026-03-30  
**Last Updated**: 2026-03-30 (post-critical-fix recheck)  
**Status**: **Medium Gaps Present**  
**Target Doc**: `docs.notes/BEHAVIORAL_SPEC.md`

---

## 1. Executive Summary

This audit was rerun against the current codebase (`src/`, `migrations/`) and compared to `docs.notes/BEHAVIORAL_SPEC.md` (sections 1-54 + appendices).

Current state is improved versus earlier audit. Core flows exist (auth middleware, checkout, verify, webhook ingress, forwarding, scheduler jobs, agent endpoints), and previously flagged critical security/callback wiring issues were fixed in code.

Main risk areas:

- Event normalization/handlers still cover only a subset of sections 13-37.
- Checkout and lifecycle APIs are still partially aligned with spec details (idempotency, cursor semantics, cancel/resume behavior).
- Coinbase webhook -> agent topup flow from sections 42-43 is not fully wired.
- Background jobs and callback protocol details remain partially aligned.

---

## 2. Findings by Severity


### Medium

| Spec Section | Feature | Current Gap |
| :--- | :--- | :--- |
| **Section 1** | Startup & Init | Missing production safeguard (`MOCK_EXTERNAL_APIS=true` panic in prod). Background workers always start (no `enable_background_jobs` gate). |
| **Section 4** | Checkout | Request model omits `product_type` and `idempotency_key`; no idempotent response cache; non-Creem providers still stubbed in endpoint implementation. |
| **Section 7 / Section 11** | Query Pagination | Subscription and payment list APIs are offset-based under the hood (subscription cursor is base64-encoded offset), not true keyset/cursor semantics as spec intent describes. |
| **Section 8 / Section 9** | Cancel / Resume | Cancel flow does not support `mode` (`immediate` vs `scheduled`) behavior from spec and does not forward callbacks. |
| **Sections 39-43** | Agent 402 + Coinbase | Base balance/token/charge exists, but Coinbase webhook -> agent topup flow from sections 42-43 is not wired (processor logs and exits for confirmed charge). |
| **Appendix A** | Event Mapping Completeness | `normalize_event_type` and handler match arms cover only a subset of canonical events; many sections 26 and 28-37 are missing. |
| **Appendix C** | Error Surface | Distinct errors such as `subscription_not_found` are not consistently returned as specified. |

### Low

| Spec Section | Feature | Current Gap |
| :--- | :--- | :--- |
| **Section 38** | Forwarding Protocol | Signature format uses `HMAC(payload + "." + timestamp)` rather than strict "HMAC over raw JSON body" statement in spec text. |
| **Sections 46-48** | Background Jobs | Job skeletons run and mutate DB, but reconciliation and scheduling flows do not consistently emit spec-defined callback events/admin alerts. |

---

## 3. Coverage Snapshot (Spec vs Implementation)

| Area | Status | Notes |
| :--- | :--- | :--- |
| **Implemented (mostly aligned)** | Partial | Section 3 base API-key rate limiting, Section 6 pending registration, Section 10 billing portal route, Section 49 cleanup framework, Sections 50/51/52 DB primitives, Section 54 health. |
| **Implemented (partially aligned)** | Partial | Sections 4-5, 7-9, 11-12, 13-25, 38-43, 46-48, 53. Critical issues previously identified in Sections 2/5/12/38/44/45 were addressed; medium/low deltas remain. |
| **Not yet implemented / not evidenced in code paths** | Gap | Large portions of sections 26-37 and complete behavior detail for sections 42-43 spec expectations. |

---

## 4. Priority Fix Order

1. **Startup and safety controls**: Section 1 (`MOCK_EXTERNAL_APIS` production guard, background-job enable gate).
2. **Checkout and API semantics**: Section 4 idempotency + request fields, Sections 7/11 true cursor semantics, Sections 8/9 cancel/resume mode + callbacks.
3. **Agent/Coinbase completion**: Sections 39-43 webhook-to-topup wiring and end-to-end 402 flow completion.
4. **Event surface completion**: Appendix A / sections 26-37 mappings and handlers.
5. **Behavior polish**: Appendix C error normalization, Section 38 signature protocol alignment, Sections 46-48 callback/admin alert completeness.
