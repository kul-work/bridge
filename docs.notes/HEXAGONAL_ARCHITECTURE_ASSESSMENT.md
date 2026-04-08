# Hexagonal Architecture Assessment

**Date**: April 8, 2026  
**Commit**: `c084dff065a2df7ac2384c2b1e5f90cc47b26dff`  
**Status**: ✅ **Architecture Sound — Trait Granularity & File Organization Need Work**

---

## Executive Summary

The hexagonal architecture refactor achieved its primary goal: clean separation between HTTP handlers, business logic, and database access. The handlers are thin (14–26 lines), the application layer owns all business rules, and `AppState` exposes typed repository views that hide implementation details from callers.

Two issues need addressing:

1. **Trait over-segmentation** — 20+ fine-grained repository traits is excessive for a single-database service with one implementor (`db::Database`). Many traits model query shapes or single-workflow projections rather than stable architectural seams. These should be consolidated into ~7-8 domain-sized traits.

2. **File organization** — `ports.rs` at 1,722 lines mixes trait definitions, data types, `impl` blocks, and helper functions in one monolithic file. This should be split into a `ports/` module directory.

---

## What The Architecture Got Right

### Thin Handlers

Handlers do exactly one thing — extract request, call application, return response:

```rust
// handlers/verify_purchase.rs — 14 lines total
pub async fn verify_purchase(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(payload): Json<VerifyPurchaseRequest>,
) -> Result<(StatusCode, Json<VerifyPurchaseResponse>), BridgeError> {
    let repo = state.verify_purchase_repo();
    let response = application::verify_purchase::verify_purchase(
        repo, auth.app_id, payload,
    ).await?;
    Ok((StatusCode::OK, Json(response)))
}
```

No business logic leaks into handlers. Adding a new endpoint is trivial.

### Composite Traits Contain Complexity at Handler Boundary

`AppState` exposes `&dyn VerifyPurchaseHandlerRepository` which hides the bound list from handlers entirely:

```rust
// state.rs — callers see this
pub(crate) fn verify_purchase_repo(&self) -> &(dyn VerifyPurchaseHandlerRepository + '_) {
    self.database.as_ref()
}
```

### Clean Application Layer

Business logic lives in `src/application/` with well-separated concerns:
- `checkout.rs` + `checkout_helpers.rs` + `checkout_types.rs`
- `verify_purchase.rs` + `verify_purchase_provider.rs` + `verify_purchase_types.rs`
- `subscription_actions.rs` + `subscription_actions_types.rs`

---

## Problem 1: Trait Over-Segmentation

### The Issue

The trait system has **reasonable intent but excessive granularity**. With only one concrete implementor (`db::Database`), no mock/fake implementations in evidence, and several overlapping responsibilities, the 20+ traits create abstraction cost without proportional benefit.

### Symptoms

**Application functions repeat full bound lists instead of using composites:**
```rust
// Current — 8 trait bounds repeated at application layer
pub async fn verify_purchase<
    R: AppLookupRepository
        + GooglePlayAccountLookupRepository
        + PaymentAcknowledgementRepository
        + ProviderConfigLookupRepository
        + SubscriptionLookupRepository
        + VerifyPurchaseRepository
        + WebhookForwardRepository
        + WebhookWriteRepository
        + ?Sized,
>(repo: &R, ...) -> Result<VerifyPurchaseResponse, BridgeError>
```

**Same repo passed multiple times at call sites:**
```rust
// subscription_actions — repo, repo, repo, repo
cancel_subscription(repo, repo, repo, repo, ...)

// checkout — database.as_ref(), database.as_ref()
application::checkout::create_checkout(database.as_ref(), database.as_ref(), ...)
```

**Traits that are too narrow to justify standalone existence:**
- `GooglePlayAccountLookupRepository` — one method, one provider-specific query
- `PurchaseOwnerLookupRepository` — three lookup variants of the same concept
- `PaymentAcknowledgementRepository` — two methods, subset of payments
- `PaymentStatusLookupRepository` — one method, subset of payments

**Overlapping responsibilities:**
- `SubscriptionReadRepository` vs `SubscriptionLookupRepository` — both query subscriptions, return different projections
- `PaymentAcknowledgementRepository` vs `PaymentStatusLookupRepository` — both are narrow payment queries

### What's Actually Good

Not all traits are over-segmented. Some represent real domain boundaries:
- `VerifyPurchaseRepository::commit_verified_purchase(...)` — a real business operation, not an artificial table wrapper
- `WebhookProcessingRepository` — substantial workflow crossing multiple concerns
- `AgentRepository`, `AdminRepository`, `SchedulerRepository` — distinct domain boundaries

### Consolidation Target

Collapse from 20+ traits to ~7-8 domain-sized traits:

| Target Trait | Absorbs |
|---|---|
| **`AppConfigRepository`** | `AppLookupRepository` + `ProviderConfigLookupRepository` |
| **`SubscriptionRepository`** | `SubscriptionReadRepository` + `SubscriptionWriteRepository` + `SubscriptionLookupRepository` + `PurchaseOwnerLookupRepository` + `GooglePlayAccountLookupRepository` |
| **`PaymentRepository`** | `PaymentReadRepository` + `PaymentAcknowledgementRepository` + `PaymentStatusLookupRepository` |
| **`WebhookRepository`** | `WebhookWriteRepository` + `WebhookReadRepository` + `WebhookForwardRepository` + `WebhookSuppressionRepository` + `WebhookProviderLookupRepository` |
| **`AgentRepository`** | `AgentRepository` + `AgentReadRepository` (already mostly merged) |
| **`AdminRepository`** | stays as-is |
| **`UserRepository`** | stays as-is |
| **`SchedulerRepository`** | stays as-is |

