# Bridge Agentic Infrastructure Readiness

Date: 2026-04-19 | Updated: 2026-05-01
Repo: `tyde/bridge`

## Summary

Bridge works well for human-supervised agent assistance. For higher parallelization and reliability, three concrete infrastructure gaps need fixing:

1. **No structured task state persistence** — agents reconstruct "done" each session instead of loading it
2. **Context hotspots** — `src/webhooks/processor.rs` mixes too many concerns, forcing long reasoning chains
3. **Type safety gaps** — raw status strings and `f64` currency parsing create silent failure modes

Everything else works fine.

## The Three Real Problems

### 1. Task State Persistence

Bridge has static guidance (`AGENTS.md`, `DESIGN.md`, tests) but no durable machine-readable layer for:
- active task goals and constraints
- accepted tradeoffs and prior failed attempts
- subsystem-specific definitions of done
- task snapshots that new sessions can load

**Effect:** Each agent session reconstructs what "done" means instead of inheriting it. Two capable sessions can produce different outcomes on the same work.

**Current Status:** Partial mitigation with `task_list` tool. Not ideal but usable.

### 2. Context Hotspot in processor.rs

`src/webhooks/processor.rs` mixes provider normalization, event branching, domain mutation, callback shaping, and dispute alerting in one module.

**Effect:** Makes parallel agent work harder. First module to split if scaling agent work.

**Action:** Break into: normalization → domain rules → callback payload → external alerting.

### 3. Type Safety & Correctness Issues

- **Raw status strings** — lifecycle states use `String` instead of typed enums. Unknown states silently coerce to `Expired`.
- **f64 currency parsing** — several paths parse amounts as floats, not fixed decimals.

**Effect:** Invalid state transitions and rounding errors remain representable until runtime.

**Action:** 
1. Create `SubscriptionStatus` enum with all valid states
2. Replace f64 currency with `Decimal` or `i64` cents

## Not Needed (Yet)

- Autonomous self-improving agents — premature
- Property-based invariant testing — useful, not critical
- Agent-eval harness with incident replay — too heavy
- Sandbox experiment infrastructure — overkill

## What Would Actually Improve Things

1. **Type the status enum** — prevents hallucinations and silent bugs
2. **Fix f64 currency parsing** — correctness issue, payment-critical
3. **Split processor.rs** — enables parallel work, improves clarity
4. **Document state transitions** — compact "what's a valid path" rules

These are achievable in weeks, not quarters.
