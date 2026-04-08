# Hexagonal Refactor Code Review

**Date**: April 8, 2026  
**Scope**: SHA `c084dff065a2df7ac2384c2b1e5f90cc47b26dff` -> HEAD  
**Status**: Lean hexagonal boundary established for `verify_purchase` and subscription actions; broader port cleanup still pending

---

## Executive Summary

The refactor initially introduced a ports-and-adapters layer without fully delivering on hexagonal architecture goals. Since then, the main `verify_purchase` flow and the subscription action workflows have been moved into application services, and the direct Axum/database coupling in those paths has been removed.

What is now fixed:

1. `verify_purchase` no longer depends on Axum from the application layer.
2. `verify_purchase` no longer passes `sqlx::Transaction` through the port boundary.
3. `AppState` no longer exposes `database_ref()`.
4. `subscriptions_actions` orchestration moved into `src/application/subscription_actions.rs`.

What still needs work:

1. Other repository traits still expose DB-shaped structs.
2. Some ports are still broader than necessary.
3. Some handlers still do their own orchestration.
4. `checkout` still has application-layer transport coupling.

### Verdict
The repo is materially better than the original review state. The hot paths now look like lean hexagonal architecture, but the overall port layer is still more infrastructure-shaped than it needs to be.

---

## What's Fixed So Far

### 1. `verify_purchase` is now application-owned