Use-case composites (`VerifyPurchaseHandlerRepository`, `WebhookProcessingRepository`) stay where they help, but become simpler since they combine fewer traits.

### Immediate Win (No Consolidation Needed)

Application functions should use the composite trait directly instead of repeating the bound list:

```rust
// After — clean, one trait bound
pub async fn verify_purchase<R: VerifyPurchaseHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    payload: VerifyPurchaseRequest,
) -> Result<VerifyPurchaseResponse, BridgeError>
```

This is a zero-risk change that instantly removes the "generic hell" at application function signatures.

---

## Problem 2: `ports.rs` Monolith

The 1,722-line `ports.rs` file mixes concerns:

| Concern | What's Mixed In |
|---|---|
| **Trait definitions** | 20+ traits |
| **Data types** | `SubscriptionLookupSnapshot`, `WebhookPaymentRecordRequest`, enums |
| **impl blocks** | All `impl XxxRepository for db::Database` (~900 lines of delegation) |
| **Helper functions** | `map_subscription_lookup_snapshot`, `with_transaction_impl` |
| **Conversion impls** | `From<Subscription> for WebhookSubscriptionSnapshot` |

### Split Plan

Reorganize into a `ports/` module directory (structure adapts to post-consolidation trait names):

```
src/ports/
├── mod.rs              — re-exports only
├── traits/
│   ├── mod.rs          — re-exports
│   ├── app.rs          — AppConfigRepository
│   ├── subscription.rs — SubscriptionRepository
│   ├── webhook.rs      — WebhookRepository
│   ├── checkout.rs     — CheckoutRepository
│   ├── payment.rs      — PaymentRepository
│   ├── agent.rs        — AgentRepository
│   ├── admin.rs        — AdminRepository
│   ├── user.rs         — UserRepository
│   └── scheduler.rs    — SchedulerRepository
├── composites.rs       — VerifyPurchaseHandlerRepository,
│                         SubscriptionActionsHandlerRepository,
│                         WebhookProcessingRepository
├── types.rs            — SubscriptionLookupSnapshot, WebhookPaymentRecordRequest,
│                         SubscriptionWebhookTransition, TransactionOutcome, etc.
├── impls/
│   ├── mod.rs          — re-exports
│   ├── subscription.rs — impl SubscriptionRepository for db::Database
│   ├── webhook.rs      — impl WebhookRepository for db::Database
│   ├── checkout.rs     — impl CheckoutRepository for db::Database
│   ├── payment.rs      — impl PaymentRepository for db::Database
│   ├── agent.rs        — impl AgentRepository for db::Database
│   ├── admin.rs        — impl AdminRepository for db::Database
│   ├── user.rs         — impl UserRepository for db::Database
│   └── scheduler.rs    — impl SchedulerRepository for db::Database
└── helpers.rs          — mapping functions, with_transaction_impl, From impls
```

### Rules for the split

- **`mod.rs` re-exports everything** so no downstream file needs path changes
- **Each trait file is self-contained** — one domain, its traits, nothing else
- **Each impl file mirrors its trait file** — easy to find the delegation code
- **Types and helpers get their own files** — no more scrolling past 200 lines of structs to find a trait

---

## K.I.S.S. Alignment

| Project Principle | Current | After Consolidation + Split |
|---|---|---|
| **Avoid over-engineering** | ⚠️ 20+ traits for one implementor | ✅ ~8 domain-sized traits |
| **Readable > Clever** | ⚠️ Generic bound lists, monolith file | ✅ Composites hide bounds, navigable files |
| **Single responsibility** | ✅ Each layer has one job | ✅ Maintained |
| **Minimal abstractions** | ⚠️ Too many micro-traits | ✅ One trait per domain |

---

## Phased Execution

### Phase 1: Use Composites in Application Layer (immediate, <1h)

- Change `application/verify_purchase.rs` to use `R: VerifyPurchaseHandlerRepository + ?Sized`
- Change `application/subscription_actions.rs` to use `R: SubscriptionActionsHandlerRepository + ?Sized`
- Eliminate `repo, repo, repo, repo` call patterns
- **Zero architectural change, zero risk**

### Phase 2: Trait Consolidation (~half day)

- Merge micro-traits into domain-sized traits per consolidation table above
- Update composite trait definitions (they get simpler)
- Update all `impl ... for db::Database` blocks
- Verify with `cargo check`

### Phase 3: File Split (~half day)

- Replace `src/ports.rs` with `src/ports/` directory
- Distribute traits, impls, types, helpers into domain-aligned files
- Re-export from `mod.rs` for zero downstream impact
- Verify with `cargo check`

---

## When Fine-Grained Traits Would Be Justified

The current granularity becomes appropriate if any of these become true:
- Real alternate implementations (not theoretical) are added
- Extensive unit tests with fakes/mocks at the application boundary
- Application logic is extracted into a reusable crate that must not know about Postgres/sqlx
- Different workflows need genuinely different storage backends

None of these apply today.

---

## Files Affected

**Phase 1:** `src/application/verify_purchase.rs`, `src/application/subscription_actions.rs`, `src/application/checkout.rs`  
**Phase 2:** `src/ports.rs`, `src/application/`, `src/state.rs`  
**Phase 3:** `src/ports.rs` → `src/ports/` directory. No changes to handlers.

---

**Estimated Effort**: 1.5–2 days total across all phases  
**Risk**: Low — each phase is independently verifiable with `cargo check`  
**Benefit**: Domain-aligned traits, navigable file structure, cleaner application signatures
