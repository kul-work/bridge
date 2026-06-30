# Strategy for Architectural Review Item 4 — Multi-Instance Worker Safety

Author: Amp

Source review item: `docs.notes/architectural-review-2026-06-24.md`, section `## 4. Critical — Multi-instance deployment can duplicate callbacks and provider side effects`.

## Goal

Make Bridge safe to run with multiple server instances while background jobs are enabled. Each instance may keep starting the same workers, but every external side effect must be protected by a durable database claim before it runs.

The important rule: do not solve this by relying on a single active worker instance. That hides the race instead of making the system safe.

## Current Risk

The review item identifies four concrete race points:

- All workers start in every process when `ENABLE_BACKGROUND_JOBS=true`.
- Pending webhook deliveries are listed without a claim or lease.
- App callback HTTP POST happens before the delivery attempt is persisted.
- Price step-up expiry calls provider cancel before the subscription row is marked expired.

With two Bridge instances, both can send the same app callback, increment attempts incorrectly, dead-letter early, or execute provider-side effects twice.

## Recommended Direction

Use durable row leases with fencing tokens.

Each unit of work should follow this pattern:

1. Claim the row in a short DB transaction.
2. Commit the claim.
3. Run the external side effect.
4. Persist the result in a second short DB transaction using the claim token.
5. Clear the active claim when the work reaches a terminal or retryable state, and set a durable retry-not-before timestamp for retryable failures.

Do not hold a DB transaction open while doing HTTP or provider API calls.

## Why Row Leases Instead of Advisory Locks

Use row leases as the primary fix because they are durable, visible in operational queries, reclaimable after process crashes, and compatible with `FOR UPDATE SKIP LOCKED`.

Advisory locks are acceptable only for coarse best-effort jobs such as cleanup. They should not guard callback delivery or provider mutations because they do not provide durable per-row ownership or fencing.

## Data Model Changes

### Webhook Delivery Claims and Retry Gate

Add claim fields and a durable retry gate to `pay.webhook_delivery`:

```sql
ALTER TABLE pay.webhook_delivery
    ADD COLUMN claim_token UUID,
    ADD COLUMN claimed_by TEXT,
    ADD COLUMN claimed_until TIMESTAMPTZ,
    ADD COLUMN next_attempt_at TIMESTAMPTZ DEFAULT NOW();
```

`claimed_until` is the active owner lease. `next_attempt_at` is the retry-not-before schedule. A failed delivery must not become immediately claimable again after its claim is cleared.

Add a partial index for claimable work:

```sql
CREATE INDEX idx_webhook_delivery_claimable
ON pay.webhook_delivery(app_id, next_attempt_at, created_at)
WHERE forwarded = false
  AND dead_lettered = false
  AND forward_attempts < 3;
```

Replace pending delivery listing with a claim operation that uses `FOR UPDATE SKIP LOCKED` and returns only rows owned by the current worker.

### Scheduler Claims

Add generic scheduler claim fields to `pay.subscriptions`:

```sql
ALTER TABLE pay.subscriptions
    ADD COLUMN scheduled_job_claim_token UUID,
    ADD COLUMN scheduled_job_claimed_by TEXT,
    ADD COLUMN scheduled_job_claimed_until TIMESTAMPTZ,
    ADD COLUMN scheduled_job_claim_kind TEXT;
```

Use `scheduled_job_claim_kind` values such as:

- `price_step_up_expiry`
- `pause_transition`
- `reconciliation`

This avoids adding separate claim columns for every scheduler job.

With one shared scheduler claim slot, every claim and completion statement must include `app_id`, `scheduled_job_claim_token`, and `scheduled_job_claim_kind`. A retryable scheduler provider failure must not clear the row into an immediately claimable state.

## Phase 1 — Claim Webhook Deliveries Before Forwarding

Replace `list_pending_webhook_deliveries` with a claim API:

```text
claim_pending_webhook_deliveries(app_id, worker_id, lease_secs, limit)
```

Suggested SQL shape:

