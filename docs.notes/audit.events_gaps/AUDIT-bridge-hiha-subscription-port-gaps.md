# Audit: Bridge/HiHa Subscription Port Gaps

## Scope

This note captures the current audit state for the payment and subscription event logic port from the old `C:\share\hiha` monolith into the Tyde split architecture:

- `C:\share\tyde\bridge`
- `C:\share\tyde\hiha`

It focuses on lifecycle ownership, callback data flow, local UX state, and remaining behavioral drift.

## Confirmed Ownership Model

- `bridge` owns provider normalization, billing state, subscription/payment lifecycle processing.
- `hiha` owns app-facing premium permissions, local UX state, app notifications, and frontend-facing subscription status behavior.

This ownership split is correct and matches the intended architecture.

## Concrete Runtime Drift

### 1. Reconciliation drift handling is specified but not really implemented in HiHa

`reconciliation.drift_detected` is documented as an event HiHa should use to correct local state when Bridge background reconciliation discovers a mismatch.

Bridge can already forward that event with:

- `previous_status`
- `corrected_status`
- `reconciliation_source`

But HiHa's live callback handler does not have an actual state-correction branch for `reconciliation.drift_detected`.

So reconciliation may be visible in callback logs, while the intended app-local correction behavior still does not happen.

## Risk Summary

If left as-is:

- reconciliation corrections may not reach local app state correctly

## Status

Updated 2026-05-08: Resolved issues related to HiHa-side caching and Bridge read-path authority removed after Option B implementation.

