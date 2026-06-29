# Proposal: Atomic Webhook Processing and Stored Callback Outbox

> Status: **Proposed** — not implemented.
> Scope: `src/webhooks/processor.rs`, `src/webhooks/ingress.rs`, `src/webhooks/scheduler.rs`, `src/webhooks/forwarding.rs`, `src/db/webhooks.rs`, `src/ports/`, `migrations/`.
> Risk tier: **3** (lifecycle/webhook-sensitive). See `AGENTS.md` → “Bridge-Specific Checks”.

## Problem

Webhook processing currently splits one logical commit across separate operations:

1. Mutate subscription/payment state.
2. Mark `pay.webhook_provider.processed = true`.
3. Create/update `pay.webhook_delivery` and forward the app callback.

Those operations are not atomic. A crash or retry at the wrong point can leave Bridge in one of these states:

- subscription/payment state changed, but no app callback was queued;
- subscription/payment state changed, but the provider webhook still looks unprocessed;
- retry rebuilds a callback from current DB state instead of replaying the event that was originally accepted.

The durable inbox already exists: `pay.webhook_provider` stores the raw provider event and dedup identity. The missing piece is a durable outbox record that stores the exact canonical callback payload accepted by processing, plus a transaction boundary that commits state mutation, outbox creation, and processed marking together.

## Goal

Move webhook processing to this invariant:

```text
Provider webhook
      |
      v
pay.webhook_provider inbox row
      |
      v
single DB transaction
  - lock provider webhook row
  - reject/suppress stale or invalid event before side effects
  - apply subscription/payment transition
  - if app callback is required, insert immutable outbox payload
  - mark provider webhook processed
      |
      v
commit
      |
      v
delivery worker sends the stored outbox payload
```

Once an event has an outbox row, delivery must send that stored event. It must not rebuild the payload from current DB state or re-run lifecycle stale checks against newer subscription state.

## Plan

### 1. Add stored canonical payload to the outbox row

Add a nullable JSONB column:

```sql
ALTER TABLE pay.webhook_delivery
  ADD COLUMN canonical_payload JSONB;
```

Update the Rust `WebhookDelivery` model and repository mapping so the payload can be loaded and deserialized as `CanonicalWebhookPayload`.

`canonical_payload` is immutable once set. For an upsert on the same `webhook_provider_id`, use write-once semantics:

```sql
canonical_payload = COALESCE(pay.webhook_delivery.canonical_payload, EXCLUDED.canonical_payload)
```

If an existing non-null payload differs from the newly generated payload for the same provider event, treat that as a correctness error and log it loudly. The same provider event must not produce two different app callbacks.

### 2. Add an explicit atomic processing boundary

Do not try to achieve this by only reordering current repo calls. Current helpers such as `mark_webhook_processed` and `create_webhook_delivery` open and commit their own transactions, so they cannot provide the required atomicity.

Add a new DB/application boundary for provider webhook processing, implemented with one `begin_app_tx` transaction. That boundary must:

1. Lock the `webhook_provider` row with `FOR UPDATE`.
2. Exit idempotently if the row is already `processed` or `suppressed`.
3. Resolve/enrich the provider event.
4. Apply lifecycle/payment mutation using tx-scoped repository operations.
5. Decide whether an app callback should be emitted.
6. If a callback should be emitted, insert `webhook_delivery` with `canonical_payload` in the same transaction.
7. Mark `webhook_provider.processed = true` in the same transaction.
8. Commit.

This likely requires tx-scoped versions of the subscription/payment/webhook mutation methods currently used by `process_webhook`. The important point is the ownership boundary: the processing commit must be one transaction, not a sequence of independently committing repo calls.

### 3. Stop pre-creating empty delivery rows in ingress

Ingress should only create or find the provider inbox row. It should not create `webhook_delivery` before processing.

New normal flow:

1. Insert or dedupe `webhook_provider`.
2. Spawn/call the atomic processor for that provider row.
3. The atomic processor creates a delivery row only if it has a canonical callback payload to send.
4. A delivery worker forwards stored payloads after commit.

