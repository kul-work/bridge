# Bridge Release Risk Checker

Use before every Bridge release or tag. Prefer the `bridge-release-gate` skill for the full workflow; this checker is the focused risk checklist.

## Classify changed areas

For the release diff, classify whether any change affects:

- provider behavior: Google Play / Creem
- payment identity: transaction ID, purchase token, amount, currency
- subscription lifecycle: status transitions, terminal states, periods, reconciliation
- webhook semantics: ingress, signature, forwarding, dedupe, stale suppression
- callback payload: event type or body fields
- migration: schema or data migration
- tenant/RLS behavior: app scoping or cross-app isolation
- logging-only
- docs-only

## Required checks by risk area

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
- configured signature-skip tests when relevant
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
- purchase token vs transaction ID assertions
- currency/amount tests
- duplicate/overwrite prevention tests
- app/user scoping tests

Migration changed:

- migration apply check
- affected query compile/type check
- deploy/rollback risk note if rollback is not supported

## Output

```text
Release risk: LOW / MEDIUM / HIGH

Changed risk areas:
- ...

Required checks before tag:
- command or focused test:
- why required:

Release notes coverage:
- sufficient / missing entries

Skeptical reviewer required:
- yes / no

Blockers:
- ...
```
