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

## Confirmed HiHa-Side Gap

Tyde HiHa is missing the old monolith's local subscription cache behavior for Google-specific lifecycle state, especially:

- `google_requires_price_step_up_consent`
- `google_price_step_up_consent_deadline`
- `google_new_price_cents`
- `google_pause_scheduled_at`
- `google_deferred_until`
- `payment_failure_notification`

The old monolith kept this kind of state locally and used it to drive banners, consent prompts, acknowledgements, and status UX.

Tyde HiHa currently does not have that equivalent local cache layer implemented.

## Concrete Runtime Drift

### 1. HiHa still depends on Bridge for premium resolution

Tyde HiHa's runtime premium guard still falls back to Bridge to determine subscription premium state when the local user flag is not enough.

That means HiHa is still partially treating Bridge as the read path for app-facing subscription state, even though the intended split says HiHa should derive app behavior from locally cached callback state.

### 2. HiHa callback ingestion is too thin

Bridge already forwards more lifecycle context than HiHa currently stores.

Bridge canonical callback payload already supports fields such as:

- `new_price_cents`
- `previous_status`
- `corrected_status`
- `reconciliation_source`
- `revocation_reason`
- `cancellation_mode`
- `google_price_step_up_consent_deadline`
- `google_pause_scheduled_at`
- `google_deferred_until`

But HiHa's current callback payload model and `webhook_callbacks` persistence only keep a minimal event row plus premium-side effects.

So even fields Bridge already forwards are currently being dropped by HiHa.

### 3. Reconciliation drift handling is specified but not really implemented in HiHa

`reconciliation.drift_detected` is documented as an event HiHa should use to correct local state when Bridge background reconciliation discovers a mismatch.

Bridge can already forward that event with:

- `previous_status`
- `corrected_status`
- `reconciliation_source`

But HiHa's live callback handler does not have an actual state-correction branch for `reconciliation.drift_detected`.

So reconciliation may be visible in callback logs, while the intended app-local correction behavior still does not happen.

## Architectural Conclusion

The current drift is bigger than "a few missing callback branches."

Tyde HiHa is still missing the entire intended `subscription_cache` style role in the split architecture.

That has two practical consequences:

1. HiHa cannot reliably surface old monolith-style lifecycle UX from its own DB.
2. Missing behavior can be restored entirely in HiHa by expanding callback ingestion and local storage for fields Bridge already sends.

## Recommended Implementation Split

### Phase 1: HiHa

Restore the app-local state layer.

Suggested work:

- add a local `subscription_cache` table or equivalent replacement
- expand Bridge callback ingestion beyond premium toggles
- persist callback-derived UX fields locally
- handle `reconciliation.drift_detected`
- remove Bridge dependency from app-facing premium/status UX paths where local callback state should be authoritative

### Phase 2: Bridge

Complete the Bridge side of the split.

Suggested work:

- expand canonical API structs to include the remaining Google lifecycle fields (`google_cancellation_context`, etc.)

## Risk Summary

If left as-is:

- HiHa frontend cannot reliably show deferred / pause scheduled / price step-up UX
- reconciliation corrections may not reach local app state correctly
- the split architecture remains partially implemented, with HiHa still reading runtime subscription state from Bridge in places where local callback-driven state was the intended model

## Status

No code changes were applied as part of this audit summary.

This note is intended to serve as the current reference point before implementation work begins.
