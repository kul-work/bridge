# Bridge Architectural Review — 2026-06-24

Scope: high-impact architectural risks only. This intentionally excludes small fixes, tracing/logging nits, and PII-only observations unless they expose a larger design problem.

## Executive Summary

Bridge has strong stated principles around idempotency, stale-event suppression, provider normalization, and callback delivery. The biggest risks are not local Rust style issues; they are system-level correctness risks around identity, durable processing, worker coordination, and state ownership.

The most important theme: Bridge currently looks like a payment event system, but several critical paths still behave like best-effort async request handling. That is risky for a service that must survive provider retries, crashes, horizontal scaling, and multiple users sharing the same provider product identifiers.

## Priority Order

1. Correct lifecycle identity, especially Google `purchase_token` vs `subscription_id`.
2. Introduce a durable inbox/outbox around webhook processing and callback delivery.
3. Add worker claiming/leases before any callback or provider side effect.
4. Centralize lifecycle state transitions behind typed statuses and a state machine.
5. Move provider-specific semantics behind real provider adapters.
6. Split raw webhook payload retention from dedupe/delivery retention.
7. Centralize production startup validation for config, URLs, and database role separation.

---

## 1. Critical — Google subscription identity is not safe enough

Bridge often treats `subscription_id` as if it identifies one subscription lifecycle. For Google Play, that value can represent the product/base subscription, not one user's purchase lifecycle.

### Evidence

- Subscription uniqueness is `(app_id, external_user_id, subscription_id, provider)`, meaning many users can share the same `subscription_id`: `migrations/02_create_subscriptions.sql`.
- Reconciliation updates by only `app_id + subscription_id`: `src/db/subscriptions.rs::update_subscription_status`.
- Forwarding stale suppression looks up by `subscription_id` only before deciding whether to suppress a callback: `src/webhooks/forwarding.rs::forward_webhook`.

### Why it matters

A reconciliation or stale-forward decision for one Google purchase can affect another user on the same product. That is entitlement corruption and cross-user state risk, not a small bug.

### Recommended architectural direction

- Model provider lifecycle identity explicitly.
- For Google, lifecycle identity should be purchase-token or internal-row based, not product/subscription-id based.
- Ban generic lifecycle mutations keyed only by `subscription_id` unless the provider guarantees it is unique per customer lifecycle.
- Make mutation APIs accept a `SubscriptionIdentity`/row id or `(app_id, provider, purchase_token)` where appropriate.
- Keep `product_id` separate from lifecycle identity in both schema and application code.

---

## 2. Critical — Webhook ingress ACKs providers before durable processing is guaranteed

Ingress inserts a `webhook_provider` row, spawns an in-memory task, then returns `204` to the provider.

### Evidence

- New webhook is inserted, then async processing is spawned, then `204` is returned: `src/webhooks/ingress.rs::handle_google_play` and `handle_creem`.
- Spawned processing failure is only logged: `src/webhooks/ingress.rs::spawn_process_and_forward_webhook`.
- Retry worker drains `webhook_delivery`, not unprocessed `webhook_provider` inbox rows: `src/webhooks/scheduler.rs::retry_webhooks`.

### Why it matters

If Bridge crashes after inserting `webhook_provider` but before processing/enqueueing delivery, the provider already received success and may never retry. The event can remain permanently unprocessed.

### Recommended architectural direction

- Treat `webhook_provider` as a durable inbox.
- Add a worker that continuously claims unprocessed inbox rows using leases or `FOR UPDATE SKIP LOCKED`.
- Or process state mutation and delivery enqueue synchronously before returning `204`.
- Make `processed=false AND suppressed=false` an operational queue, not just an admin/debug state.

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
- Unknown Google status defaults to `active` in reconciliation normalization: `src/services/provider_api.rs::normalize_google_status`.
- Some transitions are in large SQL branches, while scheduler also directly mutates lifecycle state: `src/db/subscriptions.rs` and `src/webhooks/scheduler.rs`.

### Why it matters

Unknown provider states can grant entitlement. Invalid states can enter the DB. Different writers can apply different lifecycle rules.

### Recommended architectural direction

- Put lifecycle rules behind one domain state machine.
- Use a `SubscriptionStatus` enum at the application boundary.
- Persist canonical allowed states with a DB `CHECK` or enum.
- Store raw provider state separately.
- Unknown provider states should be explicit and non-entitling unless deliberately mapped.

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

## 8. High â€” Production safety policy is split between docs, `main.rs`, and implicit environment discipline

Bridge documents several production requirements, but the runtime enforces only part of them centrally. The current hard startup failure is mostly limited to `MOCK_EXTERNAL_APIS=true` in production. Other launch-critical assumptions still depend on deployment discipline or scattered service behavior.

### Evidence

- `main.rs` rejects `MOCK_EXTERNAL_APIS=true` in production, but there is no central `Config::validate_startup()` equivalent for the rest of the production contract.
- `docs/CONFIGURATION.md` says `ADMIN_DATABASE_URL` is required in production, but `Config::from_env` still treats it as optional and database startup falls back to runtime credentials when it is absent.
- Background workers default to enabled, but production does not fail fast if `ENABLE_BACKGROUND_JOBS=false`, even though retry, reconciliation, cleanup, price-step-up expiry, and pause scheduling are part of Bridge's durable payment operations.
- Admin Clerk configuration is runtime-optional: `ADMIN_CLERK_FRONTEND_API`, `ADMIN_CLERK_ORG_ID`, and `ADMIN_CLERK_AUTHORIZED_PARTIES` are not validated as a production admin boundary at startup.
- Production URL safety is not enforced centrally for database URLs, app callback URLs, email lookup URLs, provider API URLs, admin Clerk issuer, or other outbound/inbound trusted endpoints.
- Production logging safety depends on default filters; startup does not reject explicit debug/raw trace filters such as `BPT-RAW=debug` in a production environment.

### Why it matters

Bridge is a payment system. A bad production environment should fail before serving traffic, not degrade into a subtly unsafe shape. Missing admin DB separation can run migrations with runtime credentials. Disabled workers can stop callback retry/reconciliation. Unsafe URLs can send API keys, callbacks, or provider traffic to the wrong endpoint. Debug/raw trace filters can expose sensitive operational data. These are not staging checklist problems only; they are code-level startup invariants.

### Recommended architectural direction

- Add a central `Config::validate_startup()` and call it before initializing providers, databases, workers, or routes.
- In production, require `ADMIN_DATABASE_URL` and reject runtime migration fallback.
- In production, require `ENABLE_BACKGROUND_JOBS=true` unless an explicit documented maintenance mode is introduced.
- In production, reject `EMAIL_PROVIDER=mock` and require the configured provider credentials needed by the selected provider.
- In production, require a deliberate admin auth boundary: at minimum a production Clerk issuer plus `ADMIN_CLERK_AUTHORIZED_PARTIES`, and preferably `ADMIN_CLERK_ORG_ID` or an explicit immutable admin allowlist.
- In production, reject unsafe trusted URLs: localhost/private/test hosts where inappropriate, plain `http://` for public/provider-facing endpoints, and callback/email-lookup URLs that do not match the intended app boundary.
- In production, reject debug/raw trace filters that can expose sensitive identifiers or provider payload context.
- Treat production config validation as part of Bridge's architecture contract, not as deployment documentation.

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