Suppressed, stale, unhandled, or no-callback events should not leave pending empty delivery rows. If an audit row is needed for no-callback cases, it must be terminal and excluded from `list_pending_webhook_deliveries`.

### 4. Forward from the stored outbox payload only

Change forwarding so it loads `canonical_payload` from `webhook_delivery` by delivery id and sends that payload.

For rows with non-null `canonical_payload`:

- do not call `build_canonical_payload`;
- do not merge with the current subscription row;
- do not re-run lifecycle stale suppression against current DB state;
- only update delivery attempt/dead-letter state.

Stale suppression belongs before outbox insertion. The rule is:

```text
accepted event -> durable outbox row -> send stored payload
stale/no-op event -> no callback outbox row
```

This is what prevents accepted intermediate lifecycle callbacks from being flattened or skipped by newer DB state.

### 5. Change retry behavior to replay stored payloads

`retry_webhooks` should process only two cases:

1. **Unprocessed provider row with no committed outbox**: run the atomic processor. If it commits a callback outbox row, the worker can send it after commit.
2. **Existing delivery row with stored payload**: forward the stored payload and update retry state.

For processed rows, retry must not rebuild the canonical payload. Rebuilds are allowed only for legacy rows during a one-time migration path described below.

### 6. Cover scheduler and synthetic callbacks

Scheduler-generated callbacks must use the same durable outbox semantics as provider webhooks.

Either:

- route scheduler callbacks through the same provider-row + delivery-row transaction, storing `canonical_payload` and marking the synthetic provider row processed; or
- create a separate synthetic outbox path with equivalent guarantees.

Do not leave scheduler callbacks on the current create-and-forward path where a failed immediate forward can leave an unprocessed provider row without a stored callback payload.

### 7. Handle legacy rows deterministically

`canonical_payload` is nullable only for migration compatibility. New rows created by the atomic processor must always set it when a callback is required.

For existing pending rows with `canonical_payload IS NULL`, choose one deterministic path:

- **Preferred**: run a one-time backfill for pending processed deliveries, store the rebuilt payload, then make forwarding require stored payloads.
- **Acceptable**: on first retry only, rebuild the payload, persist it with `UPDATE ... WHERE canonical_payload IS NULL`, then forward the stored value.

Do not rebuild legacy payloads on every retry. Once rebuilt, retries must use the stored payload.

## What Does Not Change

- `pay.webhook_provider` remains the provider webhook inbox and dedupe source.
- Provider dedupe by `(app_id, provider, provider_webhook_id)` remains unchanged.
- Existing delivery retry/dead-letter fields remain the retry state source.
- `build_canonical_payload` may remain as the producer for processing-time payloads and one-time legacy backfill.
- HTTP forwarding remains outside the processing transaction.

## What Must Change

- Processing, outbox insertion, and `processed = true` must commit together.
- New delivery rows must carry the canonical payload when they are created.
- Forwarding must send stored payloads, not caller-provided in-memory payloads.
- Forwarding must not suppress an already accepted outbox event because newer subscription state exists.
- Scheduler/synthetic callback paths must persist payloads before forwarding.

## Verification

- `cargo check` and `cargo clippy` pass.
- A crash injected before the atomic transaction commits leaves no partial state mutation, no processed mark, and no callback outbox row.
- A crash after commit but before HTTP forward leaves `webhook_provider.processed = true`, a delivery row with `canonical_payload`, and `forwarded = false`.
- Retrying a processed delivery sends exactly the stored `canonical_payload`, even if subscription state changed later.
- An accepted older event followed by a newer event still sends both stored callbacks in delivery order or retry order; the older accepted outbox row is not suppressed at forward time.
- Legacy `canonical_payload IS NULL` rows are rebuilt at most once and then replayed from storage.

## Recommended Checks Before Implementing

From `AGENTS.md` → Bridge Tier 3 checks:

- `.agents/checks/bridge-webhook-idempotency.md`
- `.agents/checks/bridge-subscription-lifecycle.md`
- `.agents/checks/bridge-payment-side-effects.md`
- `.agents/checks/bridge-payment-identity.md`
- `.agents/checks/bridge-release-risk.md`
