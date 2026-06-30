# Webhook Atomicity - Codex Implementation Notes

Status: implemented in current worktree, pending review.

Date: 2026-06-30

## Summary

This work closes architectural review item `3. Critical - State mutation, processed marking, and callback enqueue are not atomic`.

For new webhook records, Bridge now commits the following as one durable decision before HTTP forwarding:

1. Provider webhook state/payment/subscription mutation.
2. Canonical app callback payload storage.
3. `webhook_provider.processed = true`.

HTTP forwarding remains outside the transaction.

The old retry behavior that rebuilt processed callback payloads from current DB state has been removed. A processed delivery without stored `canonical_payload` is now an error, not a rebuild candidate.

## Implemented Pieces

### Schema

Added migration:

- `migrations/96_webhook_delivery_canonical_payload.sql`

It adds:

- `pay.webhook_delivery.canonical_payload JSONB`

This column stores the immutable app callback payload that retry must replay.

### Normal Provider Webhook Path

Ingress now calls the atomic processor:

- `src/webhooks/ingress.rs`
- `src/webhooks/processor/atomic.rs`

`process_webhook_atomically`:

1. Opens one app-scoped DB transaction.
2. Runs existing webhook processing through a tx-backed repository adapter.
3. Applies payment/subscription mutations through tx-scoped helpers.
4. Serializes the produced canonical payload.
5. Stores it on `webhook_delivery.canonical_payload`.
6. Marks `webhook_provider.processed = true`.
7. Commits.

Forwarding happens only after commit.

### Synthetic Callback Path

Synthetic callbacks also use immutable outbox storage now.

Covered callers:

- verify purchase callbacks
- subscription action callbacks
- reconciliation/drift callbacks

Relevant implementation:

- `src/db/webhooks.rs::create_synthetic_webhook_delivery`
- `src/webhooks/forwarding.rs::create_and_forward_webhook`
- `src/webhooks/forwarding.rs::queue_and_forward_webhook`

The synthetic helper creates/fetches the synthetic provider row, creates/updates the delivery with `canonical_payload`, and marks the provider row processed in one DB transaction before forwarding.

### Retry Behavior

Retry now does this for processed deliveries:

- if `canonical_payload` exists: deserialize and forward exactly that payload;
- if `canonical_payload` is missing: log an error and skip;
- it no longer calls `build_canonical_payload` for processed rows.

Unprocessed provider rows are still routed through `process_webhook_atomically`.

This makes the invariant explicit for new data:

```text
processed webhook delivery => canonical_payload must be present
```

### Google Play Acknowledgement

Webhook event handlers no longer call Google Play acknowledgement HTTP inside the mutation path.

Instead:

- payment rows persist `provider_purchase_token` and `ack_required`;
- the existing acknowledgement retry worker processes durable candidates;
- one-time product acknowledgement candidates were added alongside subscription acknowledgement candidates.

This avoids holding the webhook DB transaction open while calling Google Play.

## Test Updates

`tests/gpbi/test-otp-rtdn-02.sh` now validates its OTP-01 prerequisite.

Behavior:

- if `--token` is supplied, the explicit token is authoritative and OTP-01 is not rerun;
- if no token is supplied, the script checks the OTP-01 report row;
- if that row is missing or not `success`, it reruns OTP-01 before testing RTDN refund.

This prevents stale `refunded` OTP-01 fixtures from making OTP-RTDN-02 only prove idempotency instead of a clean `success -> refunded` transition.

## Verification Run

Observed passing checks during implementation:

- `cargo check`
- `cargo test webhooks::processor`
- `cargo test webhooks::forwarding`
- `cargo test webhooks::scheduler::tests`
- `bash ./tests/test-net-creem-callback-body.sh`
- `bash ./tests/gpbi/test-otp-01.sh`
- `bash ./tests/gpbi/test-otp-05.sh`
- `bash ./tests/gpbi/test-otp-rtdn-02.sh`
- `bash ./tests/gpbi/test-otp-rtdn-02.sh --token <fresh-token>`
- `bash ./test-otp-02.sh` from `tests/creem`
- `bash ./test-acc-03.sh` from `tests/creem`
- `git diff --check`

Manual DB spot check after a verify-purchase synthetic callback:

```text
webhook_provider.processed = true
webhook_delivery.canonical_payload IS NOT NULL
```

## Reviewer Notes

### No Legacy Rebuild Fallback

Because Bridge is not live yet, there is no need to preserve processed rows from an older production schema.

The processed-row fallback that rebuilt callback payloads from current DB state was removed. This is intentional. It keeps the invariant strict and makes missing payloads visible.

### Diff Hygiene

`src/db/payments.rs` and `src/db/subscriptions.rs` had line-ending churn during the work. They were normalized to LF and `git diff --check` was made clean.

### Email Side Effects Are Deferred

Lifecycle email lookup/sending can still happen inside webhook processing. With the new atomic transaction wrapper, that means email HTTP work can occur before DB commit and while DB locks are held.

That is not fixed in this patch. It is documented separately:

- `docs.notes/webhook-email-side-effects-transaction-risk-2026-06-30.md`

Recommended future direction: persist email intents or post-commit effects, then send emails after the webhook state transaction commits.

### Multi-Instance Delivery Claiming Is Still Separate

This patch fixes atomicity and immutable payload replay. It does not add multi-worker delivery claiming.

Future delivery-claim work should add a claim/lease mechanism on `webhook_delivery` so multiple workers cannot send the same pending delivery concurrently.

## Rejected Branch Context

The reviewed `webhook-atomicity-outbox-payload` branch was not merged because it bundled broad webhook/payment/provider rewrites and caused OTP refund regressions.

This implementation intentionally kept the fix narrower:

- keep existing event normalization/lifecycle semantics;
- add a small durable payload primitive;
- add an explicit tx-backed processing adapter;
- move Google Play acknowledgement out of webhook handler HTTP calls;
- make retry replay stored payload only.
