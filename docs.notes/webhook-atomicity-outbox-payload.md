# Proposal: Atomic Webhook State Transition & Durable Outbox Payload

> Status: **Proposed** — not implemented.
> Scope: `src/webhooks/processor.rs`, `src/webhooks/ingress.rs`, `src/webhooks/scheduler.rs`, `src/webhooks/forwarding.rs`, `src/db/webhooks.rs`, `migrations/`.
> Risk tier: **3** (lifecycle/webhook-sensitive). See `AGENTS.md` → "Bridge-Specific Checks".

## Problem

Webhook processing performs three logically coupled operations in **separate DB transactions**:

1. Mutate subscription/payment state via per-handler repo calls (each opens/commits its own `begin_app_tx`).
2. Mark `webhook_provider.processed = true` (`src/db/webhooks.rs:127-142` — its own transaction).
3. Enqueue/forward the callback via `webhook_delivery` + `forward_webhook` (`src/webhooks/ingress.rs:73-90`, `src/webhooks/scheduler.rs:106-122`, `processor.rs:1261-1283`).

Because these are not atomic, a crash between any two steps yields one of:

- **State changed, callback never queued** → app never notified (lost event).
- **State changed, `processed = false`** → duplicate provider delivery reprocesses the same event (idempotency relies on the `processed` flag alone, not on the state transition committed alongside it).
- **Retry sends a rebuilt projection, not the committed event** → `build_canonical_payload` (`src/webhooks/processor.rs:669-873`) merges the stored raw `payload` with the **current** DB subscription row. Fields like `status`, `current_period_end`, `auto_renewing`, `revocation_reason`, and pending-price-change data reflect DB state at retry time, not at original commit time. Intermediate lifecycle events can be silently flattened or distorted.

The root cause is that `webhook_delivery` stores **retry state only** (`forward_attempts`, `last_http_status`, `dead_lettered`, …) — there is no immutable canonical payload column anywhere in the schema (`migrations/04_create_webhooks.sql:29-51`). Retries therefore have nothing to replay except a rebuilt projection.

## Mechanism at a Glance

```text
Provider webhook
      |
      v
pay.webhook_provider inbox row
      |
      v
Single DB transaction
  - lock/claim inbox row
  - apply lifecycle/payment mutation
  - store immutable webhook_delivery.canonical_payload
  - mark webhook_provider.processed = true
      |
      v
Commit
      |
      v
Delivery worker
  - load canonical_payload from webhook_delivery
  - POST app callback
  - update retry/dead-letter state
```

## What We Already Have (Inbox)

The "inbox" half of the pattern is already in place as `pay.webhook_provider` (`migrations/04_create_webhooks.sql:5-24`):

- `provider_webhook_id` unique per `(app_id, provider)` via `idx_webhook_provider_app_id` — ingress dedup.
- `payload JSONB` — immutable raw provider payload.
- `processed` / `suppressed` / `recovery_claimed_at` — claim & replay semantics.
- Comment on the table literally reads: *"Incoming webhooks from payment providers. Deduplication, idempotent processing, and audit trail."*

No new inbox table is needed. The gap is **only** the outbox half plus transaction atomicity.

## Proposal

### 1. Store the canonical payload immutably (Outbox)

Add a column to hold the frozen canonical payload produced at processing time:

```sql
ALTER TABLE pay.webhook_delivery
  ADD COLUMN canonical_payload JSONB;
```

`canonical_payload` is written **once**, in the same transaction that marks `webhook_provider.processed = true`. It is never updated thereafter. It is the source of truth for retries and forwarding.

### 2. Make claim → transition → outbox → processed atomic

Inside a single `begin_app_tx`:

1. Claim the inbox row (`SELECT … FOR UPDATE` on `webhook_provider` by id, or rely on `recovery_claimed_at` for the retry path).
2. Apply the lifecycle/payment transition via the existing event handlers.
3. Insert (or update) `webhook_delivery.canonical_payload` with the freshly built `CanonicalWebhookPayload`.
4. `UPDATE pay.webhook_provider SET processed = true WHERE id = $1`.
5. Commit.

Only **after** commit does a delivery worker pick up the row and perform the HTTP forward. Forwarding is no longer in the transaction's critical path — it operates on the stored outbox payload.

### 3. Retries read the stored payload, never rebuild

`retry_webhooks` (`src/webhooks/scheduler.rs:64-186`) must stop calling `build_canonical_payload` for `processed = true` rows (the `scheduler.rs:148-181` branch). Instead it reads `webhook_delivery.canonical_payload` and forwards that. The `!processed` branch (`scheduler.rs:105-146`) keeps re-running `process_webhook` (which is correct — the event hasn't been committed yet), but the resulting canonical must be persisted by step 3 above before any forward is attempted.

`build_canonical_payload` is **not deleted** — it remains the producer of the payload during `process_webhook`. What changes is its **output** is now persisted, not held only in memory until the forward completes.

### 4. `forward_webhook` takes the payload from the row, not from an in-memory argument

`forward_webhook` (`src/webhooks/forwarding.rs`) currently receives the canonical payload as an argument from the caller. After this change it must load `canonical_payload` from `webhook_delivery` by id. This makes the forward path idempotent across crashes and retries: the payload a row carries is the payload it sends.

## What Does NOT Change

- Ingress dedup via `webhook_provider (app_id, provider, provider_webhook_id)` — unchanged.
- The `webhook_delivery` retry state columns (`forward_attempts`, `last_http_status`, `dead_lettered`, …) — unchanged.
- `build_canonical_payload` — still the producer, just persisted now.
- Stale-event suppression (`suppressed` / `timestamp_epoch_ms` high-water comparison) — unchanged.
- Per-handler state mutation logic — unchanged, only its **transaction boundary** moves inward.

## Migration Notes

- `canonical_payload JSONB` is nullable for existing rows. Backfill is optional: legacy rows without a stored payload can keep using `build_canonical_payload` on first retry (a one-time fallback), or be queued for a one-shot reprocess. New rows always have it populated.
- Delivery rows created **before** the migration ships will have `canonical_payload = NULL`. `forward_webhook` must handle `NULL` gracefully (fall back to `build_canonical_payload`, log a deprecation warning).

## Risks & Trade-offs

- **Transaction length**: the critical-path transaction now includes HTTP-irrelevant work only (state mutation + payload insert + mark processed). The HTTP forward is moved out. This is strictly better than today, where a slow app endpoint can stall the processing transaction.
- **Payload size**: `JSONB` canonical payloads are typically small (< 4 KB). No special handling expected.
- **Schema change**: one `ALTER TABLE` + nullable column; backward-compatible.

## Verification

- `cargo check` and `cargo clippy` clean.
- A crash injected between state mutation and `mark_webhook_processed` leaves the row `processed = false` with **no** state written (rollback), proving atomicity.
- A crash after commit but before forward leaves `canonical_payload` populated and `forwarded = false` — retry sends the stored payload verbatim, not a rebuilt projection.
- A retry on a row with `processed = true` returns the same `canonical_payload` bytes regardless of how much DB state has drifted since commit.

## Recommended Checks Before Implementing

From `AGENTS.md` → "Bridge-Specific Checks" (Tier 3):

- `.agents/checks/bridge-webhook-idempotency.md`
- `.agents/checks/bridge-subscription-lifecycle.md`
- `.agents/checks/bridge-payment-side-effects.md`
- `.agents/checks/bridge-payment-identity.md`
- `.agents/checks/bridge-release-risk.md`
