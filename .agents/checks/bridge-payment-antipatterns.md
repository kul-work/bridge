# Bridge Payment Anti-Patterns Checker

Use when reviewing payment/provider implementation plans, broad diffs, or any change that feels larger than one flow or one divergence.

## Reject these patterns

- Swarm coding across webhook, payment, subscription, and provider files in one unbounded pass.
- Broad prompts that ask agents to infer payment behavior instead of finding the old HiHa oracle or Bridge invariant.
- Architecture rewrites mixed into provider bug fixes.
- Logging improvements bundled with behavior changes unless observability is the explicit task.
- Tests that only check HTTP success or row existence.
- Reviews that say "looks good" without citing files, tests, invariants, or diff evidence.
- Treating old HiHa as inspiration instead of oracle for parity tasks.
- Introducing a new abstraction for a one-flow parity fix without a concrete reuse or complexity reduction.
- Duplicating state to fix a symptom instead of correcting the source of truth.
- Calling a Bridge-vs-HiHa difference intentional without an invariant, design note, or user decision.

## Safer shape

```text
One flow.
One divergence.
One oracle or Bridge-only reason.
One small patch.
One side-effect assertion.
One invariant review.
```

## Output

```text
Anti-pattern verdict: PASS / FAIL / NOT APPLICABLE

Findings:
- anti-pattern:
- evidence:
- safer split or next step:
```
