# Hexagonal Refactor Code Review

**Date**: April 8, 2026  
**Scope**: SHA `c084dff065a2df7ac2384c2b1e5f90cc47b26dff` → HEAD  
**Status**: Partially executed hexagonal design with significant architectural issues  

---

## Executive Summary

The refactor introduces a ports-and-adapters layer but **does not fully deliver on hexagonal architecture goals**. The abstraction adds indirection without achieving real decoupling because:

1. **Application layer is still coupled to HTTP/Axum**
2. **Ports expose SQLx/Postgres implementation details**
3. **Abstraction boundary is inconsistent** (some code uses ports, some bypasses them)
4. **Most adapters are thin forwarding** (boilerplate without payoff)
5. **Business orchestration still lives in handlers**

### Verdict
The refactor **improves organization** in some places but **mostly adds complexity without justifying it**. Not ready for production reliance.

---

## Critical Issues

### Issue 1: Application Layer Coupled to HTTP Framework

**Severity**: HIGH  
**File**: [`src/application/verify_purchase.rs`](file:///c:/share/tyde/bridge/src/application/verify_purchase.rs#L1-L13)

The application layer imports Axum types:
```rust
use axum::http::StatusCode;
use axum::Json;
```

And returns HTTP-shaped results:
```rust
Result<(StatusCode, Json<VerifyPurchaseResponse>), BridgeError>
```

**Why this matters**:
- The "core" is not framework-agnostic
- Cannot reuse orchestration logic from CLI, background jobs, or event consumers
- Testing requires spinning up HTTP types
- Violates fundamental hexagonal principle: domain/application layer must not know about transport

**Related**: [`src/handlers/verify_purchase.rs:122-128`](file:///c:/share/tyde/bridge/src/handlers/verify_purchase.rs#L122-L128) just delegates straight to application.

---

### Issue 2: Ports Leak Persistence Implementation Details

**Severity**: HIGH  
**File**: [`src/ports.rs`](file:///c:/share/tyde/bridge/src/ports.rs#L5-L16)

Ports import DB-layer structs directly:
```rust
use crate::db::apps::App;
use crate::db::subscriptions::Subscription;
use crate::db::payments::Payment;
use crate::db::provider_configs::ProviderConfig;
```

Examples of leaky port signatures:

| Port | Returns | Problem |
|------|---------|---------|
| `SubscriptionReadRepository` | `db::subscriptions::Subscription` | Exposes DB struct |
| `PaymentReadRepository` | `db::payments::*` | Exposes DB struct |
| `AppProviderRepository` | DB types | Exposes DB struct |
| **All transaction-based ports** | `sqlx::Transaction<'_, sqlx::Postgres>` | Exposes SQLx directly |

Examples:
- [`src/ports.rs:43-68`](file:///c:/share/tyde/bridge/src/ports.rs#L43-L68) — `SubscriptionReadRepository`
- [`src/ports.rs:188-198`](file:///c:/share/tyde/bridge/src/ports.rs#L188-L198) — transaction exposure
- [`src/ports.rs:390-400`](file:///c:/share/tyde/bridge/src/ports.rs#L390-L400) — more transaction leakage

**Why this matters**:
- Mocking requires setting up SQLx test infrastructure
- Adapters are less swappable (tied to Postgres/SQLx)
- The abstraction is shaped by infrastructure, not by business needs
- Can't easily swap to another database without rewriting all mock implementations

**Correct approach**: Ports should return domain/application DTOs, not DB structs.

---

### Issue 3: Inconsistent Abstraction Boundary

**Severity**: HIGH  
**File**: [`src/state.rs:74-80`](file:///c:/share/tyde/bridge/src/state.rs#L74-L80)

`AppState` has both:
- Repository trait objects (the "new" way)
- `database_ref()` that returns concrete `Database` (the "old" way)

**File**: [`src/handlers/verify_purchase.rs:127`](file:///c:/share/tyde/bridge/src/handlers/verify_purchase.rs#L127)
Some handlers use `state.database_ref()` directly, bypassing ports.

**Why this matters**:
- The abstraction is not mandatory — you can opt out anywhere
- Creates cognitive load: readers must remember where to look for each operation
- False sense of structure (repos are optional, not architectural boundary)
- Makes refactoring harder (unclear which path is the "right" one)

**Correct approach**: Pick one boundary rule and enforce it consistently.

---

## Design Problems

### Issue 4: Wide, Infrastructure-Shaped Ports

**Severity**: MEDIUM-HIGH  
**File**: [`src/ports.rs:185-273`](file:///c:/share/tyde/bridge/src/ports.rs#L185-L273)

`VerifyPurchaseRepository` combines:
- app/provider lookup
- subscription lookup
- payment recording
- transaction management
- webhook creation/forwarding (via supertraits)

**Usage**: [`src/application/verify_purchase.rs:57-58, 148-171, 228-367, 414-427`](file:///c:/share/tyde/bridge/src/application/verify_purchase.rs#L57-L58)

**Why this matters**:
- Port grows as call sites grow, not as business concepts grow
- To mock for tests, you need to implement many unrelated capabilities
- Testing benefit is reduced because the "seam" is too large
- Indicates the abstraction is shaped around implementation, not domain

**Correct approach**: Break into smaller, use-case-focused ports.

---

### Issue 5: Boilerplate Forwarding Without Payoff

**Severity**: MEDIUM  
**File**: Multiple locations

Examples of thin, mechanical forwarding:

| Port | Location | Pattern |
|------|----------|---------|
| `AppProviderRepository` | [`src/ports.rs:264-304`](file:///c:/share/tyde/bridge/src/ports.rs#L264-L304) | `db::module::function(self.pool(), ...)` |
| `SubscriptionReadRepository` | [`src/ports.rs:633-687`](file:///c:/share/tyde/bridge/src/ports.rs#L633-L687) | Same forwarding pattern |
| `SubscriptionWriteRepository` | [`src/ports.rs:821-891`](file:///c:/share/tyde/bridge/src/ports.rs#L821-L891) | Same forwarding pattern |
| `WebhookForwardRepository` | [`src/ports.rs:1173-1205`](file:///c:/share/tyde/bridge/src/ports.rs#L1173-L205) | Same forwarding pattern |

**Why this matters**:
- "Abstraction tax": more code to maintain, more places to update when DB signatures change
- Duplication of signatures
- Little domain logic gained, little testing benefit
- Lines of code increased without proportional clarity/decoupling

**Correct approach**: Only create ports where there's real payoff (see: Issue 8).

---

### Issue 6: Duplication Across Ports

**Severity**: MEDIUM  
**File**: [`src/ports.rs`](file:///c:/share/tyde/bridge/src/ports.rs)

Repeated patterns:
- `get_app` / `get_provider_config`: [`L347-355`](file:///c:/share/tyde/bridge/src/ports.rs#L347-L355) and repeated in other traits
- `lookup_user_by_google_obfuscated_id`: [`L200-204`](file:///c:/share/tyde/bridge/src/ports.rs#L200-L204) and [`L430-434`](file:///c:/share/tyde/bridge/src/ports.rs#L430-L434)
- Transaction runner shape: [`L188-198`](file:///c:/share/tyde/bridge/src/ports.rs#L188-L198) and [`L390-400`](file:///c:/share/tyde/bridge/src/ports.rs#L390-L400)

**Why this matters**:
- Port layer is growing faster than domain model
- Indicates abstraction is being shaped around call sites, not stable concepts
- Update burden increases with each duplication

---

### Issue 7: Business Orchestration Still in Handlers

**Severity**: MEDIUM  
**File**: [`src/handlers/subscriptions_actions.rs:51-141`](file:///c:/share/tyde/bridge/src/handlers/subscriptions_actions.rs#L51-L141)

Example: `cancel_subscription` does:
1. Validation
2. Repository read
3. Provider API call
4. Repository write
5. Webhook callback dispatch

Similar patterns:
- `resume_subscription` ([`L143-201`](file:///c:/share/tyde/bridge/src/handlers/subscriptions_actions.rs#L143-L201))
- `create_billing_portal` ([`L249-296`](file:///c:/share/tyde/bridge/src/handlers/subscriptions_actions.rs#L249-L296))
- Price step-up handlers ([`L315-402`](file:///c:/share/tyde/bridge/src/handlers/subscriptions_actions.rs#L315-L402))

**Why this matters**:
- The refactor added ports but did not move orchestration inward
- Responsibilities are only partially separated
- Transport concerns and business workflow are still mixed
- The new abstraction layer is mostly a read/write boundary, not a decoupling boundary

**Correct approach**: Move orchestration into application-layer services that use ports internally.

---

## What's Working Well

### 1. Read/Write Repository Split

**File**: [`src/ports.rs`](file:///c:/share/tyde/bridge/src/ports.rs)

- `SubscriptionReadRepository` vs `SubscriptionWriteRepository`
- `PaymentReadRepository`
- Clear, easy to understand, helps discoverability

**Keep this pattern.**

---

### 2. Composed Capability Traits

**File**: [`src/ports.rs:357-365`](file:///c:/share/tyde/bridge/src/ports.rs#L357-L365) — `AppWebhookRepository`

Small, pragmatic trait composition for callback dispatch.

**Keep this approach for small, focused capabilities.**

---

### 3. Shared Transaction Helper

**File**: [`src/ports.rs:586-621`](file:///c:/share/tyde/bridge/src/ports.rs#L586-L621) — `with_transaction_impl`

The helper itself is fine; the problem is exposing `sqlx::Transaction` through the port (see Issue 2).

**Fix the exposure, keep the helper.**

---

## Recommendations

### Immediate Actions (M effort, 1–2 days)

Scale back to a lighter boundary. Choose one path:

#### Option A: Lean Hexagonal (Recommended)
- **Keep ports for**: provider integrations, webhook dispatch, maybe 1–2 complex application services
- **Use concrete `Database`** directly for straightforward CRUD handlers
- **Move orchestration** from handlers into application services that use ports internally
- **Stop importing Axum in application layer**

#### Option B: Full Hexagonal (Higher cost, deferred)
- Only pursue if you have real drivers (see: "When to Consider Advanced Path" below)

### Guardrails (If Keeping Hexagonal Direction)

Enforce these rules immediately:

1. **Application layer must NOT import Axum, handlers, or transport types**
   - No `axum::http::StatusCode`, `axum::Json`, `StatusCode`, etc.
   - Return domain/application results, let handlers translate

2. **Ports must NOT expose `sqlx::Transaction` or any SQLx types**
   - Wrap transaction handling inside adapter implementations
   - Return domain/application DTOs from ports

3. **Ports must return domain/application DTOs, NOT `db::*` structs**
   - Define minimal DTOs in `application` or `domain` layer
   - Adapters translate DB structs ↔ domain DTOs

4. **Use one consistent boundary rule**
   - Either use ports consistently everywhere
   - Or use `Database` directly unless there's a real seam
   - Don't allow both paths at the same time

5. **Ports should represent stable business concepts, not call sites**
   - If a new feature adds a method to 3 different ports, that's a smell
   - Instead, create a new use-case-focused port

---

## When to Consider the Advanced Path

A fuller hexagonal design is justified if you have one or more of these pressures:

- [ ] Multiple persistence implementations (e.g., fallback to Redis, event log)
- [ ] Serious use of in-memory/fake adapters in unit tests
- [ ] Business workflows reused by HTTP, jobs, CLI, and event consumers
- [ ] Domain logic growing significantly beyond CRUD/orchestration
- [ ] Provider integrations becoming first-class pluggable adapters

If **none** of these are current pressures, **do not pursue full hexagonal**. Keep it simple.

---

## Optional Advanced Path (If Pursuing Full Hexagonal Later)

When/if you do want "real" hexagonal architecture:

1. **Move request/response DTOs out of handler modules**
   - Create `application/dto/` or `domain/dto/`
   - Stop handler modules from being dependencies

2. **Make application functions return domain results, not HTTP status codes**
   ```rust
   // Before: leaks HTTP
   Result<(StatusCode, Json<Response>), BridgeError>
   
   // After: pure domain
   Result<SubscriptionUpdated, DomainError>
   ```

3. **Replace DB-shaped repository traits with narrower use-case ports**
   - Instead of `SubscriptionReadRepository` with 10 methods
   - Create: `GetSubscriptionByExternalId`, `ListSubscriptionsForApp`, etc.

4. **Hide transaction handling inside adapter/service boundaries**
   - Port methods should not require passing `Transaction`
   - Adapter manages transaction scope internally

5. **Keep provider API clients and webhook dispatch as explicit outbound ports**
   - These are natural seams
   - Worth abstracting for testing and switching

---

## Impact Summary

| Aspect | Current State | Target State |
|--------|---------------|--------------|
| **Application ↔ HTTP** | Coupled | Decoupled |
| **Ports ↔ DB types** | Leaky | Hidden |
| **Abstraction consistency** | Inconsistent | Enforced |
| **Port granularity** | Wide/infrastructure-shaped | Focused/use-case-shaped |
| **Forwarding boilerplate** | High | Minimal (only where it provides value) |
| **Orchestration location** | Handlers + Ports | Application services |
| **Testing benefit** | Limited (complex mocks) | Real (simple, focused mocks) |
| **Code clarity** | Unclear which path to follow | Clear, single path |

---

## Risk Summary

### Risks if Kept As-Is
- More cognitive load for new contributors
- More boilerplate churn when DB signatures change
- False sense of decoupling while still tied to SQLx/Axum
- Hard to build mocks for complex use cases
- Scaling ports becomes painful

### Mitigation
1. **Immediately enforce the guardrails** (see above)
2. **Pick a boundary rule** and document it
3. **Schedule a follow-up refactor** to simplify (Option A recommended)
4. **Add review checklist** to prevent further leaky abstraction

---

## Files Requiring Changes (Priority Order)

### Phase 1: Fix Critical Coupling
- `src/application/verify_purchase.rs` — remove Axum imports, return domain result
- `src/ports.rs` — hide `sqlx::Transaction`, return domain DTOs
- `src/state.rs` — pick one boundary rule, remove mixed access patterns

### Phase 2: Simplify Ports (If Going with Option A)
- Remove thin forwarding adapters
- Keep only: provider integrations, webhook dispatch, maybe 1–2 orchestration ports

### Phase 3: Move Orchestration
- `src/handlers/subscriptions_actions.rs` — move logic into application services
- Create `src/application/subscriptions_service.rs` (or similar)

---

## Related Documentation

- **Project Architecture**: See `DESIGN.md`
- **Hexagonal Design Pattern**: Evans & Fowler (Port & Adapters)
- **Previous Notes**: Check `HEXAGONAL_SPLIT_REMAINING.md` for context

---

## Questions for Team

1. Are we targeting multiple database implementations? (Should drive hexagonal depth)
2. Do we need to reuse business logic across HTTP/CLI/jobs? (Should drive application layer structure)
3. How much testing effort do we currently spend mocking database? (Should inform mock benefits)
4. Is the readability cost of ports justified by current value? (Team consensus needed)

---

**End of Review**
