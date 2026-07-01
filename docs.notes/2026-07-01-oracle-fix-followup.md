# Bridge — Oracle Follow-up Fix Plan

**Date:** 2026-07-01  
**Source:** Oracle review of the current security-review fix diff  
**Verdict:** Glass is broken, but narrowly. Most security fixes look correct; two behavior regressions should be fixed before shipping.

---

## P0. Preserve Google one-time purchase identity when price is unknown

### Problem

The current fix for missing amount/currency avoids fake money by skipping payment rows when `amount_cents < 0` or `currency = 'UNKNOWN'`. That removes fake paid records, but it also drops the durable identity row for Google one-time purchases and verify-purchase flows.

### Impact

- A Google OTP webhook can forward a completed callback but create no `pay.payments` row.
- The Google acknowledgement worker only scans `pay.payments`, so the purchase may never be acknowledged.
- Later refund/cancel/user-token resolution by purchase token can fail because the token was never stored.

### Evidence

- `src/services/google_play/product_lifecycle.rs` records missing price as `amount_cents: -1` and `currency: "UNKNOWN"` while still setting Google ack fields.
- `src/application/verify_purchase.rs` has the same missing-price sentinel path.
- `src/db/payments.rs` now skips rows for negative amount or `UNKNOWN` currency.
- `find_google_product_purchases_needing_ack` only finds rows in `pay.payments`.

### Required fix

Keep a payment identity row for successful Google product purchases that need durable purchase-token/product-id/ack tracking, even when price is unknown.

Preferred model:

- Make `pay.payments.amount_cents` and `currency` nullable, or add an explicit `price_known` / `price_unknown` model.
- Store the row with purchase token, product id, provider, status, and `ack_required`.
- Do not pretend unknown price is real `0`, `N/A`, or any other money value.

Emergency narrow fallback:

- Do not skip rows when `ack_required = true` or when the row carries Google product identity needed for acknowledgement/refund resolution.
- Only skip truly money-only audit rows where no durable purchase identity is needed.

### Regression test to add

Add a test equivalent to:

```bat
cmd /c "cargo test google_one_time_purchase_missing_amount_still_records_payment_for_ack --lib 2>&1 && echo EXIT: %ERRORLEVEL%"
```

It should assert that a Google one-time purchase with missing amount/currency still creates a `pay.payments` row with:

- `provider_purchase_token`
- `product_id`
- `ack_required = true`
- no fake real-money amount/currency

---

## P1. Guard `resume_subscription` before provider side effects

### Problem

The DB update now only resumes subscriptions whose status is `paused` or `cancelled`, but the application layer calls the provider before hitting that DB guard.

### Impact

Calling resume on an already-active/trial/past-due subscription can make a provider API call, then fail locally with no DB row updated. This is confusing API behavior and an unnecessary external side effect.

### Evidence

- `src/application/subscription_actions.rs` rejects only terminal `revoked` / `expired` before calling the provider.
- `src/db/subscriptions.rs` now updates only `status IN ('paused', 'cancelled')`.

### Required fix

Before any provider call, require:

```text
status == "paused" || status == "cancelled"
```

Return a client-visible conflict, not a DB-derived 500-ish error, for any other status. Keep the DB `WHERE status IN ('paused', 'cancelled')` as defense-in-depth.

### Regression test to add

Add or update a resume test that asserts:

- `active` / `trialing` / `past_due` subscriptions return conflict.
- No provider resume call is made for invalid source states.
- `paused` and `cancelled` still follow the existing successful resume path.

---

## Deployment / verification notes

Run the focused checks after the two fixes:

```bat
cmd /c "cargo check 2>&1 && echo EXIT: %ERRORLEVEL%"
cmd /c "cargo test production_startup_rejects --lib 2>&1 && echo EXIT: %ERRORLEVEL%"
cmd /c "cargo test google_play_webhook_signature_verification_cannot_be_bypassed_outside_mock_mode --lib 2>&1 && echo EXIT: %ERRORLEVEL%"
cmd /c "cargo test google_play_lifecycle --lib 2>&1 && echo EXIT: %ERRORLEVEL%"
cmd /c "cargo test atomic_processor_rejects_mismatched_claim_token --lib 2>&1 && echo EXIT: %ERRORLEVEL%"
```

Also confirm production env is ready for the intentionally stricter Google webhook config:

- `GOOGLE_VERIFY_AUDIENCE=true`
- non-empty `GOOGLE_PUB_SUB_AUDIENCE`
- `GOOGLE_SKIP_RSA_VERIFICATION` is not `true`

---

## Non-blocking follow-up

The migration `99_alter_payments_amount_cents_bigint.sql` means deployments must run migrations before code that reads/writes `payments.amount_cents` as `i64`.
