---
name: bridge-skeptical-reviewer
description: "Strictly reviews risky Bridge payment/provider diffs for subtle behavioral regressions. Use after implementation touching webhooks, subscriptions, payments, callbacks, Google Play, Creem, migrations, or release-risk areas."
---

# Bridge Skeptical Reviewer

Use this skill after implementation and before merge/tag for risky Bridge payment/provider changes. Assume the code can compile and tests can pass while payment behavior is still subtly wrong.

This skill is not a formatter, style reviewer, or general code-quality pass. It reviews only Bridge payment/provider behavior and release-risk invariants.

## Review Scope

Use this skill when a diff touches or affects:

```text
src/webhooks/processor.rs
src/webhooks/ingress.rs
src/webhooks/forwarding.rs
src/webhooks/scheduler.rs
src/application/verify_purchase.rs
src/application/verify_purchase_provider.rs
src/db/payments.rs
src/db/subscriptions.rs
src/services/google_play/*
src/services/creem/*
migrations/* payment/subscription/webhook/provider/app tables
provider callbacks
subscription state
payment identity
delivery retries
release-risk areas from bridge-release-gate
```

If the diff is docs-only or clearly unrelated to payment/provider behavior, state that and return `ACCEPT` with evidence.

## Required Workflow

1. Collect the diff:
   - `git diff --name-only`
   - `git diff`
   - If the user provides a base/range, use `git diff BASE..HEAD` instead.
2. Classify the touched risk areas:
   - webhook
   - subscription lifecycle
   - payment identity
   - provider normalization
   - callback delivery
   - migration
   - tenant/RLS behavior
   - logging-only
   - docs-only
3. If payment/provider files are touched, read:
   - `INVARIANTS.md`
   - `DESIGN.md`
   - `docs/BEHAVIORAL_SPEC.md` or `docs/WEBHOOK_ARCHITECTURE.md` when webhook/storage behavior is touched
   - relevant tests for the touched flow
4. Review only the risky behavior. Do not broaden into unrelated refactors.
5. Reject vague confidence. Every acceptance or blocker must cite evidence from files, tests, invariants, or the diff.

## Skeptical Questions

Ask these questions against the diff:

```text
What old behavior could this accidentally change?
Can this emit duplicate semantic callbacks?
Can this suppress a valid renewal as duplicate noise?
Can a partial provider payload erase existing state?
Can a purchase token be confused with an economic transaction ID?
Can currency or amount silently default?
Can this cross app/user boundaries?
Can this make logs noisier without making diagnosis better?
Is provider signature validation, or an explicit configured signature-skip decision, resolved before mutation?
Is webhook idempotency checked in `webhook_provider` before mutation?
Does newer timestamp win over stale events?
Are terminal states respected?
Does the test assert durable side effects, not just success?
```

## Invariant Checklist

Cite these invariants directly when accepting or rejecting.

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
- provider signature validation, or an explicit configured signature-skip decision, happens before mutation
- idempotency is checked in `webhook_provider` before mutation
- deduplication does not suppress valid renewal/economic events
- primary dedupe is app-scoped `(app_id, provider, provider_webhook_id)`, not purchase token + event type
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

## Test Evidence Expectations

Flow success is not enough. For payment/provider changes, check whether tests assert durable facts such as:

```text
provider_transaction_id
provider_purchase_token
currency
amount_cents
product_id
subscription_id
status
period_start / period_end
callback event type
callback body fields
webhook_provider dedup key
app_id / external_user_id scoping
```

Treat missing side-effect assertions as blocking when the changed behavior could corrupt money, subscription state, callback semantics, or tenancy.

## Verdict Rules

Return `REJECT` if any of these are true:

- a Bridge invariant is violated or not demonstrably preserved
- payment identity can confuse purchase tokens with economic transaction/order IDs
- amount or currency can default silently on a provider path that should provide it
- duplicate or missing semantic callbacks are plausible from the diff
- stale/partial provider events can overwrite durable state
- app scope is missing from payment/subscription/provider lookups
- idempotency or signature validation/signature-skip decision happens after mutation
- tests do not assert critical durable side effects for the changed risky flow

Return `ACCEPT` only when the diff is either out of scope or the relevant risks are checked with evidence.

## Output

Use this format:

```text
Verdict: ACCEPT / REJECT

Risk areas reviewed:
- ...

Blocking concerns:
- finding:
- evidence:
- invariant:
- required fix or test:

Non-blocking notes:
- ...

Evidence checked:
- files:
- tests:
- invariants:
- commands/diff range:
```

Do not write "looks good" without evidence. If the diff is too large to review safely, reject with a request to split by flow or risk area.
