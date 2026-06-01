# Bridge Subscription Lifecycle Checker

Use when a Bridge change touches subscription status, periods, renewal, cancel, pause, resume, defer, refund, revoke, price consent, reconciliation, or subscription persistence.

## Check

- Newer provider timestamp wins over stale events.
- Stale events cannot overwrite newer subscription state.
- Terminal states are respected.
- Partial provider events do not erase durable subscription fields.
- Status transitions match Bridge canonical lifecycle rules.
- Period start/end are preserved or updated only from an explicit source.
- Renewal creates/preserves the correct economic side effects.
- Refund/revoke/cancel behavior does not emit duplicate semantic callbacks.
- Subscription lookup/write paths include explicit app scope.

## Evidence to collect

- Diff hunks touching subscription state or period fields.
- Tests for stale/newer timestamp ordering.
- Tests for terminal state handling.
- Tests asserting status and period fields.
- Tests or query evidence showing app-scoped subscription access.

## Output

```text
Subscription lifecycle verdict: PASS / FAIL / NOT APPLICABLE

Findings:
- ...

Evidence:
- files:
- tests:
```