- [`src/application/verify_purchase.rs`](file:///c:/share/tyde/bridge/src/application/verify_purchase.rs) no longer imports Axum or returns HTTP-shaped results.
- Request/response DTOs now live in [`src/application/verify_purchase_types.rs`](file:///c:/share/tyde/bridge/src/application/verify_purchase_types.rs).
- Provider verification and callback forwarding moved to [`src/application/verify_purchase_provider.rs`](file:///c:/share/tyde/bridge/src/application/verify_purchase_provider.rs).
- The handler is now a thin adapter in [`src/handlers/verify_purchase.rs`](file:///c:/share/tyde/bridge/src/handlers/verify_purchase.rs).

### 2. `VerifyPurchaseRepository` no longer leaks transaction internals

- The `verify_purchase` use case no longer passes `sqlx::Transaction` through the port boundary.
- The repository now commits the transaction internally.
- The verify-purchase path now uses application snapshots instead of raw `db::Subscription` values.

### 3. `AppState` no longer exposes the old database escape hatch

- [`src/state.rs`](file:///c:/share/tyde/bridge/src/state.rs) no longer has `database_ref()`.
- The direct handler bypass used by `verify_purchase` was removed.

### 4. Subscription action orchestration moved out of handlers

- [`src/handlers/subscriptions_actions.rs`](file:///c:/share/tyde/bridge/src/handlers/subscriptions_actions.rs) now delegates to [`src/application/subscription_actions.rs`](file:///c:/share/tyde/bridge/src/application/subscription_actions.rs).
- Cancel/resume/billing-portal/price-step-up workflows are application services now.
- Request/response DTOs live in [`src/application/subscription_actions_types.rs`](file:///c:/share/tyde/bridge/src/application/subscription_actions_types.rs).

---

## Critical Issues

### Issue 1: Application Layer Coupled to HTTP Framework

**Severity**: HIGH  
**Status**: FIXED for `verify_purchase`; still present in `src/application/checkout.rs`

The verify-purchase application flow no longer imports Axum types and no longer returns HTTP-shaped results.

**Why this mattered**
- The core was not framework-agnostic.
- Orchestration logic could not be reused from CLI, jobs, or consumers.
- Testing depended on HTTP types.

**Current note**
- This issue is no longer true for `verify_purchase`.
- It still applies to `checkout`.

---

### Issue 2: Ports Leak Persistence Implementation Details

**Severity**: HIGH  
**Status**: FIXED for the `verify_purchase` path; still true in other repository traits

The `verify_purchase` path no longer leaks `sqlx::Transaction`.
Its repository now returns application snapshots and owns the transaction internally.

**Why this still matters elsewhere**
- Several other ports still return `db::*` structs.
- The abstraction is still partly shaped by infrastructure instead of business needs.

**Correct approach**
- Keep returning application DTOs where the seam is meant to be stable.
- Avoid DB-shaped return types in application-facing ports when possible.

---

### Issue 3: Inconsistent Abstraction Boundary

**Severity**: HIGH  
**Status**: FIXED for `verify_purchase`

`AppState` no longer exposes `database_ref()`, and the `verify_purchase` handler no longer bypasses the repository boundary.

**Residual risk**
- Other parts of the codebase still have direct DB access or mixed access patterns.

---

## Design Problems

### Issue 4: Wide, Infrastructure-Shaped Ports

**Severity**: MEDIUM-HIGH  
**Status**: Improved for `verify_purchase`; still relevant for broader ports

`VerifyPurchaseRepository` is now narrower than it was, but the repo still has broad, infrastructure-shaped traits elsewhere.

**Why this matters**
- Ports should grow with stable business concepts, not call sites.
- Large seams are harder to mock and reason about.

---

### Issue 5: Boilerplate Forwarding Without Payoff

**Severity**: MEDIUM  
**Status**: Still present in several port implementations

The repo still has thin forwarding in a number of places.

**Why this matters**
- More code to maintain.
- More places to update when DB signatures change.
- Little domain logic gained in return.

---

### Issue 6: Duplication Across Ports

**Severity**: MEDIUM  
**Status**: Still present

Repeated shapes still exist across repository traits and implementations.

**Why this matters**
- It shows the abstraction is still shaped around call sites.
- It increases maintenance cost.

---

### Issue 7: Business Orchestration Still in Handlers

**Severity**: MEDIUM  
**Status**: FIXED for `subscriptions_actions`

The subscription action workflows have been moved into `src/application/subscription_actions.rs`.

**Why this mattered**
- Transport concerns and business workflow were mixed.
- The abstraction layer was mostly a read/write boundary, not a decoupling boundary.

**Current note**
- This issue is fixed for the subscription action handlers.
- It remains a useful review check for other handlers.

---

## What Is Working Well

### 1. Read/Write Repository Split

- `SubscriptionReadRepository` vs `SubscriptionWriteRepository`
- `PaymentReadRepository`

This remains a good pattern.

### 2. Composed Capability Traits

- `AppWebhookRepository`

Small trait composition is still useful when the capability is focused.

### 3. Leaner Verify-Purchase Boundary

- `verify_purchase` is now a real application service.
- The handler is a transport adapter.
- The repository owns the transaction boundary.

---

## Recommendations

### Immediate Direction

Stay with lean hexagonal, not full hexagonal.

- Keep ports for provider integrations, webhook dispatch, and the small set of application services that genuinely need them.
- Keep moving orchestration out of handlers.
- Continue replacing DB-shaped return types where the seam matters.
- Do not reintroduce transport types into application services.

### Remaining Cleanup Targets

- `src/application/checkout.rs` should be reviewed next for the same coupling pattern that was already fixed in `verify_purchase`.
- `src/ports.rs` still has ports that are broader than necessary.
- Remaining handlers should be checked for orchestration that belongs in application services.

---

## Impact Summary

| Aspect | Current State | Target State |
|--------|---------------|--------------|
| Application -> HTTP | Mixed | Decoupled in application services |
| Ports -> DB types | Partially leaky | Hidden where feasible |
| Boundary consistency | Improved | Enforced |
| Port granularity | Better for hot paths | Focused/use-case-shaped |
| Forwarding boilerplate | Reduced in hot paths | Minimal |
| Orchestration location | Handlers + application services | Application services |
| Testing benefit | Better for fixed paths | Real, focused mocks |
| Code clarity | Better than before | Clear, single path |

---

## Files Requiring Changes

### Already Addressed

- `src/application/verify_purchase.rs`
- `src/application/verify_purchase_provider.rs`
- `src/application/verify_purchase_types.rs`
- `src/application/subscription_actions.rs`
- `src/application/subscription_actions_types.rs`
- `src/handlers/verify_purchase.rs`
- `src/handlers/subscriptions_actions.rs`
- `src/state.rs`
- `src/ports.rs` for the `verify_purchase` boundary

### Next Candidates

- `src/application/checkout.rs`
- Remaining broad ports in `src/ports.rs`
- Any handlers that still mix validation, provider calls, repository reads, writes, and callback dispatch

---

## Related Documentation

- `DESIGN.md`
- Evans & Fowler, Port and Adapters
- `HEXAGONAL_SPLIT_REMAINING.md`

---

**End of Review**
