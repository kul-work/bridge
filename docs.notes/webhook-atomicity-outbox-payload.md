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

Make webhook processing crash-safe by committing subscription/payment mutation, callback outbox payload creation, and provider inbox processed/suppressed state as one durable decision.

## Mechanism at a glance

```text
Provider webhook
      |
      v
pay.webhook_provider inbox row
      |
      |  provider ACK only after the inbox row is durable;
      |  unprocessed inbox rows are recoverable work
      v
processor/retry worker claims inbox row
      |
      v
single DB transaction
  - lock provider webhook row
  - lock or conditionally update the subscription/payment identity
  - reject/suppress stale or invalid event before side effects
  - apply subscription/payment transition
  - if app callback is required, insert immutable outbox payload
  - mark provider webhook processed
      |
      v
commit
      |
      v
delivery worker claims outbox row
      |
      v
send stored outbox payload
```

Once an event has an outbox row, delivery must send that stored event. It must not rebuild the payload from current DB state or re-run lifecycle stale checks against newer subscription state.

## Already-decided — not re-litigating

- Keep `pay.webhook_provider` as the provider inbox and dedupe source; do not introduce a second provider-inbox table for this fix.
- Store the canonical callback payload on `pay.webhook_delivery` instead of creating a separate outbox table; this is the smallest schema change because the existing delivery row already owns retry/dead-letter state.
- Keep HTTP forwarding outside the processing transaction; the atomic boundary is state mutation, outbox insertion, and processed/suppressed marking, not the external callback POST.
- Keep `canonical_payload` nullable during migration only; new callback delivery rows must always store it before they become forwardable.
- Use app-side stale callback suppression rather than Bridge-enforced per-subscription delivery ordering. Bridge must include monotonic event data in every canonical callback, and app callback handlers/specs must reject stale callbacks.

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

If an existing non-null payload differs from the newly generated payload for the same provider event, fail or quarantine that processing attempt and emit an operator-visible alert/metric. The same provider event must not produce two different app callbacks.

After legacy rows are migrated or handled, add a guard so new pending callback deliveries cannot be forwardable with `canonical_payload IS NULL`. This can be a DB constraint if the table shape makes it practical, or an explicit repository invariant checked by the insert/claim path. The end state is that NULL payloads are legacy-only, not a normal retry mode.

Phase 1 must also add or map durable provider-inbox processing state for the outcome taxonomy below. Do not overload only `processed` and `suppressed` if they cannot represent the difference between terminal no-callback, poison, and retryable failure. The durable state needs at least:

- processing outcome/status;
- processing attempt count;
- next retry time/backoff state;
- last processing error or diagnostic code;
- claim owner/claim expiry for processor workers.

### 2. Add an explicit atomic processing boundary

Do not try to achieve this by only reordering current repo calls. Current helpers such as `mark_webhook_processed` and `create_webhook_delivery` open and commit their own transactions, so they cannot provide the required atomicity.

Add a new DB/application boundary for provider webhook processing, implemented with one `begin_app_tx` transaction. That boundary must:

1. Lock the `webhook_provider` row with `FOR UPDATE`.
2. Exit idempotently if the row is already `processed` or `suppressed`.
3. Use only deterministic, already-available data inside the transaction. Do not hold the transaction open across provider API calls or other network I/O.
4. Lock the affected subscription/payment identity, or apply the lifecycle mutation with a conditional timestamp/version update, so stale suppression and outbox insertion are one atomic decision for that identity.
5. Apply lifecycle/payment mutation using tx-scoped repository operations.
6. Decide whether an app callback should be emitted.
7. If a callback should be emitted, insert `webhook_delivery` with `canonical_payload` in the same transaction.
8. Mark `webhook_provider.processed = true`, or mark a terminal suppressed/no-callback state, in the same transaction.
9. Commit.

If provider enrichment requires network I/O, do it before the transaction and persist enough result data on the inbox row for deterministic processing, or leave the inbox row unprocessed with a retryable error state. The transaction itself should only validate and commit deterministic state.

Inbox rows must be claimed before pre-transaction enrichment or deterministic snapshot preparation. The `FOR UPDATE` lock inside the processing transaction is still required, but it is not the first concurrency guard; a processor claim prevents multiple workers from doing the same enrichment/work for one inbox row.

Processing outcomes must be explicit:

