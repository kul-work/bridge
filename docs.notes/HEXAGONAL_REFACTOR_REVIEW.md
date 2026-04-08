# Hexagonal Refactor Code Review

**Date**: April 8, 2026  
**Scope**: SHA `c084dff065a2df7ac2384c2b1e5f90cc47b26dff` -> HEAD  
**Status**: Lean hexagonal boundary established for `verify_purchase`, `checkout`, and subscription actions; broader port cleanup still pending

---

## Executive Summary

The refactor initially introduced a ports-and-adapters layer without fully delivering on hexagonal architecture goals. Since then, the main `verify_purchase` flow and the subscription action workflows have been moved into application services, and the direct Axum/database coupling in those paths has been removed.

What is now fixed:

1. `verify_purchase` no longer depends on Axum from the application layer.
2. `verify_purchase` no longer passes `sqlx::Transaction` through the port boundary.
3. `checkout` no longer depends on Axum from the application layer.
4. `checkout` no longer exposes handler-owned DTO/helper logic to the application layer.
5. `AppState` no longer exposes `database_ref()`.
6. `subscriptions_actions` orchestration moved into `src/application/subscription_actions.rs`.

What still needs work:

1. Some repository traits and seams are still broader than necessary.
2. Some handlers still do their own orchestration.
3. Boilerplate forwarding and duplicated shapes still exist in a few port implementations.

### Verdict
The repo is materially better than the original review state. The hot paths now look like lean hexagonal architecture, but the overall port layer is still more infrastructure-shaped than it needs to be.

---

## Design Problems

### Issue 4: Wide, Infrastructure-Shaped Ports

**Severity**: MEDIUM-HIGH  
**Status**: Still relevant for broader ports and handler seams

`VerifyPurchaseRepository` is now narrower than it was, but the repo still has broad, infrastructure-shaped traits elsewhere.

**Why this matters**
- Ports should grow with stable business concepts, not call sites.
- Large seams are harder to mock and reason about.

---

### Issue 5: Boilerplate Forwarding Without Payoff

**Severity**: MEDIUM  
**Status**: Still present in a few port implementations

The repo still has thin forwarding in a number of places.

**Why this matters**
- More code to maintain.
- More places to update when DB signatures change.
- Little domain logic gained in return.

---

### Issue 6: Duplication Across Ports

**Severity**: MEDIUM  
**Status**: Still present in a few repository traits

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

## Recommendations

### Immediate Direction

Stay with lean hexagonal, not full hexagonal.

- Keep ports for provider integrations, webhook dispatch, and the small set of application services that genuinely need them.
- Keep moving orchestration out of handlers where any mixed transport/business flow still remains.
- Do not reintroduce transport types into application services.
- Avoid adding new ports unless they carry a stable business seam.

### Remaining Cleanup Targets

- `src/ports.rs` still has broad traits and DB-shaped seams outside the fixed hot paths.
- Reduce boilerplate forwarding only when it eliminates meaningful duplication or unlocks a stable boundary.
- Audit remaining handlers for orchestration that belongs in application services before introducing new abstractions.

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

## Related Documentation

- `DESIGN.md`
- Evans & Fowler, Port and Adapters
- `HEXAGONAL_SPLIT_REMAINING.md`

---

**End of Review**
