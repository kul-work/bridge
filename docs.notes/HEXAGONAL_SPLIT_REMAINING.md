# Bridge Hexagonal Split - Remaining Findings

Scope: Bridge only. HiHa is reference-only and is not part of this document.

## Current State

Bridge already has the first useful hexagonal pieces:

- `src/application/` for checkout and verify-purchase use cases
- `src/ports.rs` with `BridgeRepository`
- `src/db/subscriptions.rs` with `SubscriptionWebhookTransition`
- `src/webhooks/processor.rs` rewritten to use repository methods instead of raw `crate::db::*` access

That means the remaining work is not a full rewrite. It is the cleanup needed to make the split strict and consistent.

## Remaining Findings

### 1. Webhook forwarding still talks to the database directly

`src/webhooks/forwarding.rs` still loads app, delivery, webhook, and subscription state directly from `crate::db::*`, and it updates retry/suppression state directly too.

Observed coupling:

- load app data from DB
- load webhook delivery from DB
- load subscription for callback suppression checks
- suppress delivery in DB
- update delivery attempts in DB

Relevant paths:

- [src/webhooks/forwarding.rs](C:/share/tyde/bridge/src/webhooks/forwarding.rs#L26)
- [src/webhooks/forwarding.rs](C:/share/tyde/bridge/src/webhooks/forwarding.rs#L29)
- [src/webhooks/forwarding.rs](C:/share/tyde/bridge/src/webhooks/forwarding.rs#L40)
- [src/webhooks/forwarding.rs](C:/share/tyde/bridge/src/webhooks/forwarding.rs#L48)
- [src/webhooks/forwarding.rs](C:/share/tyde/bridge/src/webhooks/forwarding.rs#L54)

Why this matters:

- forwarding is still an application service, but it is coupled to persistence shape
- retry logic and callback delivery are harder to test without the DB schema
- the boundary is not symmetrical with `src/webhooks/processor.rs` yet

### 2. Scheduler still mixes job orchestration with raw queries

`src/webhooks/scheduler.rs` still does direct SQLx queries for apps, deliveries, provider configs, webhook creation, and stale event maintenance.

Observed coupling:

- query enabled apps directly
- query pending deliveries directly
- update delivery attempts directly
- query provider configs directly
- mutate subscriptions directly for reconciliation
- create webhook provider and delivery rows directly

Relevant paths:

- [src/webhooks/scheduler.rs](C:/share/tyde/bridge/src/webhooks/scheduler.rs#L41)
- [src/webhooks/scheduler.rs](C:/share/tyde/bridge/src/webhooks/scheduler.rs#L49)
- [src/webhooks/scheduler.rs](C:/share/tyde/bridge/src/webhooks/scheduler.rs#L74)
- [src/webhooks/scheduler.rs](C:/share/tyde/bridge/src/webhooks/scheduler.rs#L126)
- [src/webhooks/scheduler.rs](C:/share/tyde/bridge/src/webhooks/scheduler.rs#L163)
- [src/webhooks/scheduler.rs](C:/share/tyde/bridge/src/webhooks/scheduler.rs#L516)
- [src/webhooks/scheduler.rs](C:/share/tyde/bridge/src/webhooks/scheduler.rs#L529)

Why this matters:

- the retry worker and reconciliation worker are effectively part of the application layer
- raw queries here duplicate what should become repository methods or job ports
- it is harder to see the domain rules because job orchestration and persistence are interleaved

### 3. Google Play lifecycle orchestration still combines domain decisions and side effects

The Google Play lifecycle modules are the biggest remaining "not fully split" area conceptually.

The issue is not that they exist. The issue is that each handler still bundles several responsibilities:

- interpret the Google Play event
- build normalized subscription state
- persist the record through `state.database`
- apply access changes
- trigger notifications or follow-up actions

Concrete examples:

- `handle_subscription_revoked()` stores the record, deactivates access, and sends email in one flow
- `handle_subscription_restarted()` stores the record and activates access in one flow
- `handle_subscription_cancelled_with_context()` stores the record and updates premium access in one flow
- `handle_subscription_pending()` stores the record and deactivates access in one flow
- `handle_otp_purchased()` creates the DB record and grants access in one flow
- `handle_otp_cancelled()` updates payment state and deactivates access in one flow

Relevant paths:

- [src/services/google_play/subscription_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/subscription_lifecycle.rs#L74)
- [src/services/google_play/subscription_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/subscription_lifecycle.rs#L110)
- [src/services/google_play/subscription_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/subscription_lifecycle.rs#L215)
- [src/services/google_play/subscription_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/subscription_lifecycle.rs#L255)
- [src/services/google_play/subscription_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/subscription_lifecycle.rs#L287)
- [src/services/google_play/subscription_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/subscription_lifecycle.rs#L339)
- [src/services/google_play/subscription_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/subscription_lifecycle.rs#L675)
- [src/services/google_play/subscription_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/subscription_lifecycle.rs#L714)
- [src/services/google_play/product_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/product_lifecycle.rs#L12)
- [src/services/google_play/product_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/product_lifecycle.rs#L26)
- [src/services/google_play/product_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/product_lifecycle.rs#L100)
- [src/services/google_play/product_lifecycle.rs](C:/share/tyde/bridge/src/services/google_play/product_lifecycle.rs#L123)

Why this matters:

- these functions are still acting like both use cases and adapters
- the Google Play domain rules are visible, but the side-effect boundary is not clean
- this is the main remaining place where the Bridge design still feels layered instead of fully hexagonal

### 4. Several HTTP handlers still bypass the repository port

Most webhook paths now go through the repository port, but a number of normal API handlers still talk to `crate::db::*` directly.

Main holdouts:

- `src/handlers/agent.rs`
- `src/handlers/payments.rs`
- `src/handlers/subscriptions.rs`
- `src/handlers/subscriptions_actions.rs`
- `src/handlers/users.rs`
- `src/handlers/admin.rs`
- `src/middleware/rate_limit.rs`

Relevant paths:

- [src/handlers/agent.rs](C:/share/tyde/bridge/src/handlers/agent.rs#L63)
- [src/handlers/payments.rs](C:/share/tyde/bridge/src/handlers/payments.rs#L58)
- [src/handlers/subscriptions.rs](C:/share/tyde/bridge/src/handlers/subscriptions.rs#L74)
- [src/handlers/subscriptions_actions.rs](C:/share/tyde/bridge/src/handlers/subscriptions_actions.rs#L51)
- [src/handlers/users.rs](C:/share/tyde/bridge/src/handlers/users.rs#L27)
- [src/handlers/admin.rs](C:/share/tyde/bridge/src/handlers/admin.rs#L26)
- [src/middleware/rate_limit.rs](C:/share/tyde/bridge/src/middleware/rate_limit.rs#L233)

Why this matters:

- the split is uneven if only webhook flows are port-driven
- application/use-case logic should own the rules, not the handlers
- read paths are still coupled to the DB layout

### 5. The repository port is still broad

`BridgeRepository` is useful, but it currently acts like a catch-all for many unrelated concerns:

- apps
- provider configs
- checkout cache
- subscriptions
- payments
- webhooks
- agent topups

Relevant path:

- [src/ports.rs](C:/share/tyde/bridge/src/ports.rs#L18)

Why this matters:

- the architecture is better than raw DB access, but the port is still large enough to blur boundaries
- the next cleanup would be to split ports by use case or bounded context

## Recommended Next Extraction Order

1. `src/webhooks/forwarding.rs`
2. `src/webhooks/scheduler.rs`
3. `src/services/google_play/subscription_lifecycle.rs`
4. `src/services/google_play/product_lifecycle.rs`
5. Remaining handler and middleware read paths
6. Split `BridgeRepository` into smaller ports

## Short Checklist

- [ ] Move webhook forwarding reads and delivery updates behind ports
- [ ] Move webhook scheduler queries and mutations behind ports
- [ ] Split Google Play lifecycle side effects out of the lifecycle handlers
- [ ] Replace remaining handler and middleware DB reads with application ports
- [ ] Split `BridgeRepository` into smaller, use-case-specific ports
- [ ] Re-run `cargo check` and `cargo test` after each extraction

## Practical Target

If Bridge is fully split, the dependency direction should read as:

`handlers / workers / webhook adapters -> application services -> ports -> DB/provider implementations`

Not:

`handlers / workers -> crate::db::*`

## Bottom Line

Bridge is already partially hexagonal.

The remaining work is to move:

- webhook forwarding
- scheduler jobs
- Google Play lifecycle side effects
- leftover handler reads

behind smaller ports so the application layer stops depending on concrete persistence and infrastructure details.