- **processed with callback**: state changed and an outbox payload was inserted;
- **processed without callback**: event was valid but did not require app delivery;
- **suppressed terminally**: stale, duplicate-after-dedupe, or intentionally ignored event that should not retry;
- **poison terminally**: invalid/unprocessable event that should not retry without operator intervention;
- **retryable failure**: enrichment, transient DB, or dependency failure that should remain claimable by the processor worker.

Retryable enrichment failures must persist attempts, next retry time, and last error so ACKed provider events do not hot-loop. After the retry budget or poison criteria is reached, the row should move to the poison terminal state for operator review instead of being silently marked processed.

For stale suppression, define the lock target even when the subscription/payment row does not exist yet. Use one of these implementation mechanisms and document the chosen one in the phase diff:

- an advisory lock keyed by `(app_id, provider, lifecycle identity)`;
- a single conditional UPSERT keyed by the natural subscription/payment identity and event timestamp;
- a serializable transaction with bounded retry.

Do not rely on locking an existing subscription row alone, because first events for a new subscription may not have a row to lock.

Non-callback side effects are out of the atomic callback transaction unless they get their own deterministic side-effect record. Provider acknowledgement to the webhook sender remains the HTTP ACK after durable inbox insert. Other side effects, such as Google Play purchase acknowledgement, lifecycle/admin emails, or future provider calls, must either:

- be represented as separate idempotent outbox work with deterministic keys and claim-before-side-effect; or
- run only after the atomic processing commit from a durable record that makes duplicate/lost side effects safe.

Do not perform non-callback network side effects inside the processing transaction.

This likely requires tx-scoped versions of the subscription/payment/webhook mutation methods currently used by `process_webhook`. The important point is the ownership boundary: the processing commit must be one transaction, not a sequence of independently committing repo calls.

### 3. Stop pre-creating empty delivery rows in ingress

Ingress should only create or find the provider inbox row. It should not create `webhook_delivery` before processing.

New normal flow:

1. Insert or dedupe `webhook_provider`.
2. Return the provider HTTP ACK only after the provider inbox row is durable. The atomic processor may run inline or asynchronously, but an unprocessed durable inbox row must be treated as recoverable work.
3. The atomic processor creates a delivery row only if it has a canonical callback payload to send.
4. A delivery worker forwards stored payloads after commit.

Suppressed, stale, unhandled, or no-callback events should not leave pending empty delivery rows. If an audit row is needed for no-callback cases, it must be terminal and excluded from `list_pending_webhook_deliveries`.

This ingress change belongs with the atomic processor rollout, not after it. Do not enable ACK-after-inbox-only behavior in production until the unprocessed-inbox recovery worker and claim path are deployed and verified.

### 4. Forward from the stored outbox payload only

Change forwarding so it loads `canonical_payload` from `webhook_delivery` by delivery id and sends that payload.

Forwarding workers must claim a delivery row before making the HTTP callback side effect. Use the existing project-preferred lease pattern or add `FOR UPDATE SKIP LOCKED`/lease columns so multiple Bridge instances cannot send the same stored payload concurrently or race attempt/dead-letter updates.

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

Delivery ordering is handled by app-side stale suppression, not Bridge-enforced per-subscription delivery ordering. Every canonical payload must carry enough monotonic data (`timestamp_epoch_ms`, subscription version/event id, and event type) for app backends to ignore stale callbacks delivered after newer ones. Bridge docs/specs for app callback consumers must be updated in the same implementation phase that enables stored-payload replay. The plan must not rely on retry order matching lifecycle order.

### 5. Change retry behavior to replay stored payloads

`retry_webhooks` should process only two cases:

1. **Unprocessed provider row with no committed outbox**: run the atomic processor. If it commits a callback outbox row, the worker can send it after commit.
2. **Existing delivery row with stored payload**: forward the stored payload and update retry state.

Both cases need row claiming/leases before side effects: processor workers claim inbox rows before mutation, and delivery workers claim outbox rows before HTTP forwarding.

For processed rows, retry must not rebuild the canonical payload. Rebuilds are allowed only for legacy rows during a one-time migration path described below.

### 6. Cover scheduler and synthetic callbacks

Scheduler-generated callbacks must use the same durable outbox semantics as provider webhooks.

Prefer routing scheduler callbacks through the same provider-row + delivery-row transaction, storing `canonical_payload` and marking the synthetic provider row processed. Synthetic provider rows must use deterministic identities derived from the scheduler action, app/subscription identity, and effective lifecycle timestamp or billing period. A retry of the same scheduled action must find the same synthetic inbox/outbox work item, not mint a new event.

Use a separate synthetic outbox path only if the provider-row model cannot represent the scheduler event without misleading provider audit data. If a separate path is used, it must still provide the same properties: deterministic idempotency key, claim-before-side-effect, stored canonical payload, terminal/retryable states, and no forwardable empty delivery row.

Do not leave scheduler callbacks on the current create-and-forward path where a failed immediate forward can leave an unprocessed provider row without a stored callback payload.

### 7. Handle legacy rows deterministically

`canonical_payload` is nullable only for migration compatibility. New rows created by the atomic processor must always set it when a callback is required.

For existing pending rows with `canonical_payload IS NULL`, choose one deterministic path:

- **Preferred**: run a one-time backfill for pending processed deliveries, store the rebuilt payload, then make forwarding require stored payloads.
- **Acceptable**: on first retry only, rebuild the payload, persist it with `UPDATE ... WHERE canonical_payload IS NULL`, then forward the stored value.

Do not rebuild legacy payloads on every retry. Once rebuilt, retries must use the stored payload.

After the migration path has run, forwarding should reject or quarantine a forwardable row whose `canonical_payload` is still NULL instead of rebuilding from current subscription state.

## What Does Not Change

- `pay.webhook_provider` remains the provider webhook inbox and dedupe source.
- Provider dedupe by `(app_id, provider, provider_webhook_id)` remains unchanged.
- Existing delivery retry/dead-letter fields remain the retry state source.
- `build_canonical_payload` may remain as the producer for processing-time payloads and one-time legacy backfill.
- HTTP forwarding remains outside the processing transaction.

Retention does need to be checked as part of implementation: durable delivery/outbox rows and final delivery outcomes must not disappear just because raw provider payload retention cleanup removes old inbox payloads. If `webhook_delivery` continues to reference `webhook_provider`, cleanup must preserve unresolved/dead-lettered delivery evidence or split raw payload cleanup from inbox/delivery identity retention.

Intermediate phases are not production-deployable until retention/FK behavior is safe for the new outbox durability contract, or until raw provider cleanup is disabled for affected rows. Stored payloads on `webhook_delivery` are only truly durable if cleanup cannot cascade-delete unresolved or final delivery evidence unexpectedly.

## What Must Change

- Processing, outbox insertion, and `processed = true` must commit together.
- New delivery rows must carry the canonical payload when they are created.
- Provider ACK timing must be explicit: ACK after durable inbox insert is allowed only because the durable unprocessed inbox row is recoverable processor work.
- Stale suppression must be atomic against the affected subscription/payment identity, not only against the provider webhook row.
- Forwarding must send stored payloads, not caller-provided in-memory payloads.
- Forwarding must not suppress an already accepted outbox event because newer subscription state exists.
- Processor and delivery workers must claim rows before state mutation or HTTP callback side effects.
- Scheduler/synthetic callback paths must persist payloads before forwarding.
- Scheduler/synthetic callback paths must use deterministic idempotency keys.
- Non-callback side effects must be moved to deterministic side-effect records or otherwise made idempotent and recoverable outside the processing transaction.
- App callback contracts must reject stale callbacks using monotonic payload data; Bridge will not guarantee per-subscription callback delivery order.

## Implementation Phases

Keep each implementation phase small enough to review independently. Do not mix behavior changes from later phases into earlier phases.

### Phase 1 — Schema and repository primitives

- Add `canonical_payload` and any claim/lease fields or indexes needed for inbox/outbox workers.
- Add or map durable provider-inbox processing state for terminal no-callback, terminal poison, and retryable failure outcomes, including attempts, next retry, last error, and claim expiry.
- Update `WebhookDelivery` mapping and add repository methods for write-once payload insert/update.
- Add claim-before-work query primitives for provider inbox rows and delivery rows.
- Add the legacy-null guard shape, while keeping migration compatibility for existing rows.
- Add the retention/FK migration or cleanup guard needed so delivery/outbox rows are not cascade-deleted by raw provider payload cleanup.

