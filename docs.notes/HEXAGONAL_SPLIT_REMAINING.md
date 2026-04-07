# Bridge Hexagonal Split - Remaining Findings

Scope: Bridge only. HiHa is reference-only and is not part of this document.

## Current State

Bridge already has the first useful hexagonal pieces:

- `src/application/` for checkout and verify-purchase use cases
- `src/ports.rs` with `BridgeRepository`
- `src/db/subscriptions.rs` with `SubscriptionWebhookTransition`
- `src/webhooks/processor.rs` mostly rewritten to use repository methods
- `src/webhooks/forwarding.rs` now uses repository methods for app, delivery, suppression, and retry updates
- `src/webhooks/scheduler.rs` now uses repository methods instead of raw SQL
- `src/services/google_play/subscription_lifecycle.rs` is port-driven for subscription state transitions

That means the remaining work is cleanup needed to make the split strict and consistent, not a full rewrite.

## Remaining Findings

### 1. Transaction-heavy flows still reach through `repo.pool()`

These flows still carry SQLx transaction plumbing at the application boundary instead of hiding it behind a narrower port.

Observed coupling:

- `src/webhooks/processor.rs` opens transactions via `repo.pool()` in several event branches
- `src/application/verify_purchase.rs` opens a transaction for payment and subscription writes
- `src/services/google_play/product_lifecycle.rs` opens transactions for OTP purchase/cancel flows

Relevant paths:

- [src/webhooks/processor.rs](C:/share/tyde/bridge/src/webhooks/processor.rs#L691)
- [src/application/verify_purchase.rs](C:/share/tyde/bridge/src/application/verify_purchase.rs#L201)
- [src/services/google_play/product_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/product_lifecycle.rs#L30)
- [src/services/google_play/product_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/product_lifecycle.rs#L87)

Why this matters:

- the port still leaks SQLx details upward
- the application layer still owns transaction plumbing
- these flows are harder to swap or test without the database shape

### 2. A few HTTP handlers still bypass the repository port

Most handlers now go through `BridgeRepository`, but these still call `crate::db::*` directly:

Observed coupling:

- `src/handlers/api_key.rs`
- `src/handlers/subscriptions.rs`
- `src/handlers/users.rs`

Relevant paths:

- [src/handlers/api_key.rs](C:/share/tyde/bridge/src/handlers/api_key.rs#L41)
- [src/handlers/subscriptions.rs](C:/share/tyde/bridge/src/handlers/subscriptions.rs#L88)
- [src/handlers/subscriptions.rs](C:/share/tyde/bridge/src/handlers/subscriptions.rs#L175)
- [src/handlers/users.rs](C:/share/tyde/bridge/src/handlers/users.rs#L114)
- [src/handlers/users.rs](C:/share/tyde/bridge/src/handlers/users.rs#L145)

Why this matters:

- these are the last routine read paths that still depend on DB module shape
- the split is uneven if only webhook flows are port-driven
- read-side logic should move behind the application boundary or a narrower read port

### 3. `BridgeRepository` is still broad and still exposes `pool()`

The port works, but it is still a catch-all and still leaks the database handle.

Relevant paths:

- [src/ports.rs](C:/share/tyde/bridge/src/ports.rs#L21)
- [src/ports.rs](C:/share/tyde/bridge/src/ports.rs#L405)

Why this matters:

- `pool()` keeps transaction plumbing visible to callers
- the port still mixes unrelated concerns: apps, checkout, subscriptions, payments, webhooks, and agent flows
- the next cleanup is splitting by use case or bounded context

## Resolved Since the Last Review

These are no longer leftovers for raw DB access:

- `src/webhooks/forwarding.rs`
- `src/webhooks/scheduler.rs`
- `src/services/google_play/subscription_lifecycle.rs`

They still contain orchestration and business logic, but they are no longer the direct DB holdouts the earlier note described.

## Recommended Next Extraction Order

1. `src/webhooks/processor.rs`
2. `src/application/verify_purchase.rs`
3. `src/services/google_play/product_lifecycle.rs`
4. `src/handlers/api_key.rs`
5. `src/handlers/subscriptions.rs`
6. `src/handlers/users.rs`
7. Split `BridgeRepository` into smaller ports and remove `pool()`

## Short Checklist

- [ ] Hide transaction plumbing behind a narrower port or dedicated unit of work API
- [ ] Move remaining handler read paths behind application or read ports
- [ ] Split `BridgeRepository` into smaller, use-case-specific ports
- [ ] Remove `pool()` from the public port surface if possible
- [ ] Re-run `cargo check` and `cargo test` after each extraction

## Practical Target

If Bridge is fully split, the dependency direction should read as:

`handlers / workers / webhook adapters -> application services -> ports -> DB/provider implementations`

Not:

`handlers / workers -> crate::db::*`

## Bottom Line

Bridge is already partially hexagonal.

The remaining work is to move the last direct handler reads and transaction-heavy flows behind smaller ports so the application layer stops depending on concrete persistence details.
