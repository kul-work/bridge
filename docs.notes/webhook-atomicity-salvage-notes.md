# Webhook Atomicity Branch Salvage Notes

Source branch: `webhook-atomicity-outbox-payload`

Decision: do **not** merge or cherry-pick the main implementation commit wholesale. The branch contains a broad webhook/payment refactor, not a surgical concurrency fix. Any salvage should be manual hunk-level extraction.

## Good salvage candidates

### 1. Google Play unknown status rejection

Candidate file:

- `src/services/provider_api.rs`

The branch changes unknown Google Play subscription statuses from silently mapping to `active` into an explicit provider error, with a small unit test.

Why keep it:

- Prevents false-positive premium access if Google introduces a new status Bridge does not understand.
- Is independent from the webhook outbox refactor.

Risk:

- Small behavior change: unknown provider status becomes an error instead of optimistic `active`.

Recommendation: **worth cherry-picking manually**.

### 2. SQLx migration embedding comments

Candidate file:

- `src/db/database.rs`

The branch adds comments noting that `sqlx::migrate!("./migrations")` embeds migrations at compile time.

Why keep it:

- Harmless reminder after migration/test confusion.

Risk:

- None, but low value.

Recommendation: optional.

### 3. Architecture note as future reference

Candidate file:

- `docs.notes/webhook-atomicity-outbox-payload.md`

The note is useful as a future architecture reference, but the branch version says the cleanup is complete and describes behavior that is not being shipped.

Recommendation: keep only if rewritten with a status like:

> Status: discarded implementation / future architecture reference, not current behavior.

## Maybe salvage, but reimplement narrowly

### 4. Google Play acknowledgement row claiming

The idea is good: claim candidate rows before provider acknowledgement side effects so two workers cannot acknowledge the same payment concurrently.

Do **not** cherry-pick the branch hunks directly. They are mixed into the larger scheduler/refactor work.

If needed, reimplement as a small standalone patch:

- migration: add `ack_claim_owner`, `ack_claim_expires_at` to `pay.payments`
- `src/db/payments.rs`: claim candidates with `FOR UPDATE SKIP LOCKED` / `UPDATE ... RETURNING`
- `src/webhooks/scheduler.rs`: call the claim function with a short lease
- verify two workers cannot claim the same ack row

Recommendation: good future small mitigation, but not a direct cherry-pick.

### 5. Subscription status DB constraint

Candidate migrations:

- `migrations/100_subscription_status_check.sql`
- `migrations/101_subscription_status_check_grace_period.sql`

This adds an allowed-status check for subscriptions.

Risk:

- Medium. Even `NOT VALID` constraints still apply to new/updated rows. If current code writes an unlisted status, production writes can fail.

Recommendation: only after auditing every current subscription status writer.

## Do not salvage directly

Avoid cherry-picking these from the branch:

- `src/webhooks/processor/event_handlers.rs` refactor
- `src/webhooks/processor.rs` prebuilt canonical payload changes
- `src/webhooks/forwarding.rs` stored-payload forwarding changes
- large `src/db/webhooks.rs` outbox primitive additions
- migrations `96`-`98` as-is
- deletion of `src/services/google_play/product_lifecycle.rs`
- deletion of `src/services/google_play/subscription_lifecycle.rs`
- docs updates that describe stored-payload outbox behavior as current production behavior
- subscription/payment advisory locks unless reintroduced as part of a designed small patch

Reason: these are entangled with the broader refactor and the observed OTP refund/callback regressions.

## Shortlist

If salvaging wins now, take only:

1. `src/services/provider_api.rs` unknown Google status rejection.
2. Optional `src/db/database.rs` migration embedding comments.
3. Optional rewritten architecture note as future reference.

Everything else should stay on `webhook-atomicity-outbox-payload` as reference until a deliberately scoped follow-up plan exists.
