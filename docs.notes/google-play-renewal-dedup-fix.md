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

