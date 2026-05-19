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

## Actual Issues

1. OTP/refund events can call Google subscription APIs even though one-time-product tokens are not subscription tokens. This causes expected but noisy `410 Gone` / `purchaseTokenNoLongerValid` errors.
2. `VOIDED_PURCHASE` notifications for one-time products can be made to look subscription-like because ingress may populate `subscription_id` with the product id.
3. A single Google OTP refund can produce two refund-like callbacks to the app: `purchase.one_time` with `status=refunded` and `payment.refunded`.
4. OTP refund handling can create a duplicate `pay.payments` row by using the Google purchase token as `provider_transaction_id`.
5. The duplicate payment row violates the invariant that `payments.provider_transaction_id` stores the provider economic transaction/order id, while Google purchase tokens belong in token fields.
6. If the initial purchase RTDN arrives before `verify_purchase`, Bridge cannot resolve the user because the purchase token has not yet been bound to `external_user_id`.
7. The user-resolution fallback is subscription-oriented for OTP events because it attempts a Google subscription API lookup for obfuscated account id resolution.

## Per-Issue Fix Investigation

Classification used below:

- `PARITY`: old HiHa behavior is the oracle unless a Bridge-only invariant requires divergence.
- `BRIDGE-ONLY`: Bridge intentionally differs because it is multi-app middleware with app-scoped callbacks, ingress logging, and centralized payment records.

### 1. OTP/refund events call Google subscription APIs

Parity claim:

`PARITY`: For Google Play OTP RTDN handling, Bridge must use OTP/product semantics and must not call subscription APIs for one-time-product tokens.

Old HiHa:

- file/function: `C:\share\hiha\src\services\google_play\provider.rs`, `verify_inapp_token`
- exact behavior: one-time products use `client.get_product(...)`, not `get_subscription(...)`.
- relevant docs/tests: `C:\share\hiha\docs\google\GOOGLE_PLAY_BILLING_TESTPLAN.md` OTP-RTDN-01/02 explicitly says OTP webhooks call `purchases.products.get()` and not `get_subscription()`.

Bridge:

- file/function: `src/webhooks/processor.rs`, `enrich_google_play_fields`
- current behavior: if a Google webhook has both a purchase token and a `subscription_id`, Bridge calls `client.get_subscription(package_name, subscription_id, purchase_token)`.
- divergence: one-time-product ingress stores the product id in `subscription_id`, so OTP RTDNs can enter subscription enrichment and trigger `410 Gone` for OTP/refund tokens.

Decision:

- `PARITY`
- reason: old HiHa and Google Play product lifecycle docs treat OTP tokens as product-purchase handles, not subscription handles.
- fix target: guard `enrich_google_play_fields` so it only calls `get_subscription` for real subscription events, or route OTP events to product-specific enrichment.
- assertion to add: OTP `ONE_TIME_PRODUCT_REFUNDED` / OTP `VOIDED_PURCHASE` processing does not call the subscription client.

### 2. OTP `VOIDED_PURCHASE` can be made subscription-like

Parity claim:

`PARITY`: For Google Play OTP `VOIDED_PURCHASE`, Bridge must not treat the product id as a subscription id.

Old HiHa:

- file/function: `C:\share\hiha\src\services\google_play\provider.rs`, voided purchase parsing
- exact behavior: `VOIDED_PURCHASE` becomes `purchase.voided`; `subscription_id` is set to the Google order id only for logging, while lookup/revocation is driven by purchase token.
- relevant handler: `C:\share\hiha\src\webhooks\events\subscription\common.rs`, `handle_purchase_voided`, where OTP is identified by no subscription row for the token.

Bridge:

- file/function: `src/webhooks/ingress.rs`, Google Play ingress
- current behavior: for a voided purchase with no subscription notification, Bridge first looks up a subscription by purchase token, then falls back to payment product id and stores that as `subscription_id`.
- divergence: for OTP, this makes `subscription_id = hiha_one_time`, even though that is a product id and no subscription row exists.

Decision:

- `PARITY`
- reason: old HiHa keeps voided-purchase routing token-driven; Bridge's product-id fallback creates false subscription shape.
- fix target: for `voidedPurchaseNotification.productType` identifying OTP, do not populate `subscription_id` from payment `product_id`; keep product id in fields/payload only.
- assertion to add: OTP voided purchase canonical payload has `product_id = hiha_one_time` and `subscription_id = null`.

### 3. One OTP refund emits two refund-like app callbacks

Parity claim:

`PARITY`: For one Google Play OTP refund, Bridge should emit one app-facing refund semantic, preferably `purchase.one_time` with `status=refunded`.

Old HiHa:

- file/function: `C:\share\hiha\src\services\google_play\provider.rs`, `map_otp_notification_type` and voided purchase parsing
- exact behavior: OTP refund notification type `2` maps to `purchase.voided`; voided purchases also map to `purchase.voided`.
- handler: `C:\share\hiha\src\webhooks\events\subscription\common.rs`, `handle_purchase_voided`, which performs token-based idempotency and skips if payment is already `refunded`.
- relevant test: `C:\share\hiha\docs\google\GOOGLE_PLAY_BILLING_TESTPLAN.md` WHK-05 expects redundant refund notifications for the same token not to trigger duplicate side effects.

Bridge:

- file/function: `src/webhooks/processor/normalize.rs`
- current behavior: `ONE_TIME_PRODUCT_REFUNDED` maps to `purchase.one_time_refunded`; `VOIDED_PURCHASE` maps to `payment.refunded`.
- downstream: `src/webhooks/processor.rs` maps `purchase.one_time_refunded` to app event `purchase.one_time`, while `payment.refunded` remains `payment.refunded`.
- divergence: when Google emits both RTDN variants for one OTP refund, Bridge forwards two different app-facing refund events.

Decision:

- `PARITY`
- reason: old HiHa treats duplicate refund notifications as one token-idempotent refund side effect.
- fix target: for OTP `VOIDED_PURCHASE`, either normalize to `purchase.one_time_refunded` or suppress forwarding when the OTP payment is already refunded by a paired `ONE_TIME_PRODUCT_REFUNDED`.
- assertion to add: processing both OTP refund notifications for the same token results in one outbound app callback.

### 4. OTP refund can create a duplicate payment row

Parity claim:

`BRIDGE-ONLY`: Bridge should preserve its order-id transaction invariant while matching old HiHa's idempotent refund side effect by updating the existing payment row through `provider_purchase_token`.

Old HiHa:

- file/function: `C:\share\hiha\src\db\payments.rs`, `update_payment_status`
- exact behavior: old HiHa keyed Google Play payments by purchase token in `provider_transaction_id`, so OTP refund updates the same row by token.
- handler: `C:\share\hiha\src\webhooks\events\subscription\common.rs`, `handle_purchase_voided`, updates status to `refunded` if a token payment exists.

Bridge:

- file/function: `src/services/google_play/product_lifecycle.rs`, `handle_otp_refunded`
- current behavior: refund handler calls `record_webhook_payment` with `provider_transaction_id = token` and no `provider_purchase_token`.
- database behavior: `src/db/payments.rs` upserts only on `(app_id, provider, provider_transaction_id)`, so a verified purchase row keyed by Google order id is not matched.
- divergence: Bridge creates a second row instead of updating the verified purchase row.

Decision:

- `BRIDGE-ONLY`
- reason: old HiHa used token-as-transaction-id, but Bridge explicitly split economic order id from lifecycle token.
- fix target: OTP refund/cancel paths should call `update_payment_status_for_provider(app_id, provider, token, status)` or a token-aware update helper, not insert a new token-keyed row.
- assertion to add: verified OTP purchase followed by refund leaves exactly one payment row for the token/order and changes that row to `refunded`.

### 5. Payment transaction id invariant violation

Parity claim:

`BRIDGE-ONLY`: Bridge must keep `payments.provider_transaction_id` as the provider economic transaction/order id and store Google Play purchase tokens in `provider_purchase_token`.

