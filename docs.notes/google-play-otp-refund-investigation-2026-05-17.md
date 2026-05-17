# Google Play OTP Refund Investigation - 2026-05-17

## Context

Manual test against app `hiha` / app id `43bd7125-87eb-4136-9605-6c5e524f1ab0`.

Tester purchased Google Play one-time product `hiha_one_time`, then refunded it.

Relevant user:

- `external_user_id`: `user_36lLgcNtpsqKzB5hpan8wYIN5ew`
- purchase token hash in logs: `9f02926cd296`
- Google order id: `GPA.3346-7932-0960-90782`

This document records findings only. No code changes were made during the investigation.

## Timeline

### 1. Purchase RTDN Arrived Before `verify_purchase`

Bridge received:

- time: `2026-05-17T18:52:26+03:00`
- provider webhook id: `19082919261635860`
- event: `ONE_TIME_PRODUCT_PURCHASED`
- product: `hiha_one_time`
- subscription id: missing
- token hash: `9f02926cd296`

Bridge then attempted to resolve `external_user_id`.

Resolution failed:

- subscription token lookup missed
- payment token lookup missed
- Google subscription API lookup returned `410 Gone`
- metadata was missing

Bridge suppressed the webhook:

- `processed = false`
- `suppressed = true`
- `suppressed_reason = unresolved_external_user_id`

This is expected if Google sends the purchase RTDN before the app calls Bridge `verify_purchase`, because Bridge has not yet bound the purchase token to a user.

### 2. `verify_purchase` Completed Successfully

Bridge received `verify_purchase` at `2026-05-17T18:52:30+03:00`:

- provider: `google_play`
- product type: `inapp`
- product id: `hiha_one_time`
- user: `user_36lLgcNtpsqKzB5hpan8wYIN5ew`

Google product API returned:

- `purchaseState = 0`
- `consumptionState = 0`
- `acknowledgementState = 0`
- `orderId = GPA.3346-7932-0960-90782`
- `regionCode = RO`

Bridge acknowledged the purchase successfully, recorded the payment, and forwarded callback:

- Bridge event type to Hiha: `purchase.one_time`
- status: success/completed
- amount: `2599`
- provider webhook id: `verify-purchase-e2dc1a99-6b30-41db-81b1-fbcee98a0d9c`

Hiha granted lifetime premium.

### 3. Refund Produced Two Google Notifications

Bridge received two refund-related RTDNs close together.

First:

- time: `2026-05-17T18:54:54+03:00`
- provider webhook id: `19519288543863435`
- raw event: `ONE_TIME_PRODUCT_REFUNDED`
- canonical event: `purchase.one_time_refunded`
- callback event sent to Hiha: `purchase.one_time`
- callback status: `refunded`

Second:

- time: `2026-05-17T18:54:54+03:00`
- provider webhook id: `19545006170252135`
- raw event: `VOIDED_PURCHASE`
- canonical event: `payment.refunded`
- callback event sent to Hiha: `payment.refunded`
- callback status: `refunded`
- payload had `voidedPurchaseNotification.productType = 2`
- payload had `voidedPurchaseNotification.orderId = GPA.3346-7932-0960-90782`

Hiha received both callbacks:

- `purchase.one_time` with `status=refunded`: OTP premium revoked.
- `payment.refunded`: generic subscription-ended path also marked user non-premium.

## Database Findings

### `pay.webhook_provider`

Relevant rows:

| provider_webhook_id | event_type | processed | suppressed | suppressed_reason |
|---|---|---:|---:|---|
| `19082919261635860` | `ONE_TIME_PRODUCT_PURCHASED` | false | true | `unresolved_external_user_id` |
| `verify-purchase-e2dc1a99-6b30-41db-81b1-fbcee98a0d9c` | `verify_purchase.succeeded` | false | false | null |
| `19519288543863435` | `ONE_TIME_PRODUCT_REFUNDED` | true | false | null |
| `19545006170252135` | `VOIDED_PURCHASE` | true | false | null |

Delivery rows:

- verify purchase callback forwarded once, HTTP 200
- `ONE_TIME_PRODUCT_REFUNDED` forwarded once, HTTP 200
- `VOIDED_PURCHASE` forwarded once, HTTP 200
- suppressed initial purchase RTDN had no delivery row

### `pay.payments`

Two relevant payment rows exist.

Correct verified purchase row:

- `provider_transaction_id = GPA.3346-7932-0960-90782`
- `provider_purchase_token = <Google purchase token>`
- `product_id = hiha_one_time`
- `amount_cents = 2599`
- `currency = USD`
- final `status = refunded`
- `acknowledged_at` populated

Questionable duplicate refund row:

- `provider_transaction_id = <Google purchase token>`
- `provider_purchase_token = null`
- `product_id = hiha_one_time`
- `amount_cents = 0`
- `currency = USD`
- `status = refunded`

This duplicate exists because one OTP refund path records the purchase token as `provider_transaction_id`, while the verified purchase uses Google order id as `provider_transaction_id` and stores the token in `provider_purchase_token`.

This conflicts with the invariant in `INVARIANTS.md`:

> `payments.provider_transaction_id` is the provider's economic transaction/order id. Google Play purchase tokens are lifecycle/API handles and must use dedicated token fields such as `payments.provider_purchase_token` or `subscriptions.purchase_token`.

### `pay.subscriptions`

No subscription row exists for this user/product, which is expected for a one-time product.

## Code Paths Implicated

### OTP Purchase Suppression

`src/webhooks/processor.rs`

`resolve_user` prefers Google Play purchase-token lookups. For this initial RTDN, there was no existing payment token mapping yet, so resolution failed.

It then attempted the Google subscription API as an obfuscated-account-id fallback. That is subscription-oriented and does not fit OTP tokens.

### Noisy `410 Gone`

Two generic Google Play paths can call `get_subscription` for OTP/refund tokens:

- `src/webhooks/processor.rs` around Google Play field enrichment.
- `src/webhooks/processor.rs` inside `resolve_user` obfuscated-account-id fallback.

For OTP tokens, especially after refund, Google returns:

- HTTP `410 Gone`
- reason: `purchaseTokenNoLongerValid`

This appears noisy but not fatal. The flow still completed.

### `VOIDED_PURCHASE` Made to Look Subscription-Like

`src/webhooks/ingress.rs`

For voided purchase notifications, ingress fills `subscription_id` from either:

- `subscriptions.purchase_token`, or
- `payments.product_id`

For this OTP refund, it set:

- `subscription_id = hiha_one_time`

That is a product id, not a subscription id. Downstream this makes the OTP voided purchase look somewhat subscription-like.

### Double Refund Semantics

`src/webhooks/processor/normalize.rs`

Mappings:

- `ONE_TIME_PRODUCT_REFUNDED` -> `purchase.one_time_refunded`
- `VOIDED_PURCHASE` -> `payment.refunded`

For one OTP refund, Google emitted both events, so Bridge forwarded both:

- app callback `purchase.one_time` with `status=refunded`
- app callback `payment.refunded`

This is probably too much semantic duplication for app consumers unless explicitly intended.

### Duplicate Payment Row

`src/services/google_play/product_lifecycle.rs`

The OTP refund handler records a payment using the token as transaction id:

- `provider_transaction_id = purchase_token`
- no `provider_purchase_token`

The verified purchase row used:

- `provider_transaction_id = GPA order id`
- `provider_purchase_token = purchase_token`

Because `pay.payments` uniqueness is on `(app_id, provider, provider_transaction_id)`, this creates a second payment row instead of updating the existing row.

`src/db/payments.rs` has helper queries that can find/update by either `provider_transaction_id` or `provider_purchase_token`, but the OTP refund record path still inserts by token as transaction id.

## Current Behavioral Assessment

The user-facing result was mostly correct:

- Purchase granted lifetime premium.
- Refund removed premium.
- All outbound callbacks delivered HTTP 200.

But Bridge has three issues worth addressing later:

1. OTP/refund RTDNs go through subscription-oriented Google API fallback/enrichment, causing expected but noisy `410 Gone` errors.
2. A single Google OTP refund can produce two app callbacks with refund semantics.
3. OTP refund handling can create a duplicate `pay.payments` row keyed by purchase token, violating the payment transaction id invariant.

## Suggested Follow-Up

Likely fix direction for a later session:

- Treat Google `oneTimeProductNotification` and `voidedPurchaseNotification.productType = 2` as OTP-specific before subscription enrichment.
- Avoid calling `get_subscription` for OTP events/tokens.
- For `VOIDED_PURCHASE` with `productType = 2`, do not populate `subscription_id` with `product_id`, or at least do not let that drive subscription-like handling.
- Decide whether app callbacks should receive only one OTP refund callback. Preferred candidate: `purchase.one_time` with `status=refunded`.
- Update OTP refund payment handling to update by `provider_purchase_token` when possible instead of inserting a second row with token as `provider_transaction_id`.

## Useful Queries From Investigation

```sql
select
  id,
  provider_webhook_id,
  event_type,
  subscription_id,
  left(purchase_token, 12) as token_prefix,
  processed,
  suppressed,
  suppressed_reason,
  timestamp_epoch_ms,
  created_at,
  payload
from pay.webhook_provider
where app_id = '43bd7125-87eb-4136-9605-6c5e524f1ab0'
  and provider = 'google_play'
  and provider_webhook_id in (
    '19082919261635860',
    '19519288543863435',
    '19545006170252135'
  )
order by created_at desc;
```

```sql
select
  id,
  external_user_id,
  provider,
  provider_transaction_id,
  subscription_id,
  product_id,
  amount_cents,
  currency,
  status,
  acknowledged_at,
  webhook_received_at,
  created_at,
  left(provider_purchase_token, 12) as token_prefix
from pay.payments
where app_id = '43bd7125-87eb-4136-9605-6c5e524f1ab0'
  and (
    external_user_id = 'user_36lLgcNtpsqKzB5hpan8wYIN5ew'
    or product_id = 'hiha_one_time'
    or subscription_id = 'hiha_one_time'
  )
order by created_at desc
limit 20;
```

```sql
select
  wp.provider_webhook_id,
  wp.event_type,
  wp.processed,
  wp.suppressed,
  wd.id as delivery_id,
  wd.forward_attempts,
  wd.forwarded,
  wd.forwarded_at,
  wd.last_http_status,
  wd.last_error,
  wd.dead_lettered
from pay.webhook_provider wp
left join pay.webhook_delivery wd on wd.webhook_provider_id = wp.id
where wp.app_id = '43bd7125-87eb-4136-9605-6c5e524f1ab0'
  and wp.provider_webhook_id in (
    '19082919261635860',
    '19519288543863435',
    '19545006170252135',
    'verify-purchase-e2dc1a99-6b30-41db-81b1-fbcee98a0d9c'
  )
order by wp.created_at;
```
