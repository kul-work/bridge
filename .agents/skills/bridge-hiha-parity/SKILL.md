---
name: bridge-hiha-parity
description: "Guards Bridge payment/provider work against drift from old HiHa behavior. Use when changing or investigating Bridge vs HiHa backend parity, Google Play or Creem reintegration, provider callbacks, subscription/payment flows, OTP/refund behavior, currency/amount persistence, webhook deduplication, or any request to align Bridge with old HiHa."
---

# Bridge HiHa Parity

Use this skill to prevent Bridge from rediscovering or drifting from behavior that already worked in old HiHa.

Core rule:

> Old HiHa is the behavioral oracle. Bridge must match it unless a Bridge-only invariant explicitly justifies divergence.

Old HiHa lives at:

```text
C:\share\hiha
```

Use this as the historical pre-Bridge oracle.

Do not use `C:\share\tyde\hiha` as the old-behavior oracle unless the task is explicitly about current post-Bridge HiHa integration.

Bridge lives at:

```text
C:\share\tyde\bridge
```

## Required Classification

Before coding, classify the task:

```text
PARITY: preserve old HiHa behavior exactly.
BRIDGE-ONLY: intentionally different because Bridge is multi-app/payment middleware.
UNKNOWN: stop implementation, keep investigating until old behavior or a user decision classifies it.
```

Most Google Play and Creem reintegration work should start as `PARITY`.

Valid `BRIDGE-ONLY` examples:

- RLS/app isolation
- provider config lookup
- Bridge callback delivery retries
- webhook ingress token routing
- centralized app registry
- PII minimization
- idempotent ingress logging in Bridge-specific tables

If a difference is not clearly Bridge-only, treat it as drift.

## Parity Claim

For parity tasks, state one precise claim before implementation:

```text
PARITY: For <provider> <flow>, Bridge must match old HiHa for <specific side effect>.
```

Examples:

```text
PARITY: For Google Play subscription renewal, Bridge must persist provider currency, not default USD.
PARITY: For Google Play OTP refund, Bridge must emit one refund callback, not duplicate semantic callbacks.
PARITY: For Google Play subscription renewal, Bridge must create a distinct economic payment row, not overwrite the previous one.
PARITY: For Creem payment.failed, Bridge must map the event to the same canonical payment/subscription outcome old HiHa used.
```

Do not proceed with implementation until the claim is specific enough to test.

## Required Comparison Shape

For each parity task, fill this before changing code:

```text
Old HiHa:
- file/function:
- commit/date inspected:
- exact behavior:
- relevant test/doc:

Bridge:
- file/function:
- current behavior:
- divergence:

Decision:
- PARITY or BRIDGE-ONLY:
- reason:
```

If this cannot be filled in from code, docs, tests, git history, or user-provided context, report that the task is `UNKNOWN`. Ask for the missing oracle detail only after focused investigation cannot find it.

## Workflow

1. Read Bridge `AGENTS.md`, `DESIGN.md`, and `INVARIANTS.md` when behavior or architecture may change.
2. Locate the old HiHa oracle in `C:\share\hiha`.
3. Locate the matching Bridge path in `C:\share\tyde\bridge`.
4. Identify one specific divergence.
5. Add or update one assertion proving the expected side effect.
6. Patch the smallest Bridge change needed.
7. Verify the focused test or script.
8. Record any intentional Bridge-only difference in the relevant docs or notes.

Do not refactor adjacent code unless required for the parity fix.

## Field-Level Assertions

Flow success is not enough. Payment/provider parity should assert durable side effects when relevant:

```text
provider_transaction_id
provider_purchase_token
currency
amount_cents
product_id
subscription_id
external_user_id
status
provider_status
period_start / period_end
callback event type
callback body fields
webhook dedup key
last_event_time / timestamp_epoch_ms
```

A high-level flow can pass while these fields are wrong.

## Avoid Broad Fixes

Reject broad tasks like:

```text
Fix Google Play renewal.
Fix currency.
Fix logs.
Align Bridge to HiHa.
Improve webhook processing.
```

Convert them into a narrow parity task:

```text
Find the old HiHa source of Google Play currency for subscription purchases, then make Bridge persist the same value for the equivalent flow. Do not change unrelated Google Play behavior.
```

## Logging Rule

Separate logging from behavior unless the task is explicitly about observability.

Good logs are low-noise decision points:

```text
- one log per state transition or external boundary
- include correlation IDs
- no logs inside tight retry/poll loops unless rate-limited
- no duplicate logs for the same logical event
- no secrets, tokens, or raw sensitive payloads
- every new warning/error should imply an operator action
```

If a log does not help determine what happened or what to do, do not add it.

## Deviation Ledger

When discovering a divergence, maintain or propose a small ledger entry:

```text
Flow | Behavior | Old HiHa | Bridge | Status | Intentional?
Google renewal | currency persistence | provider currency | USD | broken | no
OTP refund | callback count | one | two | broken | no
Subscription renewal | payment identity | order id row | token overwrite | fixed | no
```

Use this to keep "align to HiHa" concrete.

## Agent Constraints

- Do not let Bridge behavior be inferred from plausibility when old HiHa has an answer.
- Do not introduce a new abstraction for a parity fix unless the existing Bridge structure requires it.
- Do not broaden the patch after finding the first divergence.
- Do not treat current Bridge behavior as correct merely because tests pass.
- Do not call a difference intentional unless a Bridge invariant, doc, or user decision says so.
- Prefer one flow, one divergence, one assertion, one patch.
