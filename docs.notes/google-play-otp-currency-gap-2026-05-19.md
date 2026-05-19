# Google Play OTP Currency Gap - 2026-05-19

## Context

Manual Google Play OTP test for app `hiha`:

- Product: `hiha_one_time`
- Google order id: `GPA.3378-0396-0235-84043`
- Observed charge: `RON 25.99`
- Bridge stored payment amount correctly as `2599`
- Bridge stored payment currency as default `USD`

## Raw Google RTDN Evidence

The incoming Google RTDN webhook payloads for OTP purchase/refund did not include amount or currency.

Observed RTDN payloads contained only:

- `packageName`
- `eventTimeMillis`
- `oneTimeProductNotification.purchaseToken`
- `oneTimeProductNotification.sku`
- `oneTimeProductNotification.notificationType`
- for `VOIDED_PURCHASE`: `orderId`, `productType`, `refundType`, `purchaseToken`

No RTDN field provided:

- `currency`
- `currencyCode`
- `priceCurrencyCode`
- `amount`
- `price`

Conclusion: OTP currency cannot be derived from RTDN payloads.

## Bridge Log Evidence

Bridge `verify_purchase` called:

- `purchases.products.get`
- Google Orders API via `get_order_amount_cents`
- `purchases.products.acknowledge`

The `purchases.products.get` raw response included:

- `orderId = GPA.3378-0396-0235-84043`
- `regionCode = RO`

It did not include a currency field.

`regionCode = RO` is useful context, but it is not a currency. It hints at Romania, but does not prove `RON`.

Bridge trace showed:

- `amount_cents = 2599`

No Bridge log showed:

- `currency = RON`
- `currencyCode = RON`

## Current Bridge Behavior

`src/services/google_play/client.rs`

- `get_order_amount_cents(...)` calls Google Orders API.
- It parses `lineItems[].total` into cents.
- It returns only `Option<i32>`.
- It discards any currency code that may exist on the same Google `Money` payload.

`src/application/verify_purchase_provider.rs`

- OTP verification receives only `amount_cents` from the order lookup.
- `currency` remains `None`.

`src/application/verify_purchase.rs`

- Payment commit uses `verified.currency.or(payload.currency)`.
- Because Google OTP verification has no currency and the app did not provide one, DB insert falls back to the payments default.

`src/db/payments.rs`

- `record_payment_with_purchase_token_tx` uses `COALESCE(NULLIF($10, ''), 'USD')`.
- Missing currency becomes `USD`.

## Old HiHa Parity Check

Classification: `BRIDGE-ONLY`

Old HiHa did not correctly handle Google OTP currency either.

Old HiHa evidence:

- `C:\share\hiha\src\services\google_play\provider.rs`
  - Google OTP verification returned `amount_cents: None`.
  - Comment: `INAPP pricing not tracked in this response; stored elsewhere in Play Console`.
- `C:\share\hiha\src\handlers\payments\verify_purchase.rs`
  - Recorded `amount_cents = subscription.amount_cents.unwrap_or(0)`.
  - Passed no currency into payment persistence.
- `C:\share\hiha\src\db\payments.rs`
  - `PaymentParams` had no currency field.
  - `record_payment` inserted no currency.
- `C:\share\hiha\migrations\03_create_payments_table.sql`
  - `payments.currency TEXT DEFAULT 'USD'`.

Conclusion: old HiHa also defaulted Google OTP currency to `USD`; it usually also stored OTP amount as `0`. Bridge improved amount capture, but still needs currency capture.

## Problem

Bridge now has a partial Google OTP payment record:

- amount is provider-derived and correct (`2599`)
- currency is defaulted and wrong (`USD`)

This creates an inconsistent money pair: `USD 25.99` stored for a transaction that was charged as `RON 25.99`.

## Likely Fix Target

Do not infer currency from `regionCode`.

Instead, extend the Google Orders API lookup path:

1. Replace or supplement `get_order_amount_cents(...)` with a helper that returns both:
   - `amount_cents`
   - `currency`
2. Parse the currency from the same Google `Money` object used to parse cents, likely `lineItems[].total.currencyCode`.
3. Pass that currency through OTP verification.
4. Persist it in `pay.payments.currency`.
5. Add a focused test that a Google order payload with `currencyCode = "RON"` produces:
   - `amount_cents = 2599`
   - `currency = "RON"`

## Non-Solutions

- Do not use RTDN payloads for OTP currency; they do not include it.
- Do not map `regionCode = RO` to `RON`; region and currency are distinct and can diverge.
- Do not preserve old HiHa behavior here; old HiHa is not a correct oracle for OTP currency.
