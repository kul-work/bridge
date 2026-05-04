# Audit: Bridge/HiHa Subscription Port Gaps

## Scope

This note captures the current audit state for the payment and subscription event logic port from the old `C:\share\hiha` monolith into the Tyde split architecture:

- `C:\share\tyde\bridge`
- `C:\share\tyde\hiha`

It focuses on lifecycle ownership, callback data flow, local UX state, and remaining behavioral drift.

## Confirmed Ownership Model

- `bridge` owns provider normalization, billing state, subscription/payment lifecycle processing, and lifecycle emails.
- `hiha` owns app-facing premium permissions, local UX state, app notifications, and frontend-facing subscription status behavior.

This ownership split is correct and matches the intended architecture.

## Confirmed Bridge-Side Gap

Bridge-owned lifecycle emails are still not live for:

- `payment.failed`
- `subscription.price_step_up`
- `subscription.deferred`

Bridge already has the email infrastructure and Google-specific email helpers, but the live webhook path does not currently invoke those user-facing email flows.

The root blocker remains the same as documented in `LEFTOVER-bridge-email-lifecycle-events.md`:

- Bridge does not yet have a durable tenant-scoped mapping of `app_id + external_user_id -> email`

Without that mapping, provider webhooks cannot reliably resolve recipient email for lifecycle events.

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

### 1. HiHa subscription status is reading the wrong shape

HiHa's live `GET /api/v1/subscription-status` implementation calls Bridge's list endpoint and reads it as though it were a flat single-subscription status payload.

What exists now:

- HiHa calls Bridge `GET /api/v1/subscriptions`
- Bridge returns `{ subscriptions, pagination }`
- HiHa tries to read top-level fields such as:
  - `subscription_status`
  - `google_requires_price_step_up_consent`
  - `google_pause_scheduled_at`
  - `payment_failure_notification`

Result:

- most of those fields are effectively unavailable in live HiHa behavior
- local `is_premium` fallback is doing most of the real work
- frontend lifecycle UX cannot reliably reflect Bridge state

This is not just an incomplete port. It is a live contract mismatch.

### 2. HiHa still depends on Bridge for premium resolution

Tyde HiHa's runtime premium guard still falls back to Bridge to determine subscription premium state when the local user flag is not enough.

That means HiHa is still partially treating Bridge as the read path for app-facing subscription state, even though the intended split says HiHa should derive app behavior from locally cached callback state.

### 3. HiHa callback ingestion is too thin

Bridge already forwards more lifecycle context than HiHa currently stores.

Bridge canonical callback payload already supports fields such as:

- `new_price_cents`
- `previous_status`
- `corrected_status`
- `reconciliation_source`
- `revocation_reason`
- `cancellation_mode`

But HiHa's current callback payload model and `webhook_callbacks` persistence only keep a minimal event row plus premium-side effects.

So even fields Bridge already forwards are currently being dropped by HiHa.

### 4. Bridge callback payload is still not rich enough for full HiHa UX parity

Bridge already extracts and persists these Google-specific fields internally:

- price step-up amount
- price step-up consent deadline
- pause scheduled timestamp
- deferred-until timestamp

But the canonical Bridge-to-HiHa callback payload still does not expose all of them.

In particular, HiHa still cannot reconstruct full old-style cache behavior from callbacks alone because Bridge callback payload does not currently include at least:

- `google_price_step_up_consent_deadline`
- `google_pause_scheduled_at`
- `google_deferred_until`

So even a stronger HiHa callback ingestion layer would still need Bridge payload expansion for full parity.

### 5. Reconciliation drift handling is specified but not really implemented in HiHa

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

That has three practical consequences:

1. HiHa cannot reliably surface old monolith-style lifecycle UX from its own DB.
2. Some missing behavior can be restored entirely in HiHa by expanding callback ingestion and local storage for fields Bridge already sends.
3. Full parity still requires Bridge changes so callback payloads include the remaining Google lifecycle fields.

## Recommended Implementation Split

### Phase 1: HiHa

Restore the app-local state layer.

Suggested work:

- add a local `subscription_cache` table or equivalent replacement
- expand Bridge callback ingestion beyond premium toggles
- persist callback-derived UX fields locally
- handle `reconciliation.drift_detected`
- move `GET /api/v1/subscription-status` to read local cache instead of Bridge's list endpoint
- remove Bridge dependency from app-facing premium/status UX paths where local callback state should be authoritative

This phase gives HiHa a real app-owned source of truth for frontend lifecycle behavior.

### Phase 2: Bridge

Complete the Bridge side of the split.

Suggested work:

- implement tenant-scoped user contact storage for lifecycle emails
- activate lifecycle emails for Bridge-owned events
- expand canonical callback payload to include the remaining Google lifecycle fields HiHa needs

This phase completes the missing Bridge responsibilities without moving email ownership back into HiHa.

## Risk Summary

If left as-is:

- Bridge-owned lifecycle emails remain unsent
- HiHa frontend cannot reliably show deferred / pause scheduled / price step-up UX
- reconciliation corrections may not reach local app state correctly
- the split architecture remains partially implemented, with HiHa still reading runtime subscription state from Bridge in places where local callback-driven state was the intended model

## Status

No code changes were applied as part of this audit summary.

This note is intended to serve as the current reference point before implementation work begins.
