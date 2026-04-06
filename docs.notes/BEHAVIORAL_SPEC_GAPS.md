# Bridge Behavioral Spec Gap Review

Date: 2026-04-01
Updated: 2026-04-06
Source spec: `docs.notes/BEHAVIORAL_SPEC.md`
Codebase reviewed: `src/`, `migrations/`

## Scope

This document compares the current Bridge implementation against the behavioral spec and focuses on real behavior in code, not intent or TODO comments.

Status labels:

- `Fixed`: current code now matches the relevant spec requirement for this item.
- `Partial`: the main path exists, but required behaviors, fields, or safety checks are missing.
- `Gap`: missing, broken, or contradicted by the current implementation.

## Section Review

### Core API Flows

| Spec area | Status | Notes |
|---|---|---|
| 9. Subscription Resume | Fixed | Resume now uses `external_user_id` and `provider` query params with an empty request body. |
| 10. Billing Portal | Fixed | Uses the stored `provider_customer_id` and supports Creem plus LemonSqueezy customer portals. |

### Webhook Ingress and Processing

| Spec area | Status | Notes |
|---|---|---|
| 13-31. Canonical webhook processing | Gap | Several flows simplified to status-only updates, do not persist all spec-mandated fields or reasons. |
| 32-37. Google Play special cases | Gap | Price-step-up accept/decline API handlers schema-mismatched. |
| 38. Callback Forwarding | Partial | Explicit 10-second timeout exists and retries rebuild the canonical callback for redelivery, but there is still no explicit dead-letter state after 3 failed attempts. |
