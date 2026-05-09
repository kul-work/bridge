# Bridge Agentic Infrastructure Readiness

Date: 2026-04-19 | Updated: 2026-05-01
Repo: `tyde/bridge`

## Summary

Bridge works well for human-supervised agent assistance. One concrete infrastructure gap remains:

**Task State Persistence** — agents reconstruct "done" each session instead of loading it

Everything else works fine or has been mitigated.

## The Problem

### Task State Persistence

Bridge has static guidance (`AGENTS.md`, `DESIGN.md`, tests) but no durable machine-readable layer for:
- active task goals and constraints
- accepted tradeoffs and prior failed attempts
- subsystem-specific definitions of done
- task snapshots that new sessions can load

**Effect:** Each agent session reconstructs what "done" means instead of inheriting it. Two capable sessions can produce different outcomes on the same work.

**Current Status:** Partial mitigation with `task_list` tool. Not ideal but usable.

## Not Needed (Yet)

- Autonomous self-improving agents — premature
- Property-based invariant testing — useful, not critical
- Agent-eval harness with incident replay — too heavy
- Sandbox experiment infrastructure — overkill

## What Would Improve Things

**Document state transitions** — compact "what's a valid path" rules. Achievable in hours.
