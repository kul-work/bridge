# Bridge Hexagonal Split - Remaining Findings

Scope: Bridge only. HiHa is reference-only and is not part of this document.

## Current State

Bridge already has the first useful hexagonal pieces:

- `src/application/` for checkout and verify-purchase use cases
- `src/ports.rs` with narrower repository traits and the `with_transaction()` unit-of-work helper
- `src/db/subscriptions.rs` with `SubscriptionWebhookTransition`
- `src/webhooks/processor.rs` mostly rewritten to use repository methods
- `src/webhooks/forwarding.rs` now uses repository methods for app, delivery, suppression, and retry updates
- `src/webhooks/scheduler.rs` now uses repository methods instead of raw SQL
- `src/services/google_play/subscription_lifecycle.rs` is port-driven for subscription state transitions
- The port layer is already split into smaller traits on `Database`; there is no longer a single `BridgeRepository` symbol

That means the remaining work is cleanup needed to make the split strict and consistent, not a full rewrite.

## Remaining Findings

### 1. `Database` no longer exposes its pool publicly

The port split is already in place, and the raw connection pool is now private to `Database`.

Relevant paths:

- [src/db/mod.rs](C:/share/tyde/bridge/src/db/mod.rs#L17)
- [src/ports.rs](C:/share/tyde/bridge/src/ports.rs#L613)
- [src/ports.rs](C:/share/tyde/bridge/src/ports.rs#L977)

Why this matters:

- `pool: PgPool` keeps the handle private to `Database`
- most transaction handling now goes through `with_transaction()`, so the remaining access is internal only
- hiding the field from the public surface makes the dependency direction stricter and makes accidental raw SQL use harder

## Resolved Since the Last Review

These are no longer leftovers for raw DB access:

- `src/webhooks/processor.rs`
- `src/application/verify_purchase.rs`
- `src/services/google_play/product_lifecycle.rs`
- `src/webhooks/forwarding.rs`
- `src/webhooks/scheduler.rs`
- `src/services/google_play/subscription_lifecycle.rs`
- `src/handlers/verify_purchase.rs`
- `src/webhooks/ingress.rs`

They still contain orchestration and business logic, but they are no longer the direct DB holdouts the earlier note described.
Their transaction flow now goes through the `with_transaction()` helper exposed by the repository implementations on `Database` instead of opening SQLx transactions at the application boundary.
In the current code, that means the relevant logic has moved behind narrow repository traits on `Database`.

## Recommended Next Extraction Order

1. Audit any remaining internal callers if you want to push the field fully private later

## Short Checklist

- [x] Hide transaction plumbing behind a narrower port or dedicated unit of work API
- [x] Move remaining handler and middleware database access behind application or read ports
- [x] Split the old monolithic repository surface into smaller traits on `Database`
- [x] Hide `Database.pool` from the public surface if possible
- [x] Re-run `cargo check` and `cargo test` after each extraction

## Practical Target

If Bridge is fully split, the dependency direction should read as:

`handlers / workers / webhook adapters -> application services -> ports -> DB/provider implementations`

Not:

`handlers / workers -> crate::db::*`

## Bottom Line

Bridge is already partially hexagonal.

The remaining work is limited to any internal-only cleanup if you want to push the pool field fully private later.
