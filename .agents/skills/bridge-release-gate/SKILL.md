---
name: bridge-release-gate
description: "Classifies Bridge release risk from a diff since a tag/SHA and maps changed payment/provider areas to focused checks. Use before every Bridge release or tag."
---

# Bridge Release Gate

Use this skill before tagging or releasing Bridge. It classifies release risk from the changed areas and lists the focused checks that must pass before a release can proceed.

This skill does not commit, tag, push, run `cargo release --execute`, or replace the full release loop. It only produces a risk gate verdict and required checks.

## Required Inputs

- Base tag/SHA from the user, or the latest reachable tag if omitted.
- Current `HEAD` as the release candidate.

If the base cannot be resolved, stop and report the blocker. Do not guess a release range.

## Workflow

1. Resolve the base:
   - If the user gives a tag/SHA/range, use it.
   - Otherwise run `git describe --tags --abbrev=0`.
2. Collect release evidence:
   - `git log --oneline BASE..HEAD`
   - `git diff --name-only BASE..HEAD`
   - `git diff --stat BASE..HEAD`
   - For risky files, inspect the relevant diff hunks with `git diff BASE..HEAD -- <path>`.
3. Classify changed risk areas:
   - provider behavior
   - payment identity
   - subscription lifecycle
   - webhook semantics
   - callback payload
   - migration
   - tenant/RLS behavior
   - logging-only
   - docs-only
4. Map changed areas to mandatory focused checks.
5. Audit `Release Notes.md` only for coverage of user-visible or operator-relevant changes. Do not rewrite release notes unless the user explicitly asks.
6. Decide whether `bridge-skeptical-reviewer` is required.

## Risk Classification

Use the highest applicable risk level.

### HIGH

Use `HIGH` if the release changes any of:

- provider webhook processing or provider event normalization
- payment economic identity, `provider_transaction_id`, purchase token handling, amount, or currency
- subscription lifecycle state transitions, terminal state handling, stale-event suppression, or reconciliation behavior
- callback payload semantics, callback event type, delivery idempotency, or retry semantics
- migrations touching payment/subscription/webhook/app tenancy tables
- tenant/RLS scope for payment, subscription, app, provider, or webhook queries

### MEDIUM

Use `MEDIUM` if the release changes:

- admin/operator views over payment/provider state without changing persisted semantics
- observability that affects diagnosis of payment/provider incidents
- non-payment API behavior that still touches authenticated app boundaries
- tests around payment/provider behavior without production changes

### LOW

Use `LOW` only when changes are limited to:

- docs-only edits
- release notes only
- logging text with no semantic change and no high-risk files touched
- isolated refactors proven not to change payment/provider behavior

Do not mark a release `LOW` just because all tests pass.

## Required Check Mapping

Use changed areas, not broad hope, to choose checks.

```text
Google Play changed:
- renewal tests
- OTP tests
- refund tests
- price-change tests
- currency assertions
- purchase-token/order-ID assertions

Creem changed:
- checkout/session tests
- webhook event mapping tests
- refund/payment failure tests
- amount/currency assertions
- callback payload assertions

Webhook changed:
- provider signature validation tests
- ingress idempotency tests
- forwarding enqueue idempotency tests
- duplicate provider event tests
- stale event tests

Subscription DB/lifecycle changed:
- terminal state tests
- stale/newer timestamp tests
- app-scoping tests
- reconciliation tests
- period/provider field assertions

Payment DB/identity changed:
- economic identity tests
- provider purchase token vs transaction ID assertions
- currency/amount tests
- duplicate/overwrite prevention tests
- app/user scoping tests

Callback delivery changed:
- one logical event emits one semantic callback
- retry enqueue is idempotent
- callback body fields match contract
- failed deliveries do not mutate provider state

Migration changed:
- migration apply check
- query compile/type check for affected tables
- rollback notes or deploy risk note if rollback is not supported

Tenant/RLS changed:
- app-scoping tests
- no lookup by provider/user identifier without app scope
- admin access path remains explicit
```

Always include the normal project compile/test baseline required by the repository guidance when this gate is part of implementation or release work. This skill’s value is the focused check list on top of the baseline.

## Invariant Checklist

Apply this checklist to any high-risk change and cite violations as blockers:

```text
Money:
- no f64/f32 currency handling
- integer cents only
- currency source is explicit
- provider_transaction_id is economic transaction/order ID
- Google Play purchase token is stored only in dedicated token fields

Lifecycle:
- newer timestamp wins
- stale events cannot overwrite newer state
- terminal states are respected
- partial provider events cannot erase durable subscription fields

Webhooks:
- provider signature validation happens before mutation
- idempotency is checked before mutation
- deduplication does not suppress valid renewal/economic events
- delivery enqueue is idempotent
- duplicate callbacks are not emitted for one logical event

Tenancy:
- app scope is explicit
- RLS-compatible query path
- no cross-app lookup by provider/user identifiers alone

Boundaries:
- handlers orchestrate only
- DB layer remains query-only
- provider services translate provider concepts, not Bridge policy
```

## Output

Return this shape exactly enough for a release loop to consume:

```text
Release range: BASE..HEAD

Release risk: LOW / MEDIUM / HIGH

Reason:
- ...

Changed risk areas:
- area: evidence from file(s) or diff

Required checks before tag:
- command or focused test:
- why required:

Release notes coverage:
- sufficient / missing entries
- evidence:

Skeptical reviewer required:
- yes / no
- reason:

Blockers:
- ...
```

If no payment/provider risk areas were touched, say which evidence supports that conclusion.
