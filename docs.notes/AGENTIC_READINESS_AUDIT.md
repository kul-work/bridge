# Bridge Agentic Readiness Audit

Date: 2026-04-19
Repo: `tyde/bridge`

## Summary

Bridge shows moderate agentic readiness.

The repo already has useful agent guidance, a clear top-level module split, and a clean toolchain baseline: `cargo check`, `cargo test`, and `cargo clippy --all-targets -- -D warnings` all pass.

The main debt is not basic code quality. The bigger gap is that core payment invariants are still enforced mostly through comments, SQL branches, and string conventions rather than Rust types, fallible parsers, and invariant-driven tests.

## Gap Analysis

| Dimension | Evidence | Gap | Rating |
| --- | --- | --- | --- |
| Context Hygiene | `AGENTS.md` defines architecture, idempotency, stale-event suppression, and PII rules. `.windsurfrules` adds workflow guidance for code exploration. | There is no dedicated invariant manifesto for money, lifecycle states, or canonical domain rules. Important rules exist, but not as a compact "do/don't" source of truth for agents. | Partial |
| Type-State Pattern | `src/services/payment.rs` contains a normalized `SubscriptionStatus` enum. | Lifecycle mutations still run mostly on raw strings across application, DB, and webhook layers. Invalid states remain representable, and unknown states can silently collapse to `Expired`. | Weak |
| Verification Layer | There is meaningful scenario coverage via `tests/gpbi/` plus focused unit tests in webhook and provider parsing code. | There is no property-based or formal invariant testing. Idempotency, stale-event suppression, and monotonic ordering are tested with examples, not generative/state-model methods. | Weak to Moderate |
| Idiomatic Rust & Performance | The codebase is clippy-clean and generally readable. | Status and error handling are string-heavy. Currency parsing still uses `f64` in a few paths, which is not ideal for payment code. | Moderate |
| Modular Decoupling | The repo has a good top-level split: `application`, `db`, `handlers`, `ports`, `services`, `webhooks`, `middleware`. | `src/webhooks/processor.rs` remains a large context hotspot that mixes normalization, branching, mutation, and callback shaping, making parallel agent work harder. | Moderate |

## Detailed Findings

### 1. Context Hygiene

Good:

- `AGENTS.md` documents the architecture and several real domain rules, including:
  - minimize PII storage
  - idempotency first
  - stale event suppression using `timestamp_epoch_ms`
- `.windsurfrules` also helps agent workflow by pushing structural exploration first.

Gap:

- There is no compact invariant file that states hard payment rules such as:
  - never use floating-point for currency math
  - never use raw status strings in domain code
  - all lifecycle writes must pass monotonic event-time guards
  - Bridge DB is the normalized source of truth
- There is also drift between documented states and actual runtime values. For example, the subscription migration documents statuses like `pending`, `active`, `trial`, `past_due`, `cancelled`, `expired`, `revoked`, `paused`, and `on_hold`, while the code also writes `replaced`.

Impact:

- Agents can follow broad architecture, but they do not yet have a strict invariant contract that prevents subtle hallucinations around money, state transitions, and persistence semantics.

### 2. Type-State Pattern

Good:

- The repo has a canonical `SubscriptionStatus` enum in `src/services/payment.rs`.
- Some transition intent is visible in `SubscriptionWebhookTransition` usage and in dedicated DB mutation branches.

Gap:

- The payment lifecycle is not implemented as a Rust typestate system.
- Important flows still depend on raw string fields such as:
  - `VerifyPurchaseResponse.status: String`
  - `VerifiedPurchase.status: String`
  - `VerifyPurchaseCommitRequest.subscription_status: &'a str`
  - `VerifyPurchaseCommitRequest.payment_status: &'a str`
  - DB models with `status: String`
- Unknown status parsing currently falls back to `Expired` in `SubscriptionStatus::from(&str)`, which is dangerous because it can silently coerce novel provider states into a terminal state.

Impact:

- Invalid transitions like refunding the wrong payment state, resuming the wrong subscription state, or introducing undocumented statuses remain representable until runtime.

### 3. Verification Layer

Good:

- The repo has practical scenario coverage in `tests/gpbi/` for:
  - cancellations
  - refunds
  - webhook idempotency
  - ACK flows
  - pause/resume flows
- Unit tests cover normalization logic and parsing helpers.

Gap:

- `Cargo.toml` has no `proptest`, `quickcheck`, `loom`, `kani`, `bolero`, or similar tooling.
- There are no model-based tests for key invariants such as:
  - replaying the same webhook is a no-op
  - higher `last_event_time` always wins
  - payment refund application is idempotent
  - normalized status mapping is closed and lossless for supported providers

Impact:

