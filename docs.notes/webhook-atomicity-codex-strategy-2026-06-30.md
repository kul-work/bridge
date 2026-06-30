# Webhook Atomicity - Codex Strategy

Status: proposal, not implemented.

Context:

- Source issue: `docs.notes/architectural-review-2026-06-24.md`, item `3. Critical - State mutation, processed marking, and callback enqueue are not atomic`.
- Branch reviewed: `webhook-atomicity-outbox-payload`.
- Decision: do not merge that branch. It is a broad webhook/payment/provider refactor, and current evidence includes OTP refund regressions where expected `refunded` remains `success`.

## Goal

Close the real crash window:

1. Provider webhook is durable in `pay.webhook_provider`.
2. Payment/subscription state mutation is applied.
3. Canonical app callback payload is persisted.
4. Provider webhook is marked processed or terminal.

Those decisions must commit together when a callback is required. HTTP forwarding stays outside the transaction.

## Non-goals

- Do not rewrite Google Play lifecycle modules as part of this fix.
- Do not merge scheduler, reconciliation, ack-claim, DB status constraints, and docs rewrites into the same patch.
- Do not make multi-instance worker safety the first patch. It is related, but it is a separate failure mode.
- Do not change callback event semantics while adding durability.

## Current Dev Failure Windows

Crush's plan usefully names the concrete `dev` windows. Keep these as the working diagnosis:

1. Retry rebuilds callbacks from current state by calling `build_canonical_payload`, so an older accepted event can be retried as a newer projection.
2. Delivery retry workers list pending deliveries without a claim, so two background workers can send the same delivery.
3. Webhook state mutation and `webhook_provider.processed = true` are split across transactions.

Window 3 is the actual architectural issue #3. Window 1 is a symptom of the missing stored outbox payload. Window 2 is multi-instance hygiene and should be handled after the stored payload path is correct.

## Rejected Shortcut

Do not ship a patch that only stores `canonical_payload` after `process_webhook` returns and after `mark_webhook_processed` runs.

That would improve retry replay for rows where the write succeeds, but it still leaves the scary crash windows:

- state changed, then crash before `processed=true`;
- `processed=true`, then crash before `canonical_payload` is stored;
- delivery row exists but has no committed event payload.

The first durable-payload patch should therefore include the transaction boundary that commits mutation, payload insert, and processed/terminal state together.

## Why The Rejected Branch Broke OTP Refunds

The branch did not only add an outbox primitive. It deleted the Google Play lifecycle modules and reimplemented OTP refund paths inside `event_handlers.rs`.

The risky shape was:

- `purchase.one_time_refunded` called a new `commit_payment_with_outbox_effect` with a `WebHookPaymentStatusUpdateRequest` for `refunded`;
- the helper treated an atomic `None` result as `should_forward=false` plus `terminal_recorded=true`;
- the payment outbox commit built a `refunded` canonical payload from the request status, not from a verified persisted payment row;
- the payment status update path did not prove that the target payment row was actually changed before committing the outbox decision.

That matches the observed failures where refund tests still saw `pay.payments.status = success`.

Do not copy that shape. The atomic commit helper must return enough information to prove the durable mutation happened, or must fail/quarantine instead of emitting or terminal-recording a refund decision.

## Google Play Acknowledgement Prerequisite

Crush is right that Phase 2 has a hard prerequisite: current event handling interleaves Google Play acknowledgement HTTP calls with DB mutation.

Evidence on current `dev`:

- `src/webhooks/processor/event_handlers.rs:236` and `:295` call `acknowledge_google_play(...)`, which is provider HTTP work.
- `src/webhooks/processor/event_handlers.rs:543` calls subscription acknowledgement from the activation path after DB mutation.
- `src/webhooks/processor/event_handlers.rs:1256` calls one-time acknowledgement after OTP purchase handling.
- `src/webhooks/scheduler.rs:239-282` already has `retry_google_play_subscription_acknowledgements`, which retries acknowledgement from durable payment/subscription state and then marks `acknowledged_at`.

Before the atomic provider-processing transaction, split acknowledgement out of the DB mutation path:

1. Existing webhook handlers should decide whether acknowledgement is needed, but not call Google inside the mutation transaction.
2. The mutation transaction should persist the payment/subscription state and callback outbox.
3. Google Play acknowledgement remains best-effort after commit, backed by the existing acknowledgement retry worker.
4. Later, add row claiming for acknowledgement candidates as its own small patch.

Do not hold a DB transaction open while calling Google Play.

## Household Callback Idempotency Evidence

Household is currently safer than an arbitrary consumer app, but Bridge should not rely on that globally.

Evidence in `C:\share\tyde\household`:

- `migrations/08_create_webhook_callbacks_table.sql:7-10` creates `webhook_callbacks` with `event_id TEXT NOT NULL UNIQUE`.
- `src/db.rs:225-248` inserts callback records with `ON CONFLICT (event_id) DO NOTHING` and records whether the insert happened.
- `src/db.rs:252-255` applies premium updates only when the callback row was newly inserted.
- `src/db.rs:435-535` premium updates use set-value updates with `COALESCE(last_bridge_event_ms, 0) < ...` stale guards.

Implication: duplicate Bridge callback delivery is less dangerous for Household, but delivery claiming is still needed for Bridge correctness and for future apps that may not be equally defensive.

## Recommended Phases

### Phase 1 - Stored callback payload primitive

Add only the smallest schema/runtime primitive needed for durable callback replay:

- migration: add nullable `pay.webhook_delivery.canonical_payload JSONB`;
- update `WebhookDelivery` model mapping;
- add a write-once delivery insert helper:
  - inserts `webhook_delivery` with `canonical_payload`;
  - on duplicate `webhook_provider_id`, accepts only the same payload or an existing NULL legacy row;
  - rejects conflicting non-null payloads.

Keep `canonical_payload` nullable only for legacy rows.

Verification:

- repository test for write-once payload behavior;
- duplicate insert with identical payload is idempotent;
- duplicate insert with different payload fails visibly.

### Phase 2 - Atomic provider processing commit

Add one explicit transaction boundary for provider webhook processing. Do not try to compose existing helpers that each open their own transaction.

Prerequisite: remove Google Play acknowledgement HTTP calls from the mutation path. Acknowledgement should run best-effort after the transaction commits, with the existing acknowledgement retry worker as the recovery source of truth.

The new commit path should:

1. `BEGIN` with app RLS context.
2. Lock `pay.webhook_provider` by id with `FOR UPDATE`.
3. If already `processed` or `suppressed`, return idempotently.
4. Apply the existing payment/subscription mutation using tx-scoped helpers.
5. Build the canonical payload using the same semantics as today.
6. Insert `webhook_delivery` with `canonical_payload` when a callback is required.
7. Mark `webhook_provider.processed = true`.
8. Commit.

For stale/no-forward events:

- mark the provider row terminal/suppressed in the same transaction;
- do not create a forwardable empty delivery row.

Important constraint:

- Do not hold this transaction across provider API calls, email sends, or app callback HTTP calls.

Verification:

- crash-before-commit leaves no partial state and no processed mark;
- crash-after-commit leaves state changed, `processed=true`, `webhook_delivery.forwarded=false`, and a stored payload;
- OTP refund regression proves payment status becomes `refunded` and callback body says `refunded`.

### Phase 3 - Forward from stored payload

Change retry/forwarding for new rows:

- load `webhook_delivery.canonical_payload`;
- deserialize and send exactly that payload;
- do not call `build_canonical_payload`;
- do not re-run stale suppression at forward time for already accepted stored payloads.

Legacy handling:

- `canonical_payload IS NULL` rows are legacy-only;
- either quarantine them or explicitly route unprocessed provider rows through the new processor;
- do not silently rebuild processed legacy rows from current subscription state.

Verification:

- subscription changes after webhook processing do not alter the retried callback body;
- failed callback retry sends the same payload bytes/fields.

### Phase 4 - Delivery claiming for multi-instance workers

Only after the durable payload path is correct, add row claiming for callback delivery:

- nullable `claim_owner`, `claim_expires_at` on `webhook_delivery`;
- claim with `UPDATE ... FROM (SELECT ... FOR UPDATE SKIP LOCKED) ... RETURNING`;
- forward only claimed rows.

This is where multi-instance duplicate callback prevention belongs.

Crush's Household finding is useful here: Household appears idempotent on callback `event_id`, so duplicate Bridge delivery is less dangerous for that app. Do not generalize that to future apps. Patch 4 is still needed for Bridge correctness and for any consumer that is not equally idempotent.

Verification:

- two workers cannot claim the same delivery row;
- expired claim can be reclaimed;
- successful delivery marks forwarded once.

### Phase 5 - Separate small mitigations

Handle these independently, not inside the atomicity patch:

- Google Play unknown subscription status rejection from `src/services/provider_api.rs`;
- Google Play acknowledgement row claiming;
- scheduler synthetic callback deterministic ids;
- subscription status DB constraints.

Each one should have its own diff and focused tests.

## Verification Gate

Before merging each implementation patch:

- `cargo check 2>&1 && echo EXIT: %ERRORLEVEL%`;
- focused Rust tests for the changed repository/processor path;
- OTP refund regression: `./tests/gpbi` cases covering OTP refund must show DB status `refunded`;
- Creem OTP refund regression must show `refunded`, not `success`;
- `./tests/test-net-creem-callback-body.sh` must pass its one-time refund callback body case;
- for delivery claiming, add a concurrent-claim test proving two workers cannot claim the same delivery row.

## Railway Background Job Position

Running only one active background-task node in Railway is a good operational guardrail for now, but it does not fix item 3.

It helps with:

- duplicate callback retry workers;
- duplicate scheduler side effects;
- duplicate provider acknowledgement retries.

It does not help with:

- process crash after state mutation but before callback enqueue;
- process crash after callback enqueue but before processed mark;
- retry rebuilding callbacks from current DB state.

So the recommended production posture is:

1. Short term: one background-worker node active.
2. Code fix: atomic state + outbox + processed transaction.
3. Later: row claiming so multiple background-worker nodes are safe.

## Branch Salvage Decision

From `webhook-atomicity-outbox-payload`, salvage only by manual hunk-level extraction:

- keep: `src/services/provider_api.rs` unknown Google status rejection;
- optional: `src/db/database.rs` comments about `sqlx::migrate!` embedding migrations;
- optional: rewrite `docs.notes/webhook-atomicity-outbox-payload.md` as discarded/future reference.

Do not salvage directly:

- `src/webhooks/processor/event_handlers.rs` refactor;
- `src/webhooks/processor.rs` prebuilt canonical payload changes;
- `src/webhooks/forwarding.rs` stored payload forwarding changes;
- large `src/db/webhooks.rs` outbox additions;
- migrations `96` through `101` as-is;
- deletion of Google Play lifecycle modules;
- docs claiming stored-payload outbox is current production behavior.
