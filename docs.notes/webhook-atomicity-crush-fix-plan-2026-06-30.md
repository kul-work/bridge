# Webhook Atomicity - Crush Fix Plan

Status: proposal, not implemented.

Context:

- Source issue: `docs.notes/architectural-review-2026-06-24.md`, item `3. Critical - State mutation, processed marking, and callback enqueue are not atomic`.
- Branch reviewed: `webhook-atomicity-outbox-payload`. Decision: **reject, do not merge.** It is a broad webhook/payment refactor (32 files, +4795/-1496) with OTP refund regressions introduced by the new outbox primitive, not a surgical fix.
- Existing related doc: `docs.notes/webhook-atomicity-codex-strategy-2026-06-30.md`. This plan overlaps Patch B with that doc's Phase 1; Patch A here is the multi-instance hygiene complement.

## What is actually broken on `dev` right now

Three concrete windows remain open on `dev`, in increasing severity:

1. **Retry rebuilds from current state, not the committed event.** Pending delivery retry path calls `build_canonical_payload` (`src/webhooks/scheduler.rs:162`), which recomputes the app callback from current DB state. If state has advanced (another event landed between ingestion and retry), the retry emits a callback reflecting newer state and silently drops the intermediate event the app was supposed to observe. This is the durable correctness gap behind item #3.
2. **Delivery retry has no claim.** `list_pending_webhook_deliveries` (`src/db/webhooks.rs:165`) is a plain SELECT, no `FOR UPDATE SKIP LOCKED`, no lease. With two permanent Railway nodes running background workers in round-robin, both can pick the same pending delivery and forward it twice. Note that the inbox path already has a claim primitive to copy from: `claim_unprocessed_webhook_providers` (`src/db/webhooks.rs:191`) with `FOR UPDATE SKIP LOCKED` and `recovery_claimed_at`.
3. **Mutation and `mark_processed` are not in one transaction.** `process_webhook` applies lifecycle transitions via event handlers, then `mark_webhook_processed` runs as a separate statement. A crash between the two leaves state changed with `processed=false`. This is the genuinely hard one: Google Play acknowledgement is an HTTP side effect interleaved with DB mutation (`src/webhooks/processor/event_handlers.rs:543`, `:1256`), which forbids a single long DB transaction. Patch C below is the long-term fix and out of scope for this plan.

## Household idempotency finding

Investigated the receiving app at `c:\share\tyde\household`. Verdict: **household's callback handler is idempotent.**

- Dedupe key: `webhook_callbacks.event_id UNIQUE` (`migrations/08_create_webhook_callbacks_table.sql`).
- Ingestion inserts with `INSERT ... ON CONFLICT (event_id) DO NOTHING` inside a single transaction **before** any state mutation (`src/db.rs:225-248`).
- `apply_premium_update` runs only `if inserted` (`src/db.rs:252`); all UPDATEs are set-value, not accumulators, with an additional `COALESCE(last_bridge_event_ms, 0) < $3` stale-event guard.
- A duplicate Bridge callback returns `200 { received: true }` with outcome label `"duplicate"` / `"duplicate_event_id"` and performs no side effects.

Implication: a duplicate Bridge callback due to the missing delivery claim (window 2 above) does not harm Household. It does produce duplicate admin entries, log noise, and wasted bandwidth, so Patch A below is real hygiene, not window-dressing. Any other consumer app that is not idempotent would be exposed; patch A is the protective fix for that class.

## Plan

Two patches, each a standalone, reviewable, backward-compatible PR. Phased execution: ship Patch B first, verify, then Patch A.

### Patch B - Durable callback replay (closes window 1)

Priority: correctness. Ships first.

Goal: a retried delivery sends the payload that was committed with the event, not a projection of current state.

Scope:
- migration: add nullable `pay.webhook_delivery.canonical_payload JSONB` (NULL accepted for legacy rows).
- update `WebhookDelivery` model mapping in `src/db/webhooks.rs`.
- in the ingress happy path (`spawn_process_and_forward_delivery` in `src/webhooks/ingress.rs`), after `mark_webhook_processed`, persist the serialized canonical payload onto the delivery row in the same logical step.
- in the scheduler retry path (`src/webhooks/scheduler.rs:162`), read `canonical_payload` first; fall back to `build_canonical_payload` only when the stored field is NULL (legacy rows). No behavior change for rows that already have a payload.
- add a regression test: ingest an event, advance state with a second event, retry the first delivery, assert the retried callback reflects the first event, not the second.

