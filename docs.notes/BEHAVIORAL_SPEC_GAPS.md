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

## Section Review

### Rate Limiting

| Spec area | Status | Notes |
|---|---|---|
| 3.3 Per-IP unauthenticated limits | Gap | No middleware exists for failed-auth or unauthenticated per-IP limits. |


### Core API Flows

| Spec area | Status | Notes |
|---|---|---|
| 4. Checkout Flow | Gap | `email` is optional with fake email fallback, Google Play mobile checkout not implemented, Coinbase is rejected, metadata/redirect handling not aligned with spec. |
| 8. Subscription Cancellation | Gap | Uses JSON body `external_user_id` instead of query params, ignores provider disambiguation, missing revocation metadata for immediate cancel. |
| 9. Subscription Resume | Gap | Body-based user lookup (should be query param), no provider query param. |
| 10. Billing Portal | Gap | Only works where `provider_customer_id` exists, only implemented for Creem. |

### Webhook Ingress and Processing

| Spec area | Status | Notes |
|---|---|---|
| 13-31. Canonical webhook processing | Gap | Several flows simplified to status-only updates, do not persist all spec-mandated fields or reasons. |
| 32-37. Google Play special cases | Gap | Price-step-up accept/decline API handlers schema-mismatched. |
| 38. Callback Forwarding | Gap | No explicit 10-second timeout, no dead-letter state, retry reprocesses webhook instead of forward-only delivery. |

### Specific Webhook Gaps Worth Calling Out

- Fixed: `src/webhooks/processor.rs` handles `subscription.updated` by writing raw status when present, but the spec expects a more controlled normalized update path.
- Fixed: `src/webhooks/processor.rs` only uses the stale-event guard for some update paths. Activation and renewal still use the unguarded upsert helper.

### DB Behaviors

| Spec area | Status | Notes |
|---|---|---|
| 52. Subscription Store/Activate | Fixed | Lacks `last_event_time` guard on conflict updates. (Fixed: Added strict `last_event_time` guards to registration upsert and link replacements) |