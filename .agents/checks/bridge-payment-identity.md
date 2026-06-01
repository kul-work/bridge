# Bridge Payment Identity Checker

Use when a Bridge change touches payment rows, provider transaction identifiers, purchase tokens, order IDs, amount, currency, or app/user payment scoping.

## Check

- `provider_transaction_id` is the economic transaction/order ID, not a Google Play purchase token.
- Provider purchase tokens are stored only in dedicated token fields.
- Amounts are integer cents; no `f32`/`f64` money handling.
- Currency source is explicit and provider-derived when the provider supplies it.
- Amount and currency do not silently default on provider paths where the provider should supply them.
- Payment lookup/write paths include explicit app scope.
- No cross-app lookup uses provider/user identifiers alone.
- Duplicate handling does not overwrite a distinct economic event.

## Evidence to collect

- Diff hunks touching payment identity fields.
- Tests asserting transaction ID vs purchase token separation.
- Tests asserting `amount_cents` and `currency` when relevant.
- Tests or query evidence showing app-scoped payment access.

## Output

```text
Payment identity verdict: PASS / FAIL / NOT APPLICABLE

Findings:
- ...

Evidence:
- files:
- tests:
```
