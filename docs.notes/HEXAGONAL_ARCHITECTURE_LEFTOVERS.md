# Hexagonal Architecture Leftovers

**Date**: April 8, 2026  
**Baseline Checked From**: `ca99cea7a7361b97fc2a3d8546fbddb7157522db`  
**Current HEAD**: `ba3a5ba`  
**Related Assessment**: `docs.notes/HEXAGONAL_ARCHITECTURE_ASSESSMENT.md`

---

## Summary

The hexagonal refactor is in a good place structurally, but the implementation does not fully match the assessment's final phase status.

- The `src/ports/` module split is implemented.
- Handler-facing composite repositories are implemented.
- Trait consolidation is only partial, not complete.

This is worth tracking as leftover work so the docs do not imply the architecture cleanup is fully finished.

---

## What Is Done

- `src/ports.rs` was split into `src/ports/` with separate `traits/`, `impls/`, `types.rs`, `helpers.rs`, and `composites.rs`.
- Application entrypoints use handler composites:
  - `src/application/verify_purchase.rs`
  - `src/application/subscription_actions.rs`
  - `src/application/checkout.rs`
- `AppState` exposes typed handler repos in `src/state.rs`.

---

## Leftovers Worth Mentioning

### 1. Phase 2 Is Only Partially Done

The assessment says Phase 2 "Trait Consolidation" is completed, but the code still keeps many micro-traits alongside the new domain aggregates.

Examples still present under `src/ports/traits/`:

- `GooglePlayAccountLookupRepository`
- `PurchaseOwnerLookupRepository`
- `PaymentAcknowledgementRepository`
- `PaymentStatusLookupRepository`
- `SubscriptionLookupRepository`

The code now has aggregated traits like `AppConfigRepository`, `SubscriptionRepository`, `PaymentRepository`, and `WebhookRepository`, but the fine-grained traits were not actually collapsed away.

### 2. Webhook Processing Still Depends On Fine-Grained Traits

`src/ports/composites.rs` still defines webhook-processing-specific layers that directly depend on narrow traits:

- `WebhookProcessingLookupRepository`
- `WebhookProcessingMutationRepository`
- `WebhookProcessingTransactionRepository`

That is still a valid design, but it does not match the assessment's stated consolidation target of roughly 7-8 domain-sized traits.

### 3. Some Generic Bound Cleanup Is Still Incomplete

There are still places outside the main application entrypoints that use explicit repository bound lists instead of a higher-level composite.

Current examples:

- `src/application/verify_purchase_provider.rs`
- `src/webhooks/forwarding.rs`
- `src/webhooks/scheduler.rs`

This is smaller than the original problem, but it means the "generic hell removed" story is only mostly true, not fully true.

### 4. Assessment Status Overstates Completion

The assessment currently marks all three phases as completed. A more accurate status would be:

- Phase 1: completed
- Phase 2: partially completed
- Phase 3: completed

---

## Verification Notes

- `cargo check` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` still fails on existing `too_many_arguments` warnings in webhook-related code.

These are not proof that the refactor is bad, but they are worth recording because the current docs read as more complete than the code actually is.

---

## Suggested Interpretation

This should be tracked as non-blocking architectural cleanup, not as a broken implementation.

The important separation of handlers, application logic, and database-backed ports is already in place. The leftover work is mainly about making the trait story simpler and bringing the written status back in line with the actual implementation.
