# Migration Gap Audit: Old HiHa Payment Events vs Tyde Bridge + HiHa

Date: 2026-05-04

Scope:
- Old HiHa: `C:\share\hiha`
- Tyde Bridge: `C:\share\tyde\bridge`
- Tyde HiHa: `C:\share\tyde\hiha`

This is an investigation-only audit of payment, subscription, webhook, scheduler, email, local premium, and UX state behavior that existed in old HiHa and is missing, partial, or incorrectly split across Tyde Bridge and Tyde HiHa.

## Findings


### 2. payment.failed is only partially ported

Old locations:
- `C:\share\hiha\src\webhooks\events\payment.rs:63`
- `C:\share\hiha\src\webhooks\events\payment.rs:97`
- `C:\share\hiha\src\db\notifications.rs:111`

New expected owner:
- Bridge owns failed payment record and provider-side subscription flag.
- HiHa owns app-facing warning/acknowledgement UX unless Bridge exposes and clears it consistently.

Current Tyde locations:
- `C:\share\tyde\bridge\src\webhooks\processor\event_handlers.rs:666`
- `C:\share\tyde\bridge\src\db\subscriptions.rs:299`
- `C:\share\tyde\hiha\src\handlers\webhooks.rs:207`
- `C:\share\tyde\hiha\src\main.rs:147`

Missing behavior:
- HiHa logs the callback but does not create notification audit rows or store a local UX flag.
- Tyde HiHa does not register old `POST /api/v1/notifications/payment-failure/acknowledge`.

Impact:
- Users may not see a local actionable failed-payment warning.
- Users cannot acknowledge/clear payment-failure UX in HiHa.

Gap types:
- Missing HiHa local cache/state update.
- Missing endpoint/acknowledgment flow.
- Missing tests.



### 3. `subscription.price_step_up` is partial (HiHa routes missing)

**Status**: ✅ PARTIALLY RESOLVED (Bridge side complete)

Current Tyde locations:
- `C:\share\tyde\bridge\src\services\google_play\subscription_lifecycle.rs:244`
- `C:\share\tyde\bridge\src\db\subscriptions.rs:341`
- `C:\share\tyde\bridge\src\webhooks\scheduler.rs:290`

Missing behavior:
- No Tyde HiHa price-step-up accept/decline routes are registered.
- Decision needed: Should HiHa provide UI routes for accept/decline that call Bridge, or rely on the provider portal?

Impact:
- Users may not be able to act on price step-ups directly from the HiHa app.

Gap types:
- Missing endpoint/acknowledgment flow.

### 7. OTP refund/revoke flow is inconsistent

Old locations:
- `C:\share\hiha\src\webhooks\events\otp.rs:4`
- `C:\share\hiha\src\webhooks\events\otp.rs:128`
- `C:\share\hiha\src\webhooks\events\subscription\common.rs:450`
- `C:\share\hiha\src\services\google_play\product_lifecycle.rs:94`

New expected owner:
- Bridge records OTP payment/refund and forwards a clear OTP callback.
- HiHa grants/revokes local lifetime premium.

Current Tyde locations:
- `C:\share\tyde\bridge\src\services\google_play\product_lifecycle.rs:12`
- `C:\share\tyde\bridge\src\services\google_play\product_lifecycle.rs:55`
- `C:\share\tyde\bridge\src\webhooks\processor\event_handlers.rs:775`
- `C:\share\tyde\hiha\src\db\webhook_callbacks.rs:22`

Missing behavior:
- HiHa grants/revokes on `purchase.one_time` with statuses `completed`, `cancelled`, or `refunded`.
- Generic `payment.refunded` in HiHa always maps to subscription inactive, not OTP-specific lifetime revoke semantics.

Impact:
- OTP lifetime premium revocation can be missed if refund classification is generic.

Gap types:
- Missing tests.

### 8. Reconciliation drift is Bridge-owned but HiHa does not apply or surface drift fields

Old locations:
- `C:\share\hiha\src\schedule.rs:74`
- `C:\share\hiha\src\db\subscriptions.rs:1159`

New expected owner:
- Bridge owns reconciliation and corrective source-of-truth mutation.
- HiHa owns app-facing callback handling and UX/cache update.

Current Tyde locations:
- `C:\share\tyde\bridge\src\webhooks\scheduler.rs:118`
- `C:\share\tyde\bridge\src\webhooks\scheduler.rs:197`
- `C:\share\tyde\bridge\src\webhooks\scheduler.rs:428`
- `C:\share\tyde\hiha\src\handlers\webhooks.rs:14`

Missing behavior:
- HiHa callback payload struct omits those fields and has no event-specific handling for `reconciliation.drift_detected`.

Impact:
- Bridge correction is recorded and forwarded, but HiHa local/app state and UX do not reflect drift details.

Gap types:
- Missing HiHa local cache/state update.
- Missing tests.

## Event Checklist

### Fully or mostly ported (Bridge source-of-truth)

- `payment.refunded`: Bridge updates payment status and revokes matching subscription.
- `subscription.grace_period`: Bridge sets status `past_due` and grace timestamps.
- `subscription.on_hold`: Bridge sets status `on_hold` and payment failure flag.
- `subscription.paused`: Bridge sets status `paused`.
- `subscription.resumed`: Bridge transitions paused/cancelled to active.
- `subscription.cancelled`: Bridge stores cancellation and context (including survey feedback).
- `subscription.expired`: Bridge sets expired.
- `subscription.revoked`: Bridge sets revoked and revocation reason.
- `subscription.pause_scheduled`: Bridge stores `google_pause_scheduled_at`.
- `subscription.deferred`: Bridge stores `google_deferred_until`.
- `subscription.price_step_up`: Bridge stores all consent-related fields.
- `reconciliation.drift_detected`: Bridge detects, mutates, and forwards callback.

### Remaining HiHa Gaps

- `payment.failed`: HiHa lacks local notification audit and acknowledge endpoint.
- `subscription.price_step_up`: HiHa lacks accept/decline routes.
- `reconciliation.drift_detected`: HiHa does not consume or apply corrections to local state.
- One-time purchase refund/revoke: HiHa needs better classification to distinguish OTP from subscription.

## Ownership Map

Bridge owns:
- Provider webhook ingress, signature validation, deduplication, stale suppression.
- Provider event normalization.
- Payment and subscription source-of-truth.
- Provider API calls, purchase acknowledgement, cancellation, resume, portal, price-step-up provider actions.
- Reconciliation, pause scheduler, price-step-up expiry scheduler.
- Callback forwarding and retry/dead-letter state.
- Admin alerts for dispute/reconciliation.

HiHa owns:
- Clerk user identity and app-local premium access flags.
- App-facing status response and UX flags (queried from Bridge).
- App-specific acknowledgement endpoints such as payment-failure acknowledgement.
- App-specific user notifications.

Both, split by concern:
- Lifecycle events: Bridge mutates canonical provider/payment/subscription state; HiHa mutates app-local premium/cache/UX state.
- Refund/revoke: Bridge records provider financial truth; HiHa revokes app entitlement.
- Price step-up: Bridge tracks provider consent lifecycle and expiry; HiHa exposes user action UI and routes.

- Option B (Bridge Authoritative Read Path) was implemented, resolving the need for a local `subscription_cache` in HiHa.
- HiHa still falls back to Bridge for premium checks in guards, which is now the sanctioned pattern.