Old HiHa:

- file/function: `C:\share\hiha\src\db\payments.rs`
- exact behavior: old HiHa used `provider_transaction_id` as the token lookup key for Google Play.
- limitation: old HiHa did not have Bridge's split `provider_purchase_token` field or multi-app invariant.

Bridge:

- invariant: `INVARIANTS.md` says Google Play purchase tokens are lifecycle/API handles and must use token fields.
- current behavior: OTP refund/cancel handlers can pass the purchase token as `provider_transaction_id`.
- divergence: this violates Bridge's own invariant even if it resembled old HiHa's older schema.

Decision:

- `BRIDGE-ONLY`
- reason: this is an explicit Bridge schema invariant.
- fix target: remove token-as-transaction-id writes from Google OTP webhook lifecycle paths; keep token matching in lookup/update helpers only.
- assertion to add: no Google OTP webhook path writes a non-`GPA.` purchase token into `payments.provider_transaction_id` when an order id exists.

### 6. Initial purchase RTDN cannot resolve user before `verify_purchase`

Parity claim:

`BRIDGE-ONLY`: Bridge can suppress unresolved OTP purchase RTDNs before `verify_purchase`, but it should avoid making that suppression a permanent missed side effect when the app verifies the same token seconds later.

Old HiHa:

- file/function: `C:\share\hiha\src\webhooks\events\otp.rs`, `handle_one_time_product_purchased`
- exact behavior: Google OTP purchase RTDN looks up the user by purchase token in payments; if no row exists, it logs a warning and returns without mutation.
- limitation: old HiHa also cannot bind an RTDN to a user before the app has created a token-to-user payment row.

Bridge:

- file/function: `src/webhooks/processor.rs`, `resolve_user` and `ensure_resolved_user`
- current behavior: token lookup misses, metadata is missing, then the webhook is marked `suppressed = true` with `unresolved_external_user_id`.
- mitigating behavior: `verify_purchase` later verifies, records the payment, acknowledges it, and sends the purchase callback.
- remaining problem: the suppressed RTDN is never resumed after the token mapping exists.

Decision:

- `BRIDGE-ONLY`
- reason: Bridge's ingress logging and suppression model is middleware-specific; old HiHa's behavior still confirms the root limitation.
- fix target: either keep this as documented expected behavior because `verify_purchase` completes the side effect, or add a replay/recovery path that can reprocess previously suppressed OTP purchase RTDNs after `verify_purchase` binds the token.
- assertion to add: purchase RTDN before `verify_purchase` followed by successful `verify_purchase` grants exactly once, and either the suppressed RTDN remains intentionally suppressed or is recovered without duplicate callback.

### 7. OTP user-resolution fallback is subscription-oriented

Parity claim:

`PARITY`: For Google Play OTP events, user-resolution fallback must not call subscription lookup APIs.

Old HiHa:

- file/function: `C:\share\hiha\src\services\google_play\provider.rs`, `verify_inapp_token`
- exact behavior: OTP verification calls product APIs and extracts product-purchase fields. Subscription obfuscated-account lookup is not used for OTP RTDN resolution.
- relevant docs: `C:\share\hiha\docs\google\GOOGLE_PLAY_ONE-TIME_LIFECYCLE-v1.1.md` scopes OTP to `purchases.products`.

Bridge:

- file/function: `src/webhooks/processor.rs`, `resolve_user`
- current behavior: after token lookups miss, Google Play strategy 3 calls `gp_client.get_subscription(pkg, "", token)` for any Google Play token.
- divergence: this fallback is subscription-specific and produces noisy/failing calls for OTP tokens.

Decision:

- `PARITY`
- reason: old HiHa treats OTP and subscription provider APIs separately.
- fix target: make `resolve_user` aware of raw Google event shape/canonical event before attempting obfuscated-account fallback; skip subscription fallback for `oneTimeProductNotification` and OTP `voidedPurchaseNotification`.
- assertion to add: unresolved OTP RTDN records failure summary without `obfuscated_account_id=api_error` from a subscription API call.

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
