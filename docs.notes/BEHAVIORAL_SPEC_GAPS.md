# Bridge Behavioral Spec Gap Review

Date: 2026-04-01
Updated: 2026-04-04
Source spec: `docs.notes/BEHAVIORAL_SPEC.md`
Codebase reviewed: `src/`, `migrations/`

## Scope

This document compares the current Bridge implementation against the behavioral spec and focuses on real behavior in code, not intent or TODO comments.

Status labels:

- `Fixed`: current code now matches the relevant spec requirement for this item.
- `Partial`: the main path exists, but required behaviors, fields, or safety checks are missing.
- `Gap`: missing, broken, or contradicted by the current implementation.

## Executive Summary

The main gaps are in correctness and contract fidelity:

- `verify-purchase` is now broadly at spec with full recording and linking implementation.
- Previous schema/runtime mismatches across handlers have been resolved or found to be inaccurate for v0.1.2.
- Webhook contract fidelity is improved: unresolved webhooks are suppressed, order/retry safety (event-time guards) is verified.
- The remaining security-sensitive gap: agent charge flow still does not bind the request endpoint back to the token as spec section 41 describes.


### 1. High-risk security gaps

- Partial: `src/db/agent.rs::charge_agent` now scopes token consumption to the same app and user, but the API still does not accept or verify the request `endpoint` against the token as required by spec section 41.


## Section Review

### Rate Limiting

| Spec area | Status | Notes |
|---|---|---|
| 3.3 Per-IP unauthenticated limits | Gap | No middleware exists for failed-auth or unauthenticated per-IP limits. |


### Core API Flows

| Spec area | Status | Notes |
|---|---|---|
| 4. Checkout Flow | Gap | `email` is optional with fake email fallback, Google Play mobile checkout not implemented, Coinbase is rejected, metadata/redirect handling not aligned with spec. |
| 7. Subscription Queries | Gap | Single-item response does not return provider-specific fields the spec calls for. |
| 8. Subscription Cancellation | Gap | Uses JSON body `external_user_id` instead of query params, ignores provider disambiguation, missing revocation metadata for immediate cancel. |
| 9. Subscription Resume | Gap | Body-based user lookup (should be query param), no provider query param. |
| 10. Billing Portal | Gap | Only works where `provider_customer_id` exists, only implemented for Creem. |

### Webhook Ingress and Processing

| Spec area | Status | Notes |
|---|---|---|
| 12. Webhook Ingress | Gap | App-not-found errors not silent 404, provider header names differ, config from `provider_configs` not app-level secrets. |
| 13-31. Canonical webhook processing | Gap | Several flows simplified to status-only updates, do not persist all spec-mandated fields or reasons. |
| 32-37. Google Play special cases | Gap | Price-step-up accept/decline API handlers schema-mismatched. |
| 38. Callback Forwarding | Gap | No explicit 10-second timeout, no dead-letter state, retry reprocesses webhook instead of forward-only delivery. |

### Specific Webhook Gaps Worth Calling Out

- `src/webhooks/processor.rs` handles `subscription.updated` by writing raw status when present, but the spec expects a more controlled normalized update path.
- `src/webhooks/processor.rs` only uses the stale-event guard for some update paths. Activation and renewal still use the unguarded upsert helper.

### Agent 402

| Spec area | Status | Notes |
|---|---|---|
| 41. Token Charge | Partial | `src/db/agent.rs::charge_agent` now binds token use to the same app and user, but the request still does not carry or verify `endpoint`, so it is not fully at spec. |

### GDPR and Data Retention

| Spec area | Status | Notes |
|---|---|---|
| 44. User Anonymization | Gap | No separate app callback sent on anonymization. |
| 45. Data Export | Gap | Does not include webhook records, agent credits, or agent transactions. |

### DB Behaviors

| Spec area | Status | Notes |
|---|---|---|
| 52. Subscription Store/Activate | Gap | Lacks `last_event_time` guard on conflict updates. |