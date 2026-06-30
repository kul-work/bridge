# Webhook Lifecycle Email Side Effects Inside Processing Transaction

Date: 2026-06-30

## Status

Deferred follow-up. This is intentionally outside the webhook atomicity fix.

## Problem

`process_webhook_atomically` now runs provider webhook processing inside one database transaction so state mutation, stored canonical payload, and `webhook_provider.processed = true` commit together.

That fixes the outbox atomicity window, but it also means any external side effects performed inside webhook event handlers can happen before the database transaction commits.

The current lifecycle email helpers in `src/webhooks/processor/event_handlers.rs` perform email lookup and email send during webhook processing:

- `send_price_step_up_email`
- `send_deferred_email`
- `send_paused_email`
- `send_resumed_email`
- `send_refunded_email`
- `send_payment_failed_email`
- dispute admin alert send path

Those paths can do external HTTP work while the webhook transaction is still open.

## Risks

- Email may be sent even if the DB transaction later rolls back.
- DB locks may be held while email lookup/provider HTTP calls are in flight.
- Retry after rollback could send duplicate emails.
- Email failure handling is currently best-effort/logging in many paths, but the timing is still pre-commit.

## Desired Direction

Move lifecycle email side effects out of the webhook processing transaction.

Recommended shape:

1. During webhook processing, produce durable email intents or post-commit effects instead of sending immediately.
2. Commit subscription/payment/webhook state and canonical callback payload first.
3. Process email intents after commit, ideally through a retryable outbox table.
4. Make email intent idempotency explicit, for example keyed by:
   - `app_id`
   - `provider`
   - `provider_webhook_id`
   - email type
   - subscription/payment identity
5. Keep lifecycle email failures from rolling back payment/subscription state.

## Acceptance Criteria

- Webhook DB transaction does not await email lookup or provider email sends.
- A crash after DB commit but before email send leaves a durable retryable email intent, or a documented deliberate drop policy.
- Reprocessing the same provider webhook cannot send duplicate lifecycle emails.
- Tests cover rollback/no-send or durable-intent behavior for at least one lifecycle email path.

## Related Code

- `src/webhooks/processor/atomic.rs`
- `src/webhooks/processor/event_handlers.rs`
- `src/services/email.rs`
- `src/services/email_lookup.rs`
- `src/services/google_play/notifications.rs`
