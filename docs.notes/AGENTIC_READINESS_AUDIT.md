# Bridge Agentic Readiness Audit - Gaps

High-priority gaps affecting agent reliability and parallelization.

| Dimension | Status | Priority |
| --- | --- | --- |
| Type-State Pattern | Lifecycle states use raw strings | HIGH |
| Currency Correctness | f64 used in several parsing paths | HIGH |
| Context Hotspot | processor.rs mixes 5+ concerns | MEDIUM |
| Invariant Documentation | No compact "do/don't" rules for agents | MEDIUM |

## Details

### 1. Type-State Pattern (HIGH)

**Problem:**
- Lifecycle states use `status: String` instead of typed enums
- Unknown states default to `Expired` silently
- Invalid transitions remain representable until runtime

**Examples:**
- `VerifyPurchaseResponse.status` → String
- `Verified Purchase.status` → String
- DB models with `status: String`

**Fix:** Create `SubscriptionStatus` enum with all canonical states. Use in domain model, DB, and webhook parsing.

### 2. Currency Correctness (HIGH)

**Problem:**
- f64 used for amount parsing in checkout, provider responses, Coinbase paths
- Floating-point rounding causes cents drift
- Payment-critical code should never use floats

**Fix:** Replace with `Decimal` or consistent `i64` cents representation.

### 3. Context Hotspot in processor.rs (MEDIUM)

**Problem:**
- Mixes provider normalization, event branching, domain mutation, callback shaping, and dispute alerting
- Forces long reasoning chains; blocks parallel agent work
- ~400+ lines of mixed concern

**Fix:** Split into:
1. `normalization.rs` — provider field extraction
2. `rules.rs` — domain branching logic
3. `callback.rs` — payload shaping
4. `dispatch.rs` — external alerting (email, logging)

### 4. Invariant Documentation (MEDIUM)

**Problem:**
- Important rules exist but scattered across code, tests, DESIGN.md
- Agents must infer constraints instead of reading them

**Examples:**
- "never use f64 for currency"
- "all lifecycle writes pass monotonic event-time guards"
- "Bridge DB is normalized source of truth"
- "webhook log is idempotency checkpoint"

**Fix:** Add `INVARIANTS.md` section: "Hard Rules for Agents" with do/don't list.

## Quick Wins

1. **Type status enum** — 2-3 hours, prevents silent bugs
2. **Fix f64 → Decimal** — 3-4 hours, payment correctness
3. **Document hard rules** — 1 hour, clarifies constraints

## Not Needed (Yet)

- Property-based invariant testing frameworks
- Autonomous improvement loops
- Heavy agent-eval infrastructure