```sql
WITH candidates AS (
    SELECT id
    FROM pay.webhook_delivery
    WHERE app_id = $1
      AND forwarded = false
      AND dead_lettered = false
      AND forward_attempts < 3
      AND next_attempt_at <= NOW()
      AND (claimed_until IS NULL OR claimed_until < NOW())
    ORDER BY created_at ASC
    LIMIT $4
    FOR UPDATE SKIP LOCKED
)
UPDATE pay.webhook_delivery wd
SET claim_token = gen_random_uuid(),
    claimed_by = $2,
    claimed_until = NOW() + ($3 * INTERVAL '1 second'),
    updated_at = NOW()
FROM candidates
WHERE wd.id = candidates.id
RETURNING wd.*;
```

Then update completion so only the current claim holder can record the outcome:

```sql
WHERE app_id = $app_id
  AND id = $delivery_id
  AND claim_token = $claim_token
```

On success or terminal suppression/no-callback, the completion update should set the terminal fields, record the last HTTP result when present, clear `claim_token`, `claimed_by`, and `claimed_until`, and clear `next_attempt_at`.

On retryable failure, it should increment `forward_attempts`, persist the HTTP/error result, clear the active claim fields, and set `next_attempt_at` to a durable backoff time. For example: first failure after 1 minute, second failure after 5 minutes. If `forward_attempts + 1 >= 3`, mark `dead_lettered = true` instead of scheduling another retry.

A delivery must not be claimable again until `next_attempt_at <= NOW()`. This prevents multiple Bridge instances from burning all three attempts immediately during a callback outage.

## Phase 2 — Make All Callback Paths Use the Same Claim Flow

The retry worker and the immediate forwarding path must both claim before POST. The safest immediate-path implementation is to create the delivery already claimed by the current worker in the same enqueue transaction.

Safe order:

```text
create or claim delivery for this worker
commit
process provider webhook or load stored canonical payload
load app
perform stale suppression check
send HTTP callback, or mark stale suppressed
complete attempt using claim_token
clear claim, and set next_attempt_at only for retryable failures
```

This prevents two Bridge instances from sending the same callback at the same time.

If `ON CONFLICT` returns an existing delivery, the immediate path must not take over or forward it opportunistically. Treat it as already owned or queued and let the current owner or retry worker handle it.

`forward_webhook` should not be callable with only a `delivery_id`. Require a claimed delivery or a `claim_token` parameter so unclaimed forwarding is impossible at the API boundary.

Important limitation: leases provide at-least-once delivery, not exact-once delivery. If the HTTP callback succeeds and Bridge crashes before the completion update, the delivery can be retried after the lease expires. App callbacks must remain idempotent by event ID.

## Phase 3 — Make Scheduler Event IDs Deterministic

Replace random scheduler event IDs with deterministic IDs derived from the cause.

Current unsafe shape:

```text
scheduler-{random_uuid}
```

Recommended shapes:

```text
scheduler:price_step_up_expiry:{subscription_row_id}:{deadline_epoch_ms}
scheduler:pause_transition:{subscription_row_id}:{pause_scheduled_at_epoch_ms}
scheduler:reconciliation_drift:{subscription_row_id}:{previous_status}:{corrected_status}:{previous_last_event_time}:{previous_version}
```

Use the deterministic value as `provider_webhook_id`. The existing uniqueness on `(app_id, provider, provider_webhook_id)` can then deduplicate duplicate scheduler emissions from multiple instances.

For reconciliation, include stable pre-correction state such as `last_event_time` and `version`. That dedupes duplicate workers for the same drift cause without suppressing a later legitimate same-status drift forever.

## Phase 4 — Claim Price Step-Up Expiry Before Provider Cancel

Replace expired subscription listing with a claim API:

```text
claim_price_step_up_expired_subscriptions(app_id, worker_id, lease_secs, limit)
```

The claim statement should set:

```sql
scheduled_job_claim_token = gen_random_uuid(),
scheduled_job_claimed_by = $worker_id,
scheduled_job_claimed_until = NOW() + ($lease_secs * INTERVAL '1 second'),
scheduled_job_claim_kind = 'price_step_up_expiry'
```

Eligibility should include:

```sql
WHERE app_id = $app_id
  AND google_requires_price_step_up_consent = true
  AND google_price_step_up_consent_deadline IS NOT NULL
  AND google_price_step_up_consent_deadline < NOW()
  AND (
      scheduled_job_claimed_until IS NULL
      OR scheduled_job_claimed_until < NOW()
  )
```

Safe order:

```text
claim expired subscription
load provider config
call provider cancel
mark subscription cancelled using claim_token
enqueue deterministic subscription.cancelled callback
```

The state update should verify both the claim token and the original eligibility:

```sql
WHERE app_id = $app_id
  AND id = $id
  AND scheduled_job_claim_token = $claim_token
  AND scheduled_job_claim_kind = 'price_step_up_expiry'
  AND google_requires_price_step_up_consent = true
  AND google_price_step_up_consent_deadline < NOW()
```

Open behavior decision: the current code continues to mark the subscription cancelled even if provider cancel fails. The safer behavior is to avoid marking cancelled or emitting the callback when provider cancel fails, but that is a behavior change and should be approved explicitly.

If provider cancel fails and the job should retry, do not clear the row into an immediately claimable state. Either leave `scheduled_job_claimed_until` as the retry-not-before lease or add an explicit scheduler retry gate before the row can be claimed again. Do not emit the subscription callback unless the DB state transition succeeds under the claim token.

## Phase 5 — Cover Remaining Multi-Instance Side Effects

After webhook delivery and price step-up expiry are safe, handle the remaining side effects:

- **Pause scheduler**: use deterministic scheduler IDs and ideally combine state transition plus synthetic delivery enqueue in one transaction.
- **Reconciliation**: add a guard such as `status IS DISTINCT FROM $new_status`, include `app_id` in the update, and use deterministic scheduler event IDs based on pre-correction state.
- **Google acknowledgement retries**: claim payment or acknowledgement candidates before provider acknowledgement calls.
- **User-triggered price step-up decline**: review this Google cancel path against the same subscription/provider-action claim, or explicitly keep it out of scope for item 4.
- **Cleanup**: can remain idempotent. Use advisory lock only if duplicate cleanup creates operational noise.

## Tests to Add

Add DB-backed concurrency coverage for the new guarantees:

1. Two concurrent delivery claims never return the same delivery ID.
2. An active delivery lease excludes another worker until expiry.
3. A stale claim token cannot complete after the row is reclaimed.
4. A failed webhook delivery is not claimable before `next_attempt_at`, even after its claim is cleared.
5. A failed webhook delivery becomes claimable after `next_attempt_at`.
6. Two concurrent forwarding attempts produce one callback POST.
7. Manual reset refuses active claimed deliveries unless the claim expired.
8. Immediate ingress-created delivery is already claimed before the spawned processing task can run.
9. Synthetic scheduler delivery is already claimed when created.
10. `forward_webhook` completion fails or is impossible without the matching claim token.
11. The same scheduler cause produces the same deterministic `provider_webhook_id`.
12. Two identical scheduler enqueue attempts create one provider row and one delivery row.
13. Two concurrent price step-up expiry workers produce one provider cancel and one callback delivery.
14. Price step-up completion with the right token but wrong `scheduled_job_claim_kind` affects zero rows.
15. Provider cancel failure does not emit a scheduler callback if the safer behavior is adopted, and does not make the row immediately claimable by another worker.
16. Reconciliation duplicate workers emit one callback for the same pre-correction row version, but a later same-status drift with a new version or `last_event_time` gets a new event ID.

## Verification Before Calling It Fixed

This is a high-risk payment/provider change. Run at least:

- `cargo check`
- targeted DB-backed concurrency tests
- webhook idempotency checks
- subscription lifecycle checks
- payment side-effect checks
- observability/PII checks if logging or diagnostic output changes

## Main Risk That Remains

Row leases prevent concurrent duplicate side effects, but they do not guarantee exact-once provider behavior after a crash between provider success and DB completion.

If Google cancel or acknowledgement calls are not idempotent, Bridge needs a provider-side-effect outbox with explicit idempotency semantics. If provider calls are idempotent, row leases plus fencing tokens are the smallest safe fix.
