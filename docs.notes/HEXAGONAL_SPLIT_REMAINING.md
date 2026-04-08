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

### 1. A few ingress paths still depend on concrete `Database`

Most HTTP handlers and middleware now go through `AppState` and repository traits. The remaining concrete `Database` state usage is concentrated in these ingress paths:

Observed coupling:

- `src/handlers/verify_purchase.rs`
- `src/webhooks/ingress.rs`

Relevant paths:

- [src/handlers/verify_purchase.rs](C:/share/tyde/bridge/src/handlers/verify_purchase.rs#L123)
- [src/webhooks/ingress.rs](C:/share/tyde/bridge/src/webhooks/ingress.rs#L20)
- [src/webhooks/ingress.rs](C:/share/tyde/bridge/src/webhooks/ingress.rs#L173)
- [src/webhooks/ingress.rs](C:/share/tyde/bridge/src/webhooks/ingress.rs#L276)
- [src/webhooks/ingress.rs](C:/share/tyde/bridge/src/webhooks/ingress.rs#L378)

Why this matters:

- these are still coupled to the database module shape instead of a narrow read or use-case port
- the split is uneven if only webhook processor and application flows are port-driven
- ingress-only logic should move behind the application boundary or a narrower port where it is reused

### 2. `Database` still exposes its pool publicly

The port split is already in place, but the raw connection pool still leaks through the public struct field.

Relevant paths:

- [src/db/mod.rs](C:/share/tyde/bridge/src/db/mod.rs#L17)
- [src/ports.rs](C:/share/tyde/bridge/src/ports.rs#L613)
- [src/ports.rs](C:/share/tyde/bridge/src/ports.rs#L977)

Why this matters:

- `pub pool: PgPool` still makes the concrete database handle available to callers
- most transaction handling now goes through `with_transaction()`, so the remaining leak is the public field itself
- hiding the field would make the dependency direction stricter and make accidental raw SQL use harder

## Resolved Since the Last Review

These are no longer leftovers for raw DB access:

- `src/webhooks/processor.rs`
- `src/application/verify_purchase.rs`
- `src/services/google_play/product_lifecycle.rs`
- `src/webhooks/forwarding.rs`
- `src/webhooks/scheduler.rs`
- `src/services/google_play/subscription_lifecycle.rs`

They still contain orchestration and business logic, but they are no longer the direct DB holdouts the earlier note described.
Their transaction flow now goes through the `with_transaction()` helper exposed by the repository implementations on `Database` instead of opening SQLx transactions at the application boundary.
In the current code, that means the relevant logic has moved behind narrow repository traits on `Database`.
- The earlier handler and middleware holdouts now use `AppState` and repository traits instead of the raw database state.

## Recommended Next Extraction Order

1. `src/handlers/api_key.rs`
2. `src/handlers/subscriptions.rs`
3. `src/handlers/users.rs`
4. `src/handlers/agent.rs`
5. `src/handlers/subscriptions_actions.rs`
6. `src/handlers/verify_purchase.rs`
7. `src/webhooks/ingress.rs`
8. Hide `Database.pool` if the codebase can be updated cleanly

## Short Checklist

- [x] Hide transaction plumbing behind a narrower port or dedicated unit of work API
- [ ] Move remaining handler and middleware database access behind application or read ports
- [x] Split the old monolithic repository surface into smaller traits on `Database`
- [ ] Hide `Database.pool` from the public surface if possible
- [ ] Re-run `cargo check` and `cargo test` after each extraction

## Practical Target

If Bridge is fully split, the dependency direction should read as:

`handlers / workers / webhook adapters -> application services -> ports -> DB/provider implementations`

Not:

`handlers / workers -> crate::db::*`

## Bottom Line

Bridge is already partially hexagonal.

The remaining work is to move the last direct handler and middleware reads behind smaller ports and, if possible, stop exposing the raw pool field so the application layer stops depending on concrete persistence details.
