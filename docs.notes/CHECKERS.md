# Bridge Checkers

Conditional checklists and templates invoked by `AGENTS.md` rules or by skills at boundaries (release, review). Read the relevant section when the trigger condition matches — not all sections at once.

## 1. Before Changing Payment/Provider Code

**Trigger:** any change to `src/webhooks/*`, `src/application/verify_purchase*`, `src/db/payments.rs`, `src/db/subscriptions.rs`, `src/services/google_play/*`, or provider callback paths.

### 1.1 Classify the Task

```text
PARITY: must match old HiHa behavior.
BRIDGE-ONLY: intentionally differs (RLS, idempotent ingress, multi-app, PII minimization).
UNKNOWN: stop implementation, investigate until classified.
```

If PARITY → use `bridge-hiha-parity` skill before implementing. \
If UNKNOWN → do not implement. Gather evidence and classify first.

### 1.2 Parity Claim Template

For PARITY tasks, state one precise claim before writing code:

```text
PARITY: For <provider> <flow>, Bridge must match old HiHa for <specific side effect>.

Old HiHa:
- file/function:
- trigger:
- provider fields used:
- DB writes:
- callback payload:
- duplicate/stale handling:

Bridge:
- current file/function:
- current behavior:
- divergence:

Decision:
- preserve old behavior, or intentionally differ because:
```

If this cannot be filled in, the task is not ready for implementation.

### 1.3 Field-Level Assertions

Flow success is not enough. After changing payment/provider code, verify tests assert durable side effects when relevant:

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

If no assertions exist for the affected fields → add them.

### 1.4 Bug Fix Guardrail

When fixing a payment/provider bug:

1. Reproduce or identify the failing behavior from raw logs/tests.
2. Classify: PARITY, BRIDGE-ONLY, or UNKNOWN (§1.1).
3. If PARITY → run oracle extraction first (§1.2).
4. Patch the smallest code path.
5. Add/adjust an assertion that would have caught the bug.
6. Every payment bug leaves behind a guardrail test or a documented reason why not.

---

## 2. Invariant Checklist

**Trigger:** consumed by skeptical reviewer and release gate skills. Also apply manually when reviewing diffs touching the areas listed in §1.

### Money

- No f64/f32 currency handling
- Integer cents only
- Currency source is explicit
- `provider_transaction_id` is economic transaction/order ID
- Google Play purchase token stored only in dedicated token fields

### Lifecycle

- Newer timestamp wins
- Stale events cannot overwrite newer state
- Terminal states are respected
- Partial provider events cannot erase durable subscription fields

### Webhooks

- Provider signature validation happens before mutation
- Idempotency is checked before mutation
- Deduplication does not suppress valid renewal/economic events
- Delivery enqueue is idempotent
- Duplicate callbacks are not emitted for one logical event

### Tenancy

- App scope is explicit
- RLS-compatible query path
- No cross-app lookup by provider/user identifiers alone

### Layer Boundaries

- Handlers orchestrate only
- DB layer remains query-only
- Provider services translate provider concepts, not Bridge policy

---

## 3. Side-Effect Test Auditor

**Trigger:** after writing or modifying tests for payment verification, webhook processing, callback delivery, subscriptions, or provider normalization.

Ask:

```text
What durable facts should this flow write or emit?
Are those facts asserted by tests?
```

Examples of insufficient tests:

- HTTP 200 only
- "callback happened" without checking payload
- payment row exists, but identity/currency/amount are not checked
- subscription became active, but period/provider fields are not checked

Output:

```text
Test verdict: PASS / FAIL

Missing assertions:
- field:
- why it matters:
- suggested test file:
```

---

## 4. Skeptical Review Questions

**Trigger:** before merge/tag, especially for high-churn files:
`src/webhooks/processor.rs`, `src/webhooks/ingress.rs`, `src/webhooks/forwarding.rs`, `src/webhooks/scheduler.rs`, `src/application/verify_purchase.rs`, `src/application/verify_purchase_provider.rs`, `src/db/payments.rs`, `src/db/subscriptions.rs`, `src/services/google_play/*`

Answer every question with evidence, not confidence:

- What old behavior could this accidentally change?
- Can this emit duplicate semantic callbacks?
- Can this suppress a valid renewal as duplicate noise?
- Can a partial provider payload erase existing state?
- Can a purchase token be confused with an economic transaction ID?
- Can currency or amount silently default?
- Can this cross app/user boundaries?
- Can this make logs noisier without making diagnosis better?

Output:

```text
Verdict: ACCEPT / REJECT

Blocking concerns:
- ...

Evidence checked:
- files:
- tests:
- invariants:
```

---

## 5. Release Risk Classification

**Trigger:** before every Bridge release. Use `bridge-release-gate` skill for the full workflow.

### 5.1 Risk Area Classification

For each change since the previous tag, classify:

```text
Provider behavior changed?      → Google Play / Creem
Payment identity changed?        → provider_transaction_id, purchase token confusion
Subscription lifecycle changed?  → status transitions, terminal states
Webhook semantics changed?       → ingress, forwarding, dedup
Callback payload changed?        → event type, body fields
Migration changed?               → schema, data migration
Tenant/RLS behavior changed?     → app scoping, cross-app isolation
Logging-only change?
Docs-only change?
```

### 5.2 Risk → Required Checks Mapping

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

Callback payload changed:
- field-level assertions per §1.3
- side-effect audit per §3
```

### 5.3 Output

```text
Release risk: LOW / MEDIUM / HIGH

Changed risk areas:
- ...

Required checks before tag:
- ...

Release notes coverage:
- sufficient / missing entries
```

---

## 6. Fallback Intake Template

**Trigger:** when a task cannot be cleanly classified as PARITY or BRIDGE-ONLY, or when it mixes provider behaviors, side effects, and custom logic. Use this to force-clarify scope before starting implementation.

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

If a task cannot be expressed in this shape, it is probably too broad for safe implementation.

---

## 7. Anti-Patterns

```text
Swarm coding across webhook/payment/subscription files.
Broad prompts that ask agents to infer payment behavior.
Architecture rewrites mixed into provider bug fixes.
Logging improvements bundled with behavior changes.
Tests that only check success status.
Reviews that say "looks good" without citing evidence.
Treating old HiHa as inspiration instead of oracle for parity tasks.
```

The safer rule for Bridge:

```text
One flow. One divergence. One oracle. One small patch. One side-effect test. One invariant review.
```