# Bridge Architectural Review — 2026-06-24

Scope: high-impact architectural risks only. This intentionally excludes small fixes, tracing/logging nits, and PII-only observations unless they expose a larger design problem.

## Executive Summary

Bridge has strong stated principles around idempotency, stale-event suppression, provider normalization, and callback delivery. The biggest risks are not local Rust style issues; they are system-level correctness risks around identity, durable processing, worker coordination, and state ownership.

The most important theme: Bridge currently looks like a payment event system, but several critical paths still behave like best-effort async request handling. That is risky for a service that must survive provider retries, crashes, horizontal scaling, and multiple users sharing the same provider product identifiers.

## Priority Order

3. Add worker claiming/leases before any callback or provider side effect.
4. Centralize lifecycle state transitions behind typed statuses and a state machine.
5. Move provider-specific semantics behind real provider adapters.
6. Split raw webhook payload retention from dedupe/delivery retention.

---

## 3. Critical — State mutation, processed marking, and callback enqueue are not atomic

Webhook processing mutates subscription/payment state, later marks the webhook processed, then separately enqueues and forwards callback delivery.

### Evidence

- `process_webhook` mutates state through event handlers, then later marks the webhook processed: `src/webhooks/processor.rs::process_webhook`.
- Delivery enqueue happens after processing returns: `src/webhooks/ingress.rs::spawn_process_and_forward_webhook` and `src/webhooks/forwarding.rs::queue_and_forward_webhook`.
- `webhook_delivery` stores retry state, not the immutable canonical payload: `migrations/04_create_webhooks.sql`.
- Retries rebuild canonical payload from current DB state: `src/webhooks/scheduler.rs::retry_webhooks`.

### Why it matters

There are several bad crash windows:

- State changed, but no callback was queued.
- State changed, but `processed=false`, so a duplicate provider delivery can reprocess.
- Retry sends a payload based on current state, not the event originally committed.

This can lose callbacks, duplicate side effects, or skip intermediate lifecycle events.

### Recommended architectural direction

Use a real inbox/outbox transaction:

1. Claim inbox event.
2. Apply lifecycle/payment transition.
3. Insert immutable outbox payload.
4. Mark inbox processed.
5. Let delivery workers send the stored outbox payload.

Callbacks should be event records, not rebuilt projections.

---

## 4. Critical — Multi-instance deployment can duplicate callbacks and provider side effects

Every server instance starts background workers if enabled, but workers do not claim rows before acting.

### Evidence

- All jobs start in each process when `ENABLE_BACKGROUND_JOBS=true`: `src/main.rs` background worker startup.
- Pending deliveries are selected without claim/lease/in-progress state: `src/db/webhooks.rs::list_pending_webhook_deliveries`.
- HTTP callback POST happens before the delivery attempt is persisted: `src/webhooks/forwarding.rs::forward_webhook`.
- Price step-up worker calls provider cancel before marking the row expired: `src/webhooks/scheduler.rs::process_price_step_up_expiry`.

### Why it matters

With two Bridge instances, both can:

- Send the same callback.
- Increment attempts incorrectly.
- Dead-letter prematurely.
- Execute provider cancel/ack/reconcile side effects twice.

### Recommended architectural direction

- Introduce row leasing/claims for delivery and scheduler work.
- Claim before external side effects.
- Use `FOR UPDATE SKIP LOCKED`, `claimed_by`, `claimed_until`, or PostgreSQL advisory locks.
- Make synthetic scheduler event IDs deterministic, not random UUIDs.

---

## 5. High — Subscription lifecycle ownership is split across DB SQL, webhook processor, scheduler, and provider helpers

The docs say subscription status must be typed, unknown statuses explicit, and transitions monotonic. In code, status is still mostly raw strings and transitions live in many places.

### Evidence

- DB status is plain `TEXT`: `migrations/02_create_subscriptions.sql`.
- Rust subscription model exposes raw `String`: `src/db/subscriptions.rs`.
- Reconciliation accepts provider status as `String`: `src/webhooks/scheduler.rs`.
- Some transitions are in large SQL branches, while scheduler also directly mutates lifecycle state: `src/db/subscriptions.rs` and `src/webhooks/scheduler.rs`.

### Why it matters

Invalid states can enter the DB. Different writers can apply different lifecycle rules.

### Recommended architectural direction

- Put lifecycle rules behind one domain state machine.
- Use a `SubscriptionStatus` enum at the application boundary.
- Persist canonical allowed states with a DB `CHECK` or enum.
- Store raw provider state separately.

---

## 6. High — Provider abstraction exists in name, but provider semantics leak everywhere

Bridge claims provider abstraction, but Google/Creem logic is spread through ingress, processor, scheduler, and provider API switches.

### Evidence

- Provider-specific webhook handlers live directly in ingress: `src/webhooks/mod.rs` and `src/webhooks/ingress.rs`.
- Processor contains Google-specific enrichment: `src/webhooks/processor.rs`.
- Provider API switches on provider strings: `src/services/provider_api.rs`.

### Why it matters

Provider-specific identity, timestamp, and status semantics reach the shared lifecycle layer. That is likely why Google product identity leaks into subscription lifecycle identity.

### Recommended architectural direction

Define a provider adapter boundary that emits normalized domain input:

```text
ProviderWebhookAdapter
  verify_signature(...)
  parse_event(...) -> ProviderEvent {
    provider_event_id,
    occurred_at,
    lifecycle_identity,
    product_id,
    raw_status,
    canonical_event,
    side_effect_requirements
  }
```

The application layer should consume `ProviderEvent`, not parse Google/Creem JSON or infer provider identity rules itself.

---

## 7. High — Raw webhook retention can erase operational delivery state

`webhook_delivery` depends on `webhook_provider`, and cleanup deletes old provider rows.

### Evidence

- Delivery rows cascade from provider rows: `migrations/04_create_webhooks.sql`.
- Cleanup deletes all provider rows older than 90 days: `migrations/90_enable_row_level_security.sql::cleanup_old_webhook_provider`.

### Why it matters

Dead-lettered or unresolved callback delivery state can disappear with raw webhook cleanup. That weakens auditability and removes dedupe/delivery evidence.

### Recommended architectural direction

Separate retention for:

- Raw provider payloads.
- Dedupe tombstones.
- Delivery/outbox records.

Raw payload can expire; final delivery outcome and provider event identity should survive longer and should not cascade-delete unresolved work.


---

## Overall Architectural Direction

Bridge should move from best-effort async processing toward a durable event-processing model:

```text
Provider webhook
      |
      v
Durable inbox row
      |
      v
Claimed processor transaction
      |
      +--> lifecycle/payment state transition
      +--> immutable callback outbox row
      +--> inbox marked processed
      |
      v
Claimed delivery worker
      |
      v
App callback endpoint
```

This would give Bridge a cleaner source of truth for payment lifecycle events, safer horizontal scaling, and better crash recovery.
