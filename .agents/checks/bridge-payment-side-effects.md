# Bridge Payment Side-Effects Checker

Use after changing payment verification, checkout, webhook processing, callback delivery, subscriptions, or provider normalization.

## Check

Flow success is not enough. Verify tests assert durable facts that the flow writes or emits.

Relevant durable fields include:

- `provider_transaction_id`
- `provider_purchase_token`
- `currency`
- `amount_cents`
- `product_id`
- `subscription_id`
- `status`
- `period_start` / `period_end`
- callback event type
- callback body fields
- webhook dedup key
- `app_id` / `external_user_id` scoping

## Insufficient evidence

- HTTP 200 only.
- "Callback happened" without payload assertions.
- Payment row exists, but identity/currency/amount are not checked.
- Subscription became active, but period/provider fields are not checked.

## Bug-fix guardrail

For every payment/provider bug fix:

1. Reproduce or identify the failing behavior from raw logs/tests.
2. Classify the task as `PARITY`, `BRIDGE-ONLY`, or `UNKNOWN`.
3. If `PARITY`, use the old HiHa oracle first.
4. Patch the smallest code path.
5. Add or adjust an assertion that would have caught the bug, or document why no assertion is practical.

## Output

```text
Side-effect test verdict: PASS / FAIL / NOT APPLICABLE

Missing assertions:
- field:
- why it matters:
- suggested test file:

Evidence:
- files:
- tests:
```
