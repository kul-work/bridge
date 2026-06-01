# Bridge Webhook Idempotency Checker

Use when a Bridge change touches provider webhook ingress, signature handling, deduplication, stale suppression, forwarding, callback enqueue, callback payloads, or delivery retries.

## Check

- Provider signature validation, or an explicit configured signature-skip decision, happens before mutation.
- Webhook idempotency is checked before mutating payment/subscription state.
- Primary provider webhook dedupe is app-scoped and provider-scoped.
- Deduplication does not suppress valid renewal/economic events.
- Stale events cannot overwrite newer durable state.
- Delivery enqueue is idempotent.
- One logical provider event emits at most one semantic callback.
- Failed callback delivery does not mutate provider/payment/subscription state incorrectly.
- Callback payload fields remain contract-compatible.

## Evidence to collect

- Diff hunks touching ingress, processor, forwarding, scheduler, callback payloads, or webhook DB queries.
- Tests for duplicate provider events.
- Tests for valid renewal events not being deduped away.
- Tests for stale events.
- Tests for callback enqueue idempotency and callback body fields.

## Output

```text
Webhook idempotency verdict: PASS / FAIL / NOT APPLICABLE

Findings:
- ...

Evidence:
- files:
- tests:
```