Non-goals:
- no outbox table, no atomic transition primitive, no new transaction semantics.
- no changes to `build_canonical_payload` or to event handler behavior.
- no schema change that breaks existing rows (`NULL` default keeps the migration safe).

Effort estimate: migration + `db::webhooks.rs` (read/write) + `webhooks::scheduler.rs` (read path) + `webhooks::ingress.rs` (write step) + one test. Under 5 files.

Risk: low. Backward-compatible. Retry behavior improves; existing rows keep working via the rebuild fallback.

### Patch A - Delivery claim (closes window 2)

Priority: hygiene on `dev`, protective for any non-idempotent consumer. Ships after Patch B is verified.

Goal: no two workers forward the same pending delivery concurrently.

Scope:
- migration: add `claimed_by TEXT` and `claimed_until TIMESTAMPTZ` to `pay.webhook_delivery`.
- in `src/db/webhooks.rs`, add `claim_pending_webhook_deliveries(app_id, owner, lease_seconds, limit)` mirroring the inbox claim primitive: `FOR UPDATE SKIP LOCKED` over candidate rows, then `UPDATE ... SET claimed_by, claimed_until ... RETURNING *`.
- in `src/webhooks/scheduler.rs`, replace the plain `list_pending_webhook_deliveries` call with `claim_pending_webhook_deliveries`; skip rows whose lease is still held by another owner.
- release the claim (clear `claimed_by`/`claimed_until`) after forward completes, or let a short lease expire naturally for the crash case.
- add a test: two concurrent claimants cannot claim the same delivery; a dead lease becomes re-claimable after expiry.

Non-goals:
- no change to forward logic, dead-letter policy, attempt counting, or HTTP semantics.
- no scheduler refactor, no ack-row claiming (that lives in the rejected branch and is out of scope here).
- no synthetic event ID changes.

Effort estimate: migration + `db::webhooks.rs` (new claim fn) + `webhooks::scheduler.rs` (call site) + one test. Under 4 files.

Risk: low. Pattern is already proven in `claim_unprocessed_webhook_providers`. The only operational change is that workers stop racing on the same row.

### Patch C - Out of scope (noted for the record)

Mutating subscription/payment transition + `mark_processed` + outbox insert in one transaction would close the remaining crash window between mutation and the processed flag. Blocked by the HTTP acknowledgement side effect interleaved with DB mutation in `event_handlers.rs`. Resolving it requires splitting acknowledgment out of the mutation step (ack is best-effort today; `retry_google_play_subscription_acknowledgements` is the source of truth) and is the scope the rejected branch attempted. Not a quick win. Defer until Patch B and Patch A are shipped and stable.

## Salvage from the rejected branch

Based on `docs.notes/webhook-atomicity-salvage-notes.md` and independent verification of the branch:

- **Take**: item #1 only - the `src/services/provider_api.rs` change making unknown Google Play subscription statuses an explicit provider error instead of silently mapping to `active`. Independent of the outbox refactor, small, correct behavior. Cherry-pick manually onto `dev`; run Google status-mapping tests after.
- **Keep, rewritten**: item #3 architecture note, but only with a header like `Status: discarded implementation / future architecture reference, not current behavior`.
- **Do not take**: the payment outbox primitives, event_handlers refactor, lifecycle module deletions, scheduler claim hunks, subscription status CHECK migrations, advisory locks. All entangled with the observed OTP refund regressions.

## Verification gate before each patch merges

- `cargo check 2>&1 && echo EXIT: %ERRORLEVEL%`
- `cargo clippy` on touched files
- relevant test suite: `./tests/gpbi/run-all-otp-tests.sh`, `./tests/creem/run-all-otp-tests.sh`, `./tests/test-net-creem-callback-body.sh`, plus the new regression test added in the patch
- admin dashboard sanity: no new duplicate delivery entries beyond existing legacy rows

## Open questions for the user

1. Confirm the phased order: Patch B first, Patch A second.
2. Any other consumer app besides Household that callbacks to Bridge? If yes, that app's idempotency must be verified before deciding whether Patch A is hygiene or urgent.
3. Lease duration preference for `claimed_until`: short (e.g. 30s, fast reclaim on crash) vs longer (e.g. 5min, safer for slow app callbacks)?