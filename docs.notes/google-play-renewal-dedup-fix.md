# Google Play Renewal Dedup Fix

Date: 2026-05-16

## Summary

Google Play test subscriptions renew multiple times with the same purchase token and the same event type. Bridge currently treats those renewal events as duplicates because of the secondary webhook deduplication rule on `(app_id, provider, purchase_token, event_type)`.

This drops valid `SUBSCRIPTION_RENEWED` events after the first renewal. The app receives only the first renewal, then the final cancellation/expiration events.

## Observed Test Run

Test setup:

- Provider: `google_play`
- App: `hiha`
- Product/subscription id: `hiha_monthly`
- User: `user_36lLgcNtpsqKzB5hpan8wYIN5ew`
- Google Play monthly test renewal cadence: about 5 minutes

Observed external provider behavior:

- Gmail inbox received 7 monthly subscription receipts.
- Google Play Console Orders showed 7 orders for the day.
- After the renewal sequence, Google sent a final subscription cancellation email.

Bridge/HiHa observed behavior:

- Bridge processed the initial verify-purchase activation.
- Bridge processed only the first renewal.
- Bridge discarded renewals 2 through 6 as duplicate/recovered webhooks.
- Bridge processed final cancellation and expiration.
- HiHa received only:
  - `subscription.activated` from verify purchase
  - `subscription.activated` from the first renewal
  - `subscription.cancelled`
  - `subscription.expired`

## Database Evidence

Actual `pay.webhook_provider` rows after the run:

- `SUBSCRIPTION_PURCHASED`
  - Suppressed with `suppressed_reason = unresolved_external_user_id`
- `verify_purchase.succeeded`
- `SUBSCRIPTION_RENEWED`
  - Only the first renewal was stored.
- `SUBSCRIPTION_CANCELED`
- `SUBSCRIPTION_EXPIRED`

Missing renewal webhook ids from logs were not inserted into `pay.webhook_provider`:

- `19071854013335023`
- `19509230033292529`
- `19509038880510487`
- `19539560238624322`
- `19071831317818765`

Current final subscription state:

- `pay.subscriptions.status = expired`
- `pay.subscriptions.google_subscription_state = 6`
- `pay.subscriptions.version = 5`
- `pay.subscriptions.current_period_end = NULL`

## Log Evidence

First renewal was processed:

```text
2026-05-16T17:30:40.2495012+03:00 INFO bridge::webhooks::ingress: Google Play webhook processed: 19072370492819658
```

Later renewals were incorrectly classified as already recovered:

```text
2026-05-16T17:35:38.9704632+03:00 INFO bridge::webhooks::ingress: Duplicate Google Play webhook already recovered: 19071854013335023
2026-05-16T17:40:39.197588+03:00 INFO bridge::webhooks::ingress: Duplicate Google Play webhook already recovered: 19509230033292529
2026-05-16T17:45:38.6880525+03:00 INFO bridge::webhooks::ingress: Duplicate Google Play webhook already recovered: 19509038880510487
2026-05-16T17:50:38.2887016+03:00 INFO bridge::webhooks::ingress: Duplicate Google Play webhook already recovered: 19539560238624322
2026-05-16T17:55:38.8523557+03:00 INFO bridge::webhooks::ingress: Duplicate Google Play webhook already recovered: 19071831317818765
```

Final lifecycle events were processed because they used different event types:

```text
2026-05-16T18:00:36.6846153+03:00 INFO bridge::webhooks::ingress: Google Play webhook processed: 19070511399571227
2026-05-16T18:00:37.7704378+03:00 INFO bridge::webhooks::ingress: Google Play webhook processed: 19070686141265294
```

## Root Cause

The migration `migrations/12_add_webhook_provider_secondary_dedup.sql` creates this unique index:

```sql
CREATE UNIQUE INDEX IF NOT EXISTS idx_webhook_provider_token_event_dedup
ON webhook_provider(app_id, provider, purchase_token, event_type)
WHERE purchase_token IS NOT NULL;
```

The same secondary dedup logic is also reflected in `src/db/webhooks.rs` when fetching an existing webhook after `ON CONFLICT DO NOTHING`:

```sql
provider_webhook_id = $3
OR ($4::TEXT IS NOT NULL AND purchase_token = $4 AND event_type = $5)
```

This is invalid for Google Play subscription renewals:

- Google Play reuses the same purchase token throughout a subscription lifecycle.
- Each renewal event has event type `SUBSCRIPTION_RENEWED`.
- Therefore every renewal after the first conflicts with the first renewal row.

The actual idempotency key for Google Play Pub/Sub delivery should be the provider webhook/message id, not `(purchase_token, event_type)`.

## Required Fixes

1. Remove or narrow `idx_webhook_provider_token_event_dedup`.

   The safest immediate fix is to drop this index and rely on `(app_id, provider, provider_webhook_id)` for webhook idempotency.

   ```sql
   DROP INDEX IF EXISTS pay.idx_webhook_provider_token_event_dedup;
   ```

2. Remove the fallback duplicate lookup by `(purchase_token, event_type)` from `create_webhook_provider`.

   After insert conflict, fetch the existing row only by:

   ```sql
   app_id = $1
   AND provider = $2
   AND provider_webhook_id = $3
   ```

3. Add a regression test for repeated Google Play renewals.

   The test should insert/process multiple `SUBSCRIPTION_RENEWED` events with:

   - same `app_id`
   - same `provider = google_play`
   - same `purchase_token`
   - same `event_type = SUBSCRIPTION_RENEWED`
   - different `provider_webhook_id`
   - increasing `eventTimeMillis`

   Expected result:

   - each renewal gets a distinct `pay.webhook_provider` row
   - each renewal is processed
   - each renewal can be forwarded to the app

4. Keep stale event suppression based on provider event time.

   Deduplication and stale suppression are different concerns:

   - Duplicate delivery: same provider webhook/message id.
   - Stale lifecycle event: older `timestamp_epoch_ms` than `pay.subscriptions.last_event_time`.

   Do not use `(purchase_token, event_type)` as a duplicate proxy for renewable lifecycle events.

5. Recheck `current_period_end` handling for Google Play v2.

   During this run, Google Play API logs showed:

   ```text
   GooglePlay subscription retrieved: state: Some("SUBSCRIPTION_STATE_ACTIVE"), expiry: None
   ```

   This left renewal callbacks with `current_period_end = NULL`, and HiHa did not extend `premium_expires_at` after the first renewal.

   After fixing dedup, verify whether Bridge should read expiry from Google Play v2 `lineItems[].expiryTime` instead of a top-level expiry field.

## Related Issue: Renewal Expiry Is Not Propagated

The renewal dedup fix is necessary, but it is not enough to fix `hiha.users.premium_expires_at`.

Observed `hiha.users` state after the full lifecycle:

- `is_premium = false`
- `premium_activated_at = 2026-05-16T14:25:44.100Z`
- `premium_expires_at = 2026-05-16T14:30:34.854Z`
- `last_bridge_event_ms = 1778943638046`

This means HiHa correctly applied the terminal expiration event and removed premium access, but the stored expiry was never extended past the initial verify-purchase period.

Observed `hiha.webhook_callbacks`:

- `verify_purchase.succeeded` callback had `current_period_end = 2026-05-16T14:30:34.854Z`
- first `SUBSCRIPTION_RENEWED` callback had `current_period_end = NULL`
- final `SUBSCRIPTION_CANCELED` callback had `current_period_end = NULL`
- final `SUBSCRIPTION_EXPIRED` callback had `current_period_end = NULL`

The initial verify-purchase path already handles Google Play v2 expiry correctly by checking:

```rust
purchase
    .line_items
    .first()
    .and_then(|line_item| line_item.expiry_time.as_deref())
    .or(purchase.expiry_time.as_deref())
```

The webhook enrichment path currently checks only the top-level v2 field:

```rust
if fields.current_period_end.is_none() {
    fields.current_period_end = resource.expiry_time.clone();
}
```

