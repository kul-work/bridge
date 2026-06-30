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

## Alternative Strategy Without a DB Outbox

If Bridge intentionally avoids adding a database outbox table for lifecycle emails, use in-memory post-commit effects instead. This is not equivalent to a durable outbox, but it removes external I/O from the webhook transaction with the smallest schema-free change.

Recommended shape:

1. During webhook processing, collect typed post-commit effects instead of sending emails directly.
2. Add those effects to `EventEffects`, for example as lifecycle email effects and dispute admin alert effects.
3. Keep each effect as plain data: app/provider/webhook identity, external user id, subscription id, email type, and template-specific payload such as price step-up deadline, deferred-until timestamp, current period end, or app URL.
4. Return the canonical webhook payload together with the collected post-commit effects from `process_webhook`.
5. In `process_webhook_atomically`, commit the DB transaction first, then execute the collected effects.
6. Move email lookup and provider email sends into the post-commit effect executor.
7. Treat post-commit email failures as best-effort notification failures: log them with non-PII diagnostic context and never roll back payment/subscription/webhook state.

Tradeoff:

- This avoids email lookup and provider HTTP calls inside the DB transaction.
- If the DB transaction rolls back, no post-commit effects run.
- Reprocessing a normally processed webhook should not send another email because `webhook_provider.processed = true` prevents the handler path from running again.
- A process crash after DB commit but before effect execution can drop the email because there is no durable intent. This must be documented as a deliberate best-effort lifecycle notification policy.

Idempotency should still be best-effort and explicit. Build a stable email idempotency key from `app_id`, `provider`, `provider_webhook_id`, email type, and subscription/payment identity. Pass it through email context or provider-specific headers where supported. Without provider idempotency or durable storage, crash-after-send-before-log/mark cases cannot be made perfectly duplicate-proof.

Suggested vertical slice:

1. Convert one path, preferably `payment.failed`, from direct send to post-commit effect.
2. Add a rollback/no-send test for that path.
3. Add a send-after-commit test for that path.
4. Convert the remaining lifecycle helpers after the pattern is proven.
5. Handle dispute admin alerts separately because their recipient and payload semantics differ from user lifecycle notifications.

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
