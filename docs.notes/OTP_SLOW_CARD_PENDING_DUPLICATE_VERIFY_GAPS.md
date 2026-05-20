# OTP Slow Card Pending Duplicate Verify Gaps

Date: 2026-05-20

## Context

A simulated Google Play one-time product purchase was tested with the slow card pending flow for product `hiha_one_time`.

Observed identifiers:

- Bridge app: `hiha`
- App ID: `43bd7125-87eb-4136-9605-6c5e524f1ab0`
- External user ID: `user_36lLgcNtpsqKzB5hpan8wYIN5ew`
- Google order ID: `GPA.3327-8821-9228-16853`
- Purchase token tail: `ZVfwN98g`
- Google RTDN event ID: `19143201500096844`

## What Happened

Google sent a `ONE_TIME_PRODUCT_PURCHASED` RTDN at `2026-05-20 15:19:14 +03:00`.

Bridge verified the Pub/Sub JWT and stored the webhook, but suppressed processing:

- `processed = false`
- `suppressed = true`
- `suppressed_reason = unresolved_external_user_id`

The Google RTDN payload contained the OTP SKU and purchase token, but did not provide enough user-binding data for Bridge to resolve `external_user_id`. For OTP, Bridge skipped obfuscated-account resolution and had no metadata mapping available.

About 90 seconds later, three `/verify-purchase` requests arrived for the same OTP token and same user:

- `2026-05-20 15:20:43.837 +03:00`
- `2026-05-20 15:20:44.669 +03:00`
- `2026-05-20 15:20:44.686 +03:00`

At verification time, Google reported the purchase as completed:

- `purchaseState = 0`
- `acknowledgementState = 0`
- `orderId = GPA.3327-8821-9228-16853`

Bridge stored only one payment row for the order, but each verify request generated a distinct synthetic `verify_purchase.succeeded` webhook and forwarded a distinct `purchase.one_time` callback to HiHa.

## Confirmed Database Outcome

Bridge `pay.payments` had one row for the Google order:

- `provider_transaction_id = GPA.3327-8821-9228-16853`
- `product_id = hiha_one_time`
- `status = success`
- `amount_cents = 2599`
- `currency = RON`
- `acknowledged_at = 2026-05-20T12:20:46.738Z`

Bridge `pay.webhook_provider` had one suppressed Google RTDN row and three synthetic verify rows:

- `19143201500096844` / `ONE_TIME_PRODUCT_PURCHASED` / suppressed
- `verify-purchase-65223487-0f74-409f-8bfd-b83c74b63df8`
- `verify-purchase-65f1c8ba-a842-4ced-b7bb-f5a9eecc4003`
- `verify-purchase-1e46fea4-8ede-471d-9a4f-3cea685f9668`

Bridge `pay.webhook_delivery` forwarded all three synthetic verify callbacks successfully with HTTP `200`.

HiHa `webhook_callbacks` stored all three callbacks as distinct events.

HiHa `users` ended in the correct entitlement state:

- `is_premium = true`
- `premium_expires_at = null`
- `last_bridge_event_ms = 1779279646913`

## Bridge Gaps

### 1. Verify-purchase is not idempotent by OTP purchase token or order

Bridge correctly deduped the payment record using the Google order identity, but it did not dedupe the side effect of forwarding verify-created callbacks.

Impact:

- Three concurrent verifies for the same OTP token produced three app callbacks.
- Downstream apps saw three `purchase.one_time` events for one Google purchase.
- The payment table stayed correct, but the event stream did not represent one business event.

Expected behavior:

- Repeated successful verification of the same OTP purchase token/order should not emit multiple independent purchase callbacks.
- Bridge should either return the existing verification result without forwarding again, or use a stable provider event identity that downstream systems can dedupe.

### 2. Google acknowledgement is not concurrency-protected

All three verify calls saw `acknowledgementState = 0` and attempted acknowledgement close together.

One acknowledgement succeeded. Another hit Google:

```text
409 Conflict
reason = concurrentUpdate
```

Impact:

- No entitlement corruption occurred.
- The logs showed a provider error even though the purchase was successfully acknowledged by another concurrent verify path.
- Repeated acknowledgement attempts are avoidable provider noise.

Expected behavior:

- Acknowledgement should be guarded by a token/order-level idempotency check or lock.
- If another concurrent request is already acknowledging the token, later requests should wait, skip, or re-read state.

### 3. Synthetic verify callback IDs are random per request

Bridge creates verify callback IDs as `verify-purchase-{uuid}`.

Impact:

- The same purchase token/order can produce multiple unique event IDs.
- HiHa cannot dedupe these callbacks by `event_id`.
- Delivery idempotency only applies to each synthetic event, not to the underlying purchase.

Expected behavior:

- For verify-created OTP purchase callbacks, the provider event identity should include stable purchase identity, such as provider, app, purchase token hash, or Google order ID.
- Alternatively, Bridge should suppress duplicate callback creation when a successful callback for the same OTP token/order already exists.

### 4. OTP RTDN cannot resolve external_user_id on its own

The first Google RTDN was valid, but Bridge suppressed it because `external_user_id` could not be resolved.

Impact:

- OTP fulfillment depends on a later client/server verify call.
- If the client never calls verify after the Google RTDN, Bridge may store a suppressed provider event but not grant access.

Expected behavior:

- This may be acceptable by design, but it is a known limitation.
- If webhook-first OTP fulfillment is required, Bridge needs a reliable token-to-user binding before or during purchase.

## HiHa Gaps

### 1. Callback dedupe only uses event_id

HiHa inserted all three callbacks because each Bridge callback had a unique event ID.

Impact:

- Callback history contains three rows for one OTP purchase.
- HiHa logs showed "OTP purchase completed, lifetime premium granted" three times.
- The final entitlement state was correct, but the processing history was noisy and semantically duplicated.

Expected behavior:

- HiHa should consider semantic idempotency for OTP purchases, such as user plus product plus provider purchase token/order, if Bridge sends enough identity to support that.
- At minimum, logs should distinguish "already granted" from "granted now" when the entitlement state does not materially change.

### 2. Grant logging does not reflect whether state changed

HiHa logged a successful grant for each inserted callback.

Impact:

- Operators may think access was granted three separate times.
- The log does not show that only one final lifetime premium entitlement exists.

Expected behavior:

- HiHa should log the grant only when the user entitlement update actually changes state, or log repeated callbacks as duplicate/no-op.

### 3. Entitlement state is guarded, but callback history is not semantically guarded

HiHa's `last_bridge_event_ms` check prevents older events from overwriting newer state. This protected the final user state.

Impact:

- User entitlement outcome remained correct.
- The callback table still accepted all semantically duplicate purchase callbacks because their event IDs were different.

Expected behavior:

- Event history should either store semantic duplicate markers or enforce a unique constraint/lookup for OTP purchase identity once Bridge includes stable purchase identity in callbacks.

## Summary

The user-facing entitlement outcome was correct: one lifetime premium state was granted for one Google OTP purchase.

The main defect is not triple charging or triple entitlement. The defect is an idempotency gap across repeated OTP verification:

- Bridge dedupes payment persistence, but not verify callback emission.
- Bridge acknowledges Google concurrently under repeated verify calls.
- HiHa dedupes callback rows by event ID only, so random Bridge verify event IDs are all accepted.
- HiHa entitlement state remains correct, but callback history and logs show duplicate grants.

