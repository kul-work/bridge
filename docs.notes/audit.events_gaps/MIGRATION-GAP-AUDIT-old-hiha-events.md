# Migration Gap Audit: Old HiHa Payment Events vs Tyde Bridge + HiHa

Date: 2026-05-04

Scope:
- Old HiHa: `C:\share\hiha`
- Tyde Bridge: `C:\share\tyde\bridge`
- Tyde HiHa: `C:\share\tyde\hiha`

This is an investigation-only audit of payment, subscription, webhook, scheduler, email, local premium, and UX state behavior that existed in old HiHa and is missing, partial, or incorrectly split across Tyde Bridge and Tyde HiHa.

## Findings

### 1. HiHa callback state is too thin for subscription lifecycle

Old locations:
- `C:\share\hiha\src\webhooks\processor.rs:37`
- `C:\share\hiha\src\webhooks\events\subscription\common.rs`
- `C:\share\hiha\src\db\subscriptions.rs`

New expected owner:
- Bridge owns provider lifecycle source-of-truth and normalized callbacks.
- HiHa owns app-facing local premium/cache/UX state derived from callbacks.

Current Tyde locations:
- `C:\share\tyde\hiha\src\db\webhook_callbacks.rs:99`
- `C:\share\tyde\hiha\migrations\03_create_webhook_callbacks_table.sql:8`
- `C:\share\tyde\hiha\docs\BEHAVIORAL_SPEC.md:241`

Missing behavior:
- No implemented `subscription_cache` table even though the Tyde HiHa behavioral spec says the subscription status endpoint should read local cache.
- `subscription.activated` and `subscription.resumed` set `users.is_premium = true` but do not set `premium_expires_at`.
- `subscription.cancelled` always maps to inactive access through `SubscriptionInactive`, even though old HiHa preserved access until `current_period_end` for scheduled cancellations.
- No local storage for `auto_renewing`, `revocation_reason`, `revoked_at`, `payment_failure_notification`, `google_requires_price_step_up_consent`, `google_new_price_cents`, price-step-up deadline, pause/deferred timestamps, or reconciliation fields.

Impact:
- Premium access and UI status can become stale or wrong.
- Scheduled cancellations can remove access immediately instead of at period end.
- HiHa cannot reliably surface lifecycle UX flags without querying Bridge directly or adding the missing cache.

Gap types:
- Missing HiHa local cache/state update.
- Missing DB mutation.
- Missing tests.
- Docs/spec drift.

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


### 5. `subscription.deferred` mutates Bridge but is not forwarded with useful HiHa-consumable state

Old locations:
- `C:\share\hiha\src\webhooks\processor.rs:74`
- `C:\share\hiha\src\services\google_play\subscription_lifecycle.rs:616`

New expected owner:
- Bridge stores provider deferred state and forwards deferred timestamp.
- HiHa stores the deferred timestamp and surfaces it in UX.

Current Tyde locations:
- `C:\share\tyde\bridge\src\webhooks\processor\event_handlers.rs:573`
- `C:\share\tyde\bridge\src\db\subscriptions.rs:389`
- `C:\share\tyde\hiha\src\handlers\webhooks.rs:226`

Missing behavior:
- HiHa callback payload struct has no `google_deferred_until` field.
- HiHa lacks local storage for the deferred timestamp.

Impact:
- Bridge state is correct and forwarded, but HiHa cannot show deferred renewal date or update local status/cache.

Gap types:
- Missing HiHa local cache/state update.
- Missing tests.

### 6. `subscription.price_step_up` is partial

Old locations:
- `C:\share\hiha\src\webhooks\processor.rs:71`
- `C:\share\hiha\src\services\google_play\subscription_lifecycle.rs:428`
- `C:\share\hiha\src\handlers\payments\subscription.rs:390`
- `C:\share\hiha\src\handlers\payments\subscription.rs:444`

New expected owner:
- Bridge owns provider step-up state and expiry/cancel scheduler.
- HiHa owns user-facing consent UI state and accept/decline endpoints that call Bridge.

Current Tyde locations:
- `C:\share\tyde\bridge\src\services\google_play\subscription_lifecycle.rs:244`
- `C:\share\tyde\bridge\src\db\subscriptions.rs:341`
- `C:\share\tyde\bridge\src\webhooks\scheduler.rs:290`
- `C:\share\tyde\hiha\src\handlers\webhooks.rs:217`
- `C:\share\tyde\hiha\src\main.rs:147`

Missing behavior:
- HiHa only logs the callback; it does not persist consent-required state locally.
- HiHa callback payload omits `new_price_cents` and consent deadline.
- No Tyde HiHa price-step-up accept/decline routes are registered.

Impact:
- Users may not see that consent is required or be able to act from HiHa.
- Consent expiry may cancel users without an app-visible action path.

Gap types:
- Missing HiHa local cache/state update.
- Missing endpoint/acknowledgment flow.
- Missing tests.

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

### Fully or mostly ported in Bridge source-of-truth

- `payment.refunded`: Bridge updates payment status and revokes matching subscription by token/subscription id in `C:\share\tyde\bridge\src\webhooks\processor\event_handlers.rs:775`. HiHa behavior is partial for OTP and local UX/email.
- `payment.partially_refunded`: Bridge updates payment status without premium revoke in `C:\share\tyde\bridge\src\webhooks\processor\event_handlers.rs:831`. HiHa logs only.
- `subscription.grace_period`: Bridge sets status `past_due` and grace timestamps in `C:\share\tyde\bridge\src\db\subscriptions.rs:95`; HiHa only sets premium true and expiry if callback has period end.
- `subscription.on_hold`: Bridge sets status `on_hold` and payment failure flag in `C:\share\tyde\bridge\src\db\subscriptions.rs:146`; HiHa marks non-premium but no cache/email.
- `subscription.paused`: Bridge sets status `paused` in `C:\share\tyde\bridge\src\db\subscriptions.rs:167`; scheduler can also emit pause callback. HiHa marks non-premium but no cache/email.
- `subscription.resumed`: Bridge transitions paused/cancelled to active in `C:\share\tyde\bridge\src\services\google_play\subscription_lifecycle.rs:120`; HiHa sets premium true but misses expiry.
- `subscription.cancelled`: Bridge stores cancellation and context in `C:\share\tyde\bridge\src\db\subscriptions.rs:267`; HiHa incorrectly treats all cancellations as immediate inactive.
- `subscription.expired`: Bridge sets expired in `C:\share\tyde\bridge\src\db\subscriptions.rs:246`; HiHa marks inactive.
- `subscription.revoked`: Bridge sets revoked and revocation reason in `C:\share\tyde\bridge\src\services\google_play\subscription_lifecycle.rs:70`; HiHa marks inactive but drops reason/cache/email.
- `subscription.pause_scheduled`: Bridge stores `google_pause_scheduled_at` in `C:\share\tyde\bridge\src\webhooks\processor\event_handlers.rs:538`; HiHa can log event but cannot persist/surface local state.
- `reconciliation.drift_detected`: Bridge detects, mutates, emails admin, and forwards callback in `C:\share\tyde\bridge\src\webhooks\scheduler.rs:149`; HiHa does not consume drift fields.

### Partial or missing

- `payment.failed`: Bridge DB mutation exists, but user email, local notification audit, and acknowledge endpoint are missing in Tyde HiHa.
- `subscription.price_step_up`: Bridge state/scheduler exists, but HiHa callback field consumption, local UI state, accept/decline routes, and user email are missing.
- `subscription.deferred`: Bridge state exists, but callback fields/event effects and HiHa local state/email are missing.
- `subscription.renewed`: Bridge maps to activation and records payment; HiHa only sees `subscription.activated`, so renewal-specific local expiry handling/logging is incomplete.
- `subscription.trial_started`: Bridge stores `trial` but sends callback as `subscription.activated` with status `trial`; HiHa does not treat trial distinctly or set expiry.
- `subscription.recovered`: Bridge maps to activation; HiHa treats as activation, not recovery-specific email/UX.
- `payment.succeeded`: No standalone old behavior found. Old success behavior was represented by `order.completed`, `subscription.paid`, `subscription.created`, `subscription.recovered`, or OTP purchase. Bridge maps these to activation or `purchase.one_time`.
- One-time purchase refund/revoke: Bridge can emit `purchase.one_time` for cancel but generic refunds can arrive as `payment.refunded`; HiHa OTP revoke depends on event classification.

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
- Local app-facing subscription/notification cache derived from Bridge callbacks.
- Frontend-facing status response and UX flags.
- App-specific acknowledgement endpoints such as payment-failure acknowledgement.
- App-specific user notifications if Bridge intentionally avoids storing/sending user email.

Both, split by concern:
- Lifecycle events: Bridge mutates canonical provider/payment/subscription state; HiHa mutates app-local premium/cache/UX state.
- Refund/revoke: Bridge records provider financial truth; HiHa revokes app entitlement.
- Price step-up: Bridge tracks provider consent lifecycle and expiry; HiHa exposes user action UI and routes.

## Open Questions / Assumptions

- I treated `C:\share\tyde\hiha\docs\BEHAVIORAL_SPEC.md` as intended design. The implementation lacks the documented `subscription_cache` table, so this looks accidental rather than an intentional drop.
- HiHa currently falls back to Bridge for premium checks in guards. That contradicts the HiHa agent guide statement that premium should be updated via Bridge callbacks and not polled for subscription status.

