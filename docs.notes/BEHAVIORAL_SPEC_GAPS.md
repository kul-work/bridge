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

### Webhook Ingress and Processing

| Spec area | Status | Notes |
|---|---|---|
| 32-37. Google Play special cases | Fixed | Price-step-up accept/decline handlers now return contract-compliant payloads, set Google Play consent state consistently, and forward `subscription.price_step_up` with `new_price_cents`. |
| 38. Callback Forwarding | Partial | Explicit 10-second timeout exists and retries rebuild the canonical callback for redelivery, but there is still no explicit dead-letter state after 3 failed attempts. |
