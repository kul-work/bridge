# Bridge Agentic Readiness Audit - Gaps

## Gap Analysis (Filtered)

| Dimension | Gap |
| --- | --- |
| Context Hygiene | There is no dedicated invariant manifesto for money, lifecycle states, or canonical domain rules. Important rules exist, but not as a compact "do/don't" source of truth for agents. |
| Type-State Pattern | Lifecycle mutations still run mostly on raw strings across application, DB, and webhook layers. Invalid states remain representable, and unknown states can silently collapse to `Expired`. |
| Verification Layer | There is no property-based or formal invariant testing. Idempotency, stale-event suppression, and monotonic ordering are tested with examples, not generative/state-model methods. |
| Idiomatic Rust & Performance | Status and error handling are string-heavy. Currency parsing still uses `f64` in a few paths, which is not ideal for payment code. |
| Modular Decoupling | `src/webhooks/processor.rs` remains a large context hotspot that mixes normalization, branching, mutation, and callback shaping, making parallel agent work harder. |

## Detailed Gaps

### 1. Context Hygiene
- There is no compact invariant file that states hard payment rules such as:
  - never use floating-point for currency math
  - never use raw status strings in domain code
  - all lifecycle writes must pass monotonic event-time guards
  - Bridge DB is the normalized source of truth
- There is also drift between documented states and actual runtime values. For example, the subscription migration documents statuses like `pending`, `active`, `trial`, `past_due`, `cancelled`, `expired`, `revoked`, `paused`, and `on_hold`, while the code also writes `replaced`.

**Impact:** Agents can follow broad architecture, but they do not yet have a strict invariant contract that prevents subtle hallucinations around money, state transitions, and persistence semantics.

### 2. Type-State Pattern
- The payment lifecycle is not implemented as a Rust typestate system.
- Important flows still depend on raw string fields such as `VerifyPurchaseResponse.status`, `VerifiedPurchase.status`, `VerifyPurchaseCommitRequest.subscription_status`, and DB models with `status: String`.
- Unknown status parsing currently falls back to `Expired` in `SubscriptionStatus::from(&str)`, which is dangerous because it can silently coerce novel provider states into a terminal state.

**Impact:** Invalid transitions like refunding the wrong payment state, resuming the wrong subscription state, or introducing undocumented statuses remain representable until runtime.

### 3. Verification Layer
- `Cargo.toml` has no `proptest`, `quickcheck`, `loom`, `kani`, `bolero`, or similar tooling.
- There are no model-based tests for key invariants such as:
  - replaying the same webhook is a no-op
  - higher `last_event_time` always wins
  - payment refund application is idempotent
  - normalized status mapping is closed and lossless for supported providers

**Impact:** The repo proves important scenarios, but not the state-space around them. For a payments middleware, this leaves avoidable risk in ordering, duplication, and provider drift.

### 4. Idiomatic Rust & Performance
- Currency parsing uses `f64` in several places:
  - checkout amount formatting from `amount_cents`
  - provider response parsing for major-unit amounts
  - Coinbase charge summation and webhook amount extraction
- Error ergonomics are broad and stringly typed. `BridgeError` and `AppError` rely heavily on `String` payloads instead of narrower typed sub-errors.
- The webhook normalization layer creates many owned `String` values early, even when the data is only used transiently.

**Impact:** The biggest issue here is correctness and clarity. The float usage is the most important late-2025/2026 anti-pattern for payment code.

### 5. Modular Decoupling
- `src/webhooks/processor.rs` is still a large, mixed-responsibility module that handles provider field extraction, event normalization, domain branching, mutation coordination, callback payload shaping, Coinbase topups, and dispute email alerting.

**Impact:** That file is the primary context-window saturation risk for specialized agents. It is likely the first module that should be split if the goal is high-quality parallel agent work.

## Actionable Refactors to Address Gaps
1. Replace raw status strings with canonical typed domains.
2. Add a real lifecycle transition layer with typed guards.
3. Introduce a money value object and remove `f64` currency parsing.
4. Add property-based invariant tests for ordering and idempotency.
5. Split `webhooks/processor.rs` into focused modules.
