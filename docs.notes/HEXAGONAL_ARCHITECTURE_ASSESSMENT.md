# Hexagonal Architecture Assessment

**Date**: April 8, 2026  
**Commit**: `c084dff065a2df7ac2384c2b1e5f90cc47b26dff`  
**Status**: ✅ **Architecture Sound — Trait Granularity & File Organization Need Work**

---

## Executive Summary

The hexagonal architecture refactor achieved its primary goal: clean separation between HTTP handlers, business logic, and database access. The handlers are thin (14–26 lines), the application layer owns all business rules, and `AppState` exposes typed repository views that hide implementation details from callers.

Two issues need addressing:

1. **Trait over-segmentation** — 30 repository traits is excessive for a single-database service with one implementor (`db::Database`). Many traits model query shapes or single-workflow projections rather than stable architectural seams. These should be consolidated into ~7-8 domain-sized traits.

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

The trait system has **reasonable intent but excessive granularity**. With only one concrete implementor (`db::Database`), no mock/fake implementations in evidence, and several overlapping responsibilities, the 30 traits (including 8 composites) create abstraction cost without proportional benefit.

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

### Agent Implementation Guide — Phase 1 Details

#### File: `src/application/verify_purchase.rs`

**Current** (lines 5-9, 19-31): Function imports 8 individual traits and repeats them as bounds.

**Change**: Replace imports and signature:
```rust
// Replace individual trait imports with:
use crate::ports::VerifyPurchaseHandlerRepository;

// Replace the generic signature with:
pub async fn verify_purchase<R: VerifyPurchaseHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    payload: VerifyPurchaseRequest,
) -> Result<VerifyPurchaseResponse, BridgeError> {
```

**Internal callsites**: This function already receives a single `repo: &R` and calls methods on it directly. No internal changes needed — the composite supertrait already provides all required methods.

Also check `verify_purchase_provider.rs` — its functions take separate typed repos. Those should also use `VerifyPurchaseHandlerRepository` if they use a subset of the same traits.

#### File: `src/application/subscription_actions.rs`

This file has **6 public functions** with mixed patterns. Some take separate `app_repo`/`callback_repo`/`subscription_repo`/`subscription_write_repo` params despite all being the same `&dyn SubscriptionActionsHandlerRepository` at the callsite (see `handlers/subscriptions_actions.rs` lines 24-28: `repo, repo, repo, repo`).

**Functions to change:**

| Function | Current Params | After |
|---|---|---|
| `cancel_subscription` | `app_repo: &R, callback_repo: &C, subscription_repo: &S, subscription_write_repo: &W` (4 params, 4 generics) | `repo: &R` (1 param, `R: SubscriptionActionsHandlerRepository`) |
| `resume_subscription` | Same 4-param pattern | Same single-repo fix |
| `acknowledge_subscription` | `lookup_repo: &L, subscription_write_repo: &W` (2 params, 2 generics) | `repo: &R` (1 param, `R: SubscriptionActionsHandlerRepository`) |
| `create_billing_portal` | `app_repo: &R, subscription_repo: &S` (2 params, 2 generics) | `repo: &R` (1 param, `R: SubscriptionActionsHandlerRepository`) |
| `accept_price_step_up` | `lookup_repo: &L, callback_repo: &C, subscription_write_repo: &W` (3 params, 3 generics) | `repo: &R` (1 param, `R: SubscriptionActionsHandlerRepository`) |
| `decline_price_step_up` | `lookup_repo: &L, subscription_write_repo: &W` (2 params, 2 generics) | `repo: &R` (1 param, `R: SubscriptionActionsHandlerRepository`) |

**Also change**: `dispatch_subscription_callback` (private helper at line 418) — currently takes `R: AppLookupRepository + WebhookForwardRepository + WebhookWriteRepository`. Change to `R: SubscriptionActionsHandlerRepository` since it's only ever called with the same repo.

**Handler side** (`handlers/subscriptions_actions.rs`): All 6 handlers currently pass `repo, repo, repo, repo`. After the application function changes, each becomes:
```rust
// Before:
application::subscription_actions::cancel_subscription(repo, repo, repo, repo, input)
// After:
application::subscription_actions::cancel_subscription(repo, input)
```

#### File: `src/application/checkout.rs`

**Current** (lines 14-22): Takes two separate repos — `checkout_repo: &C` and `app_repo: &A`.

**Decision**: There is **no existing composite trait** for checkout. Two options:

1. **Create a `CheckoutHandlerRepository`** composite (preferred, matches the pattern for other handlers):
   ```rust
   // Add to ports.rs near the other composites (~line 425)
   pub(crate) trait CheckoutHandlerRepository:
       AppLookupRepository + CheckoutRepository + ProviderConfigLookupRepository + Send + Sync {}
   impl<T> CheckoutHandlerRepository for T
   where T: AppLookupRepository + CheckoutRepository + ProviderConfigLookupRepository + Send + Sync {}
   ```
   Then update the function and handler to use a single `repo: &R` where `R: CheckoutHandlerRepository`.

2. **Leave as-is** — checkout only has 2 params, not 4. Lower priority.

**Handler side** (`handlers/checkout.rs` lines 17-21): Currently calls `database.as_ref(), database.as_ref()`. The handler doesn't use `state.subscription_actions_repo()` pattern — it calls `state.database()` directly. After adding the composite, add a `checkout_repo()` method to `AppState` (in `state.rs`) following the existing pattern.

#### Verification

After all Phase 1 changes, run:
```bash
cargo check
cargo clippy
```

No test changes expected — these are signature-only refactors. The actual method dispatch is unchanged because the composite supertraits already expose all the same methods.

---

## Problem 2: `ports.rs` Monolith

The 1,722-line `ports.rs` file mixes concerns:

| Concern | What's Mixed In |
|---|---|
| **Trait definitions** | 30 traits |
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
| **Avoid over-engineering** | ⚠️ 30 traits for one implementor | ✅ ~8 domain-sized traits |
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

---

## Implementation Plan

**Plan File**: `C:\Users\Mihaita Nita\.windsurf\plans\hexagonal-architecture-refactor-9540d6.md`

**Phase Status**:
- Phase 1: Use Composites in Application Layer - **Completed** ✅
- Phase 2: Trait Consolidation - **Pending**
- Phase 3: File Split - **Pending**
