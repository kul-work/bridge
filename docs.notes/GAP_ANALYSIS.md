# Bridge - Gap Analysis vs. Behavioral Specification

**Date**: 2026-03-30  
**Last Updated**: 2026-03-30 (post-leftover-fix recheck)  
**Status**: **Low/Medium Gaps Present**  
**Target Doc**: `docs.notes/BEHAVIORAL_SPEC.md`

---

## 1. Executive Summary

This audit was rerun against the current codebase (`src/`, `migrations/`) and compared to `docs.notes/BEHAVIORAL_SPEC.md` (sections 1-54 + appendices).

Current state is improved versus earlier audits. Core flows exist (auth middleware, checkout, verify, webhook ingress, forwarding, scheduler jobs, agent endpoints), and previously flagged startup/callback/security issues are fixed in code.

Main risk areas:

- Event normalization/handlers still cover only a subset of sections 13-37.
- Some lifecycle semantics are provider-partial (cancel mode behavior depends on provider support).
- Background jobs and callback protocol details remain partially aligned.

---

## 2. Findings by Severity

### Medium

| Spec Section | Feature | Current Gap |
| :--- | :--- | :--- |
| **Appendix A** | Event Mapping Completeness | `normalize_event_type` and handler match arms still cover only a subset of canonical events; multiple sections 26 and 28-37 remain missing. |
| **Section 8 / Section 9** | Cancel / Resume | `mode` is now accepted and passed downstream, but behavior parity is still provider-dependent (full support currently Creem-oriented). |

### Low

| Spec Section | Feature | Current Gap |
| :--- | :--- | :--- |
| **Sections 46-48** | Background Jobs | Reconciliation/scheduler jobs now emit callbacks and alerts in key paths, but not all spec-defined variants are evidenced. |

---

## 3. Coverage Snapshot (Spec vs Implementation)

| Area | Status | Notes |
| :--- | :--- | :--- |
| **Implemented (mostly aligned)** | Partial | Section 3 base API-key rate limiting, Section 6 pending registration, Section 10 billing portal route, Section 49 cleanup framework, Sections 50/51/52 DB primitives, Section 54 health. |
| **Implemented (partially aligned)** | Partial | Sections 4-5, 7-9, 11-12, 13-25, 38-43, 46-48, 53. Startup guards, checkout idempotency fields/cache, keyset pagination, cancel/resume callbacks, Coinbase topup wiring, forwarding signature format, and `subscription_not_found` error shape are now addressed. |
| **Not yet implemented / not evidenced in code paths** | Gap | Portions of sections 26-37 still need event-specific handler coverage and behavior-level parity. |

---

## 4. Priority Fix Order

1. **Event surface completion**: Appendix A / sections 26-37 mappings and handlers.
2. **Provider behavior parity**: Sections 8/9 cancel-mode semantics where providers differ.
3. **Background behavior polish**: Sections 46-48 callback/admin alert completeness.