- The repo proves important scenarios, but not the state-space around them. For a payments middleware, this leaves avoidable risk in ordering, duplication, and provider drift.

### 4. Idiomatic Rust & Performance

Good:

- The code compiles cleanly and passes strict clippy.
- `unwrap()` is not broadly abused in production code.
- The code is mostly readable and pragmatic.

Gap:

- Currency parsing uses `f64` in several places:
  - checkout amount formatting from `amount_cents`
  - provider response parsing for major-unit amounts
  - Coinbase charge summation
  - Coinbase webhook amount extraction
- Error ergonomics are broad and stringly typed. `BridgeError` and `AppError` rely heavily on `String` payloads instead of narrower typed sub-errors.
- The webhook normalization layer creates many owned `String` values early, even when the data is only used transiently.

Impact:

- The biggest issue here is correctness and clarity, not raw speed. The float usage is the most important late-2025/2026 anti-pattern for payment code.

### 5. Modular Decoupling

Good:

- The crate is reasonably split into layers that map well to agent roles:
  - `handlers` for HTTP/API
  - `application` for orchestration
  - `services` for provider integrations
  - `db` for persistence
  - `ports` for trait boundaries
  - `webhooks` for ingress/processing/scheduling

- `ports/traits` and `ports/composites` provide a useful interface seam for more specialized work.

Gap:

- `src/webhooks/processor.rs` is still a large, mixed-responsibility module that handles:
  - provider field extraction
  - event normalization
  - domain branching
  - mutation coordination
  - callback payload shaping
  - Coinbase topups
  - dispute email alerting

Impact:

- That file is the primary context-window saturation risk for specialized agents. It is likely the first module that should be split if the goal is high-quality parallel agent work.

## 5 High-Impact Actionable Refactors

1. Replace raw status strings with canonical typed domains.

   Introduce `PaymentStatus` and make `SubscriptionStatus` the only legal normalized state types in Rust domain code. Remove silent fallbacks like unknown status -> `Expired`, and add DB-level status constraints so runtime writes cannot drift from the schema.

2. Add a real lifecycle transition layer with typed guards.

   Start with a small typestate or transition API around the high-risk payment/subscription states: `Pending`, `Active`, `Paused`, `Cancelled`, `Revoked`, and `Refunded`. The goal is to make invalid transitions impossible or centrally rejected, instead of relying on distributed string comparisons.

3. Introduce a money value object and remove `f64` currency parsing.

   Use `rust_decimal` or a strict minor-unit parser, normalize immediately into a `MoneyCents` newtype, and carry only integer minor units through domain/storage layers. This is the highest-value correctness refactor for payment handling.

4. Add property-based invariant tests for ordering and idempotency.

   Add `proptest` and write a small state-model test suite that verifies monotonic `last_event_time`, duplicate webhook no-op behavior, refund idempotency, and normalization invariants across supported providers.

5. Split `webhooks/processor.rs` into focused modules.

   Separate:
   - provider field extraction
   - canonical event normalization
   - subscription lifecycle routing
   - payment-side events
   - callback payload construction

   This will improve agent parallelism, reduce review load, and make invariant testing easier.

## Proposed Context Rules Snippet

Add the following to `AGENTS.md` or a dedicated repo context file:

```md
## Context Rules

- Currency math is never done with `f32` or `f64`.
- Parse provider money with `rust_decimal` or provider minor units, then convert once into `MoneyCents(i64)`.
- Persist only minor units (`*_cents` / `*_minor`) in the database.

- Subscription and payment state must never travel as raw strings in Rust domain code.
- Use canonical enums (`SubscriptionStatus`, `PaymentStatus`) with fallible parsing.
- Unknown provider states must return an error and be logged with provider/event context; never coerce to `expired` or another fallback state.

- All lifecycle mutations go through typed transition functions.
- Invalid transitions must be unrepresentable or rejected in one place.
- `last_event_time` is monotonic and must be checked before every state mutation.

- Bridge DB is the source of truth for normalized state.
- Provider-specific fields are audit data and must not bypass normalization rules.
- Do not duplicate lifecycle state across tables to fix symptoms.

- Idempotency is required at every external boundary.
- Check webhook/event dedupe state before side effects.
- Replayed events must be a no-op, not a second mutation.

- Store opaque identifiers by default.
- Do not persist email/name unless the flow explicitly requires it for compliance or notification.

- New lifecycle logic must ship with:
- 1 property/model test for ordering or idempotency
- 1 integration-path test for the real provider flow it affects

- Keep files small enough for single-agent reasoning.
- Target <300 LOC per module for dispatchers/normalizers/handlers.
- Split provider normalization, mutation logic, and callback shaping into separate modules.
```

## Verification Notes

This audit was validated against the current codebase and verified with:

- `cargo check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`

All three passed at audit time.
