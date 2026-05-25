# Agentic Process Guardrails for Bridge/Tyde

Date: 2026-05-25

Scope: Practical agent/sub-agent process guidance for Bridge and Tyde payment work, based on the Bridge split history and the later stabilization work through `v0.3.2`.

## 1. Core Lesson

Bridge did not mainly suffer from lack of implementation speed. It suffered from late behavior discovery, parity drift from old HiHa, and missing field-level guardrails around provider/payment side effects.

Agent orchestration can help, but not by creating a coding swarm. The useful pattern is:

```text
oracle extractor -> small implementation -> invariant enforcer -> side-effect test auditor -> skeptical reviewer -> release risk gate
```

Use sub-agents as narrow critics and evidence gatherers. Keep actual code changes small and owned by one implementer at a time.

### 1.1 Amp Implementation Map

In Amp, these roles are not background daemons or autonomous coding swarms. The practical implementation unit is a reusable skill, a shared checklist/resource, or a manual workflow invoked in a thread.

A loop is not a separate runtime object. It is a repeatable workflow recipe. If the recipe is used often, implement it as one orchestrator skill: a normal `SKILL.md` whose workflow tells Amp which phases to run, when to stop, and which other checklists/skills to apply. The orchestrator is triggered the same way as any other skill, for example: "Run `bridge-release-gate` from the latest tag." It is not a hook and it does not run in the background.

| Section | Role / loop | Suitable as Amp skill? | Practical form |
|---|---:|---|---|
| 2 | Oracle Extractor | Yes | Strengthen/use `bridge-hiha-parity` as the oracle-extraction skill. |
| 3 | Invariant Enforcer | Maybe | Prefer a shared checklist/resource consumed by reviewer/gate skills; promote to standalone skill only if invoked directly often. |
| 4 | Side-Effect Test Auditor | Yes | Standalone `bridge-side-effect-test-auditor` skill. |
| 5 | Release Risk Gate | Yes | Standalone `bridge-release-gate` skill. |
| 6 | Skeptical Reviewer | Yes | Standalone `bridge-skeptical-reviewer` skill. |
| 7.1 | Parity Loop | Optional orchestrator | Only create a `bridge-parity-loop` skill if this full sequence is invoked often; otherwise follow it manually. |
| 7.2 | Bug Fix Loop | Optional orchestrator | Useful as a Bridge payment/provider bugfix workflow; too broad as a generic bugfix skill. |
| 7.3 | Release Loop | Already implemented by skill | `bridge-release-gate` is the orchestrator for the release loop; do not create a second release-loop skill. |

Do not create one skill per heading by default. Create skills around actual Amp invocation moments: extracting old behavior, auditing side-effect tests, reviewing risky payment diffs, and preparing a release.

## 2. Oracle Extractors

An oracle extractor reads old HiHa, provider docs, existing Bridge docs, and current tests to define the exact behavior before Bridge code changes.

Use this role for:

- Google Play subscriptions, renewals, refunds, OTP, acknowledgement, price changes.
- Creem checkout, refunds, one-time payments, and webhook semantics.
- Any task described as parity, migration, reintegration, or "match old behavior".

The oracle extractor must not write code. Its job is to prevent rediscovery.

Required output:

```text
Flow:

Task classification:
- PARITY / BRIDGE-ONLY / UNKNOWN

Old HiHa oracle:
- file/function:
- trigger:
- provider fields used:
- DB writes:
- callback payload:
- duplicate/stale handling:
- relevant test/doc:

Bridge target:
- current file/function:
- current behavior:
- divergence:

Decision:
- preserve old behavior, or intentionally differ because:
```

If this cannot be filled in, the task is not ready for implementation.

Bad prompt:

```text
Fix Google Play renewal.
```

Good prompt:

```text
PARITY: For Google Play subscription renewal, find the old HiHa source of truth for payment identity and currency persistence. Return the exact DB side effects and callback fields Bridge must preserve. Do not change code.
```

## 3. Invariant Enforcers

An invariant enforcer reviews a proposed change only against Bridge invariants. It should be strict, repetitive, and boring.

Use this role after any change touching:

- `src/webhooks/*`
- `src/application/verify_purchase*`
- `src/db/payments.rs`
- `src/db/subscriptions.rs`
- `src/services/google_play/*`
- provider callbacks, migrations, subscription state, payment identity, or delivery retries.

Checklist:

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

Required output:

```json
{
  "accepted": false,
  "blocking_findings": [
    "Example: renewal path writes purchase token into provider_transaction_id, violating payment identity invariant"
  ],
  "non_blocking_notes": []
}
```

No "looks good" reviews. The enforcer must cite the invariant or state that no invariant was touched.

## 4. Side-Effect Test Auditors

A side-effect test auditor checks whether tests prove durable behavior, not just flow success.