Verification focus: migration applies cleanly; repository tests cover write-once payload behavior, claim exclusivity, processing outcome persistence, retry/backoff state, retention cleanup safety, and NULL payload guards.

### Phase 2 — Atomic provider processor boundary

- Introduce the tx-scoped processing boundary that locks the provider row and affected subscription/payment identity.
- Move state mutation, outbox insertion, and processed/suppressed marking into one transaction.
- Keep provider/network enrichment outside the transaction or persist retryable enrichment state before processing.
- Record explicit terminal and retryable processing outcomes.
- Claim inbox rows before enrichment/snapshot work, then re-lock the row inside the transaction before committing the processing decision.
- Choose and implement the stale-suppression create-path guard: advisory lifecycle lock, conditional UPSERT, or serializable transaction with retry.
- Stop ingress from pre-creating empty delivery rows as part of this rollout.

Verification focus: crash-window tests before commit and after commit, stale suppression races, and idempotent reprocessing of already terminal rows.

### Phase 3 — Forwarding and retry from stored payloads

- Change forwarding to claim delivery rows and send `canonical_payload` only.
- Change retry logic to process unprocessed inbox rows or forward claimed stored outbox rows.
- Remove normal-path rebuilds from current subscription state; allow rebuild only in the legacy path.
- Ensure callback payloads carry enough monotonic data for app-side stale suppression.
- Update app-facing callback docs/specs to require stale callback rejection. Coordinate downstream app changes if existing handlers apply callbacks blindly.
- Add deterministic side-effect records or equivalent idempotency/recovery for non-callback side effects that are currently coupled to webhook processing.

Verification focus: multi-instance duplicate-send prevention, stored-payload replay after subscription changes, app-side stale contract coverage, non-callback side-effect idempotency, and out-of-order retry safety.

### Phase 4 — Scheduler/synthetic callback parity

- Route scheduler callbacks through the same durable outbox semantics, preferably via deterministic synthetic provider rows.
- Use deterministic synthetic event identity for scheduler action, app/subscription identity, and effective lifecycle timestamp or billing period.
- Ensure scheduler retries find existing work instead of creating duplicate callbacks.

Verification focus: scheduler crash/retry idempotency and no forwardable empty delivery rows.

### Phase 5 — Legacy cleanup, retention, and release checks

- Backfill or first-retry-persist legacy `canonical_payload IS NULL` rows, then reject/quarantine remaining forwardable NULL payload rows.
- Confirm retention/FK cleanup preserves durable delivery outcomes and unresolved/dead-lettered work. This is a release gate even if the migration landed in Phase 1.
- Run Bridge Tier 3 checks and update docs/specs that describe webhook delivery behavior.

Verification focus: legacy rows rebuild at most once, retention cleanup preserves delivery evidence, and full Bridge payment/webhook checks pass.

## Verification

- `cargo check` and `cargo clippy` pass.
- A crash injected before the atomic transaction commits leaves no partial state mutation, no processed mark, and no callback outbox row.
- A crash after commit but before HTTP forward leaves `webhook_provider.processed = true`, a delivery row with `canonical_payload`, and `forwarded = false`.
- Retrying a processed delivery sends exactly the stored `canonical_payload`, even if subscription state changed later.
- Concurrent provider rows for the same subscription/payment identity cannot both pass stale suppression incorrectly; the older event either loses the conditional update/lock decision or is accepted before the newer event with a stored payload.
- Delivery workers on multiple Bridge instances cannot send the same outbox row concurrently.
- An accepted older event followed by a newer event still sends both stored callbacks, and app-visible payload ordering/version data prevents stale app-side state regression if retries deliver out of lifecycle order.
- Synthetic scheduler callbacks retry to the same deterministic inbox/outbox identity instead of creating duplicate events.
- Legacy `canonical_payload IS NULL` rows are rebuilt at most once and then replayed from storage.
- New forwardable delivery rows with `canonical_payload IS NULL` are rejected by tests/invariants after the migration path.

## Recommended Checks Before Implementing

From `AGENTS.md` → Bridge Tier 3 checks:

- `.agents/checks/bridge-webhook-idempotency.md`
- `.agents/checks/bridge-subscription-lifecycle.md`
- `.agents/checks/bridge-payment-side-effects.md`
- `.agents/checks/bridge-payment-identity.md`
- `.agents/checks/bridge-release-risk.md`