Google Play v2 subscription expiry is commonly present on `lineItems[].expiryTime`, not top-level `expiryTime`. This matches the run logs:

```text
GooglePlay subscription retrieved: state: Some("SUBSCRIPTION_STATE_ACTIVE"), expiry: None
```

So, after fixing renewal deduplication, Bridge would process all renewal events, but renewal callbacks could still carry `current_period_end = NULL` until the webhook enrichment path is changed to use `line_items[0].expiry_time` before falling back to `resource.expiry_time`.

Required additional fix:

```rust
if fields.current_period_end.is_none() {
    fields.current_period_end = resource
        .line_items
        .first()
        .and_then(|line_item| line_item.expiry_time.clone())
        .or_else(|| resource.expiry_time.clone());
}
```

Acceptance criteria for this related fix:

- Each processed Google Play renewal updates `pay.subscriptions.current_period_end`.
- Each forwarded renewal callback includes non-null `current_period_end` when Google Play v2 has `lineItems[].expiryTime`.
- HiHa advances `hiha.users.premium_expires_at` on every renewal.
- Final `SUBSCRIPTION_EXPIRED` still sets `hiha.users.is_premium = false`.

## Related Issue: Renewals Do Not Create Payment Rows

`pay.payments` currently contains only one row for the full Google Play test lifecycle, even though Google Play Orders showed 7 orders and Gmail received 7 receipts.

Observed `pay.payments` row:

- `provider = google_play`
- `provider_transaction_id = <purchase_token>`
- `subscription_id = hiha_monthly`
- `product_id = hiha_monthly`
- `amount_cents = 549`
- `status = success`
- `created_at = 2026-05-16T14:25:42.951Z`
- `webhook_received_at = 2026-05-16T14:30:39.906Z`

The table has this unique index:

```sql
CREATE UNIQUE INDEX uq_pay_app_provider_txnid
ON pay.payments(app_id, provider, provider_transaction_id);
```

This unique index is correct if `provider_transaction_id` is a true provider transaction/order id. The current Google Play subscription webhook path uses the purchase token as the transaction id:

```rust
provider_transaction_id: p.pointer("/subscriptionNotification/purchaseToken")
```

Because the purchase token is reused across the subscription lifecycle, every renewal updates the same `pay.payments` row instead of creating one payment row per charge/order.

Required additional fix:

- For Google Play subscription payments, use a per-order identifier, not the purchase token, as `provider_transaction_id`.
- Prefer Google Play v2 `latestOrderId` from `SubscriptionPurchaseV2` when enriching webhook fields.
- Keep `purchase_token` on `pay.subscriptions.purchase_token`; do not use it as the recurring payment transaction id.
- If a renewal cannot be enriched with `latestOrderId`, choose an explicit fallback policy:
  - either skip creating a payment row and log a warning, or
  - use a synthetic id based on provider webhook id, such as `google_play_rtdn:<message_id>`, while marking that it is not a Play order id.

The preferred behavior for audit and reconciliation is one `pay.payments` row per Google Play order/charge:

- initial order
- renewal 1
- renewal 2
- renewal 3
- renewal 4
- renewal 5
- renewal 6

Acceptance criteria for this related fix:

- A full Google Play monthly test lifecycle with 7 Play Console orders creates 7 `pay.payments` rows.
- Each row has a distinct `provider_transaction_id`.
- Renewal rows do not overwrite the initial purchase row.
- `amount_cents` is populated when available from Google Play order or recurring price data.
- Fraud protection still prevents the same provider order id from being claimed by a different `external_user_id`.

## Related Issue: Subscription Row Loses Known Nullable Fields

`pay.subscriptions` correctly has only one row for this lifecycle. That part is expected: a subscription lifecycle is represented by one row keyed by app/user/subscription/provider and by purchase token.

Many nullable columns are expected to be `NULL` for a simple Google Play monthly auto-renewing subscription:

- `payment_state`: legacy/basic payment-state field; Google Play v2 uses `subscriptionState`.
- `cancel_reason`: Google Play v2 uses `canceledStateContext`, not this numeric field.
- `provider_customer_id`: Google Play does not provide a Stripe/Creem-style customer id.
- `revocation_reason`, `revoked_at`: only set for revocation/refund flows.
- grace-period, pause, deferred, prepaid, price-step-up, committed-payment fields: only set for those specific lifecycle scenarios.
- Apple columns: expected `NULL` for Google Play.

However, this run shows several fields that are likely bugs or incomplete enrichment:

### `current_period_end = NULL`

Covered by the renewal expiry issue above. This should be populated from Google Play v2 `lineItems[].expiryTime` when available.

### `google_obfuscated_account_id = NULL`

This may be a bug depending on what the Android client sent in the Billing purchase flow.

Bridge's verify-purchase path can store `google_obfuscated_account_id`, but only if Google Play returns `externalAccountIdentifiers.obfuscatedAccountId`.

If the Android client passed `setObfuscatedAccountId(...)` during purchase, this should usually be present. If it was not passed, `NULL` is expected.

Required check:

- Confirm Android purchase launch sets obfuscated account id.
- Enable/inspect raw Google Play v2 API response for `externalAccountIdentifiers`.
- If present in API response but missing in DB, fix verify-purchase commit or webhook enrichment.

### `google_linked_purchase_token = NULL`

Expected for a first purchase with no upgrade/downgrade/resubscribe link. This is not suspicious for this run.

### `google_subscription_state = 6`

This is correct for the final state:

- `6 = SUBSCRIPTION_STATE_EXPIRED`

During active renewals, this should have been `0`, but the final expiration event correctly overwrote it.

### `auto_renewing = false`

Correct for the final expired/cancelled state.

### `google_cancellation_context = system_initiated`

Likely correct for Google Play test subscriptions after the accelerated renewal limit, where Google stops/cancels the test lifecycle automatically.

### Nullable Field Preservation Bug

`upsert_subscription_tx` updates an existing subscription found by `purchase_token` using direct assignment:

```sql
SET status = $1,
    current_period_end = $2,
    auto_renewing = $3,
    payment_state = $4,
    provider_customer_id = $5
```

This can overwrite previously known values with `NULL` when a later webhook lacks that field. In this run, the first renewal had `current_period_end = NULL`, and the subscription row lost the initial expiry from verify purchase.

Required fix:

- Preserve existing nullable values when incoming webhook data is absent.
- Use `COALESCE` for fields where missing data should not erase known state:

```sql
current_period_end = COALESCE($2, current_period_end),
auto_renewing = COALESCE($3, auto_renewing),
payment_state = COALESCE($4, payment_state),
provider_customer_id = COALESCE($5, provider_customer_id)
```

For terminal events, explicit false/status changes should still apply:

- `auto_renewing = false` on cancelled/expired/revoked should be preserved because false is not `NULL`.
- `status` and `last_event_time` should still update from valid newer events.

Acceptance criteria for this related fix:

- A renewal webhook with missing expiry must not erase an existing `current_period_end`.
- A later enriched renewal with expiry must advance `current_period_end`.
- Terminal events still update status and `auto_renewing = false`.
- Existing Google account/link metadata must not be erased by lifecycle webhooks that do not contain those fields.

## Acceptance Criteria

- A full monthly Google Play test lifecycle with 7 orders results in:
  - initial activation
  - 6 processed/stored renewal webhooks
  - final cancellation/expiration events
- `pay.webhook_provider` contains all distinct Google Pub/Sub message ids.
- Bridge logs no longer say `Duplicate Google Play webhook already recovered` for legitimate renewals with new message ids.
- HiHa receives all renewal callbacks or the intentional canonical callback for each renewal.
- `pay.subscriptions.current_period_end` and `hiha.users.premium_expires_at` advance on renewal when Google provides expiry data.

## Files To Change

- `migrations/12_add_webhook_provider_secondary_dedup.sql`
- add a new migration to drop or replace `idx_webhook_provider_token_event_dedup`
- `src/db/webhooks.rs`
- tests around Google Play webhook ingestion/renewals