This role exists because payment tests can pass while important fields are wrong.

Use it whenever a change affects payment verification, webhook processing, callback delivery, subscriptions, or provider normalization.

The auditor asks:

```text
What durable facts should this flow write or emit?
Are those facts asserted by tests?
```

Field-level assertions to look for:

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
webhook dedup key
app_id / external_user_id scoping
```

Examples of insufficient tests:

```text
- HTTP 200 only.
- "callback happened" without checking payload.
- payment row exists, but identity/currency/amount are not checked.
- subscription became active, but period/provider fields are not checked.
```

Required output:

```text
Test verdict: PASS / FAIL

Missing assertions:
- field:
- why it matters:
- suggested test file:
```

For Bridge, flow tests are necessary but not sufficient. Side-effect parity tests are release blockers for provider/payment work.

## 5. Release Risk Gates

A release risk gate reviews the diff since the previous tag and decides what focused checks are mandatory before tagging.

Use before every Bridge release.

The gate should classify changed areas:

```text
Provider behavior changed?
Payment identity changed?
Subscription lifecycle changed?
Webhook semantics changed?
Callback payload changed?
Migration changed?
Tenant/RLS behavior changed?
Logging-only change?
Docs-only change?
```

Then map risk to required checks:

```text
Google Play changed:
- renewal tests
- OTP tests
- refund tests
- price-change tests
- currency assertions
- purchase-token/order-ID assertions

Webhook changed:
- ingress idempotency tests
- forwarding enqueue idempotency tests
- duplicate provider event tests
- stale event tests

Subscription DB changed:
- terminal state tests
- app-scoping tests
- reconciliation tests

Payment DB changed:
- economic identity tests
- currency/amount tests
- duplicate/overwrite prevention tests
```

Required output:

```text
Release risk: LOW / MEDIUM / HIGH

Changed risk areas:
- ...

Required checks before tag:
- ...

Release notes coverage:
- sufficient / missing entries
```

The release gate should be changed-area based. "Run all tests and hope" is not enough for Bridge.

### 5.1 Minimal Amp Skill Template: `bridge-release-gate`

Create this only if release gating will be invoked repeatedly from Amp.

File path:

```text
.agents/skills/bridge-release-gate/SKILL.md
```

Template:

```md
---
name: bridge-release-gate
description: "Bridge release readiness gate. Use before tagging or releasing Bridge to classify diff risk, map changed areas to focused checks, audit release notes, and require skeptical review for high-risk payment/provider changes."
---

# Bridge Release Gate

This skill does not commit, tag, push, or run `cargo release --execute`.

## Workflow

1. Resolve the base:
   - If the user gives a tag/SHA, use it.
   - Otherwise run `git describe --tags --abbrev=0`.
2. Collect release evidence:
   - `git log --oneline BASE..HEAD`
   - `git diff --name-only BASE..HEAD`
   - `git diff --stat BASE..HEAD`
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
4. Map risk to required focused checks.
5. Run the narrowest meaningful checks first. On Windows, use:
   - `cargo check 2>&1 && echo EXIT: %ERRORLEVEL%`
6. Audit `Release Notes.md` against changed risk areas.
7. If risk is MEDIUM or HIGH, run the skeptical reviewer workflow before declaring readiness.

## Output

Release risk: LOW / MEDIUM / HIGH

Changed risk areas:
- ...

Required checks:
- ...

Checks run:
- ...

Failures:
- ...

Release notes coverage:
- sufficient / missing

Verdict:
- READY / NOT READY
```

## 6. Skeptical Reviewers

A skeptical reviewer is a narrow reviewer that assumes payment behavior can be subtly wrong even when code compiles and tests pass.

Use after implementation and before merge/tag, especially for high-churn files:

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
```

Reviewer questions:

```text
What old behavior could this accidentally change?
Can this emit duplicate semantic callbacks?
Can this suppress a valid renewal as duplicate noise?
Can a partial provider payload erase existing state?
Can a purchase token be confused with an economic transaction ID?
Can currency or amount silently default?
Can this cross app/user boundaries?
Can this make logs noisier without making diagnosis better?
```

Required output:

```text
Verdict: ACCEPT / REJECT

Blocking concerns:
- ...

Evidence checked:
- files:
- tests:
- invariants:
```

Skeptical reviewers should reject vague confidence. Evidence matters more than tone.

### 6.1 Minimal Amp Skill Template: `bridge-skeptical-reviewer`

Create this as a reusable Amp skill for risky Bridge payment/provider diffs.

File path:

```text
.agents/skills/bridge-skeptical-reviewer/SKILL.md
```

Template:

```md
---
name: bridge-skeptical-reviewer
description: "Strict Bridge payment/provider reviewer. Use after implementation touching webhooks, subscriptions, payments, provider callbacks, Google Play, Creem, migrations, or release-risk areas."
---

# Bridge Skeptical Reviewer

You are not checking formatting or general code quality. You are checking whether this diff could subtly break Bridge payment behavior.

## Required workflow

1. Run `git diff --name-only` and `git diff`.
2. If payment/provider files are touched, read:
   - `INVARIANTS.md`
   - `DESIGN.md`
   - relevant tests for the touched flow
3. Classify the risk area:
   - webhook
   - subscription lifecycle
   - payment identity
   - provider normalization
   - callback delivery
   - migration
4. Review only Bridge payment/provider risks:
   - Can this emit duplicate semantic callbacks?
   - Can this suppress a valid renewal as duplicate noise?
   - Can a partial provider payload erase durable state?
   - Can a purchase token be confused with an economic transaction ID?
   - Can currency or amount silently default?
   - Can this cross app/user boundaries?
   - Is idempotency checked before mutation?

## Output

Verdict: ACCEPT / REJECT

Blocking concerns:
- ...

Non-blocking notes:
- ...

Evidence checked:
- files:
- tests:
- invariants:
```

## 7. Workflow Loops

These loops are recipes, not Amp objects. They are implemented in one of two ways:

```text
Manual loop: user asks Amp to follow the recipe in this section for one task.
Orchestrator skill: one normal skill owns the recipe and runs the phases in order.
```

Use an orchestrator skill only when the same loop is invoked repeatedly. Otherwise, keep the loop as documentation and ask Amp to follow it explicitly in the thread.

### 7.1 Parity Loop

Use for old HiHa parity and provider reintegration.

```text
1. Oracle extractor defines old behavior.
2. Implementer makes the smallest Bridge change.
3. Invariant enforcer checks Bridge invariants.
4. Side-effect test auditor checks field-level assertions.
5. Skeptical reviewer accepts/rejects the patch.
```

Stop if classification is `UNKNOWN`.

Amp implementation: usually manual, or an optional future `bridge-parity-loop` skill if this exact sequence becomes frequent. The skill would not be a separate daemon; it would be a `SKILL.md` that tells Amp to run the phases above and stop at the phase gates.

### 7.2 Bug Fix Loop

Use for concrete production/test bugs.

```text
1. Reproduce or identify the failing behavior from raw logs/tests.
2. Classify as PARITY, BRIDGE-ONLY, or UNKNOWN.
3. If PARITY, run oracle extraction first.
4. Patch the smallest code path.
5. Add/adjust side-effect assertion that would have caught the bug.
6. Run invariant review.
```

Every payment bug should leave behind a guardrail test or a documented reason why not.

Amp implementation: usually manual. Create a dedicated payment/provider bugfix orchestrator only if these bugs become common enough that the same phase gates are repeatedly requested.

### 7.3 Release Loop

Use before `vX.Y.Z` tags. This loop is implemented by the `bridge-release-gate` skill template in section 5.1; do not create a separate release-loop skill.

```text
1. Release risk gate reviews diff since previous tag.
2. Required focused checks are listed.
3. Checks run and failures are fixed.
4. Release notes are audited against changed risk areas.
5. Skeptical reviewer signs off on high-risk changes.
```

Amp trigger examples:

```text
Run bridge-release-gate from the latest tag.
Run bridge-release-gate for v0.3.2..HEAD.
```

## 8. Anti-Patterns

Avoid these agentic patterns:

```text
Swarm coding across webhook/payment/subscription files.
Broad prompts that ask agents to infer payment behavior.
Architecture rewrites mixed into provider bug fixes.
Logging improvements bundled with behavior changes.
Tests that only check success status.
Reviews that say "looks good" without citing evidence.
Treating old HiHa as inspiration instead of oracle for parity tasks.
```

The safer rule for Bridge is:

```text
One flow. One divergence. One oracle. One small patch. One side-effect test. One invariant review.
```

## 9. Practical Task Template

Evaluation: this section is still relevant, but only as a manual intake template for custom Bridge payment/provider work that is not already covered by a narrower skill. It should not be pasted at the end of every task, and it should not replace the dedicated Amp skills above.

Use it when a task is too ambiguous to safely classify, or when the request mixes parity, provider behavior, side effects, and release risk.

```text
Task type: PARITY / BRIDGE-ONLY / UNKNOWN

Flow:

Parity claim or Bridge-only reason:

Oracle required:
- yes/no

Files likely involved:

Allowed scope:

Forbidden scope:

Required side-effect assertions:
- provider_transaction_id:
- provider_purchase_token:
- currency:
- amount_cents:
- callback payload:
- app/user scoping:

Invariant review required:
- money identity
- lifecycle monotonicity
- webhook idempotency
- callback duplication
- tenant isolation

Verification command(s):
```

If a task cannot be expressed in this shape, it is probably too broad for safe agent implementation.
