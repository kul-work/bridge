# Google Play Initial Payment Identity - Oracle Recommendation

Date: 2026-05-16

## Context

After the Google Play renewal fixes, `pay.payments` still has one inconsistency for the initial subscription payment created by `verify_purchase`:

- The initial row uses the Google purchase token as `provider_transaction_id`.
- The initial row has `product_id = NULL`.
- Renewal rows created from Google Play webhooks use Google order ids such as `GPA...0`, `GPA...1`, `GPA...2`.
- Renewal rows have `product_id = hiha_monthly`.

The first instinct was to make `verify_purchase` use `SubscriptionPurchaseV2.latestOrderId` as `provider_transaction_id`, matching renewal rows.

Oracle's recommendation is **not to do that as the small safe fix**.

## Why Not Switch Initial Verify-Purchase Rows to `latestOrderId` Now

In the current schema, Google purchase tokens are not stored as a dedicated column in `pay.payments`. Instead, several code paths currently treat `payments.provider_transaction_id` as the Google purchase-token lookup key for the initial subscription payment.

Changing only the verify-purchase initial row to `latestOrderId` would make the row cleaner for reporting, but it would break or weaken token-based behavior in multiple paths:

- `verify_purchase` acknowledgement checks and marks payments by purchase token.
- Google webhook acknowledgement helpers check and mark payments by purchase token.
- The acknowledgement retry scheduler joins `subscriptions.purchase_token` to `payments.provider_transaction_id`.
- Manual subscription acknowledgement updates payments by `provider_transaction_id = purchase_token` when a token exists.
- Refund/void and suppression paths still look up or update payment status by purchase token.

There is also a subtle behavior risk: re-verifying the same subscription token after a renewal could see a newer `latestOrderId` and create a new payment row from `verify_purchase`, even though verify-purchase is supposed to attach the current user/subscription lifecycle, not create renewal economic events.

## Smallest Safe Fix

Keep the initial Google subscription payment row token-keyed for now, and make that explicit.

Recommended invariant:

- Initial Google subscription payment row: **purchase-token keyed**, used as the ackable initial purchase row.
- Renewal rows: **Google order-id keyed**, used as economic renewal event rows.

Implementation outline:

1. In `src/ports/impls/payment.rs::commit_verified_purchase`, set `product_id` for subscription payments.
   - Current subscription behavior: `product_id = None`.
   - Recommended behavior: `product_id = Some(subscription_id)`.
   - This fixes the observed `NULL product_id` without changing ack/idempotency assumptions.

2. Keep verify-purchase Google subscription `provider_transaction_id = purchase_token`.
   - Do not plumb `SubscriptionPurchaseV2.latestOrderId` into initial verify-purchase payment recording as part of this small fix.

3. Make the initial Google `SUBSCRIPTION_PURCHASED` webhook record/update the same token-keyed payment row.
   - Special-case the raw Google event type `SUBSCRIPTION_PURCHASED`.
   - For that initial event, record `provider_transaction_id = purchase_token`.
   - Keep renewal events using enriched Google order ids.

4. Leave acknowledgement logic unchanged for this small fix.
   - Google API acknowledgement still uses purchase token.
   - Payment ack lookup/mark still uses the initial token-keyed payment row.
   - The retry scheduler and manual acknowledgement behavior continue to work.

5. Add focused regression tests.
   - Verify-purchase Google subscription records non-null `product_id`.
   - Verify-purchase plus initial `SUBSCRIPTION_PURCHASED` webhook upsert the same payment row.
   - Renewal webhook still records a separate GPA order-id row.

## Longer-Term Clean Fix

If Bridge needs all Google subscription payment rows to use order ids consistently, do it as a schema-backed change, not a hotfix.

The clean model would be:

- Keep `payments.provider_transaction_id` as the Google order id when available.
- Add a dedicated Google purchase-token linkage to `pay.payments` or another mapping table.
- Move token-based lookups off `provider_transaction_id`.
- Keep legacy fallback for old rows where `provider_transaction_id` is the purchase token.

That larger change would need to update acknowledgement, retry scheduling, refund/void handling, webhook suppression helpers, and lookup helpers together.

## Decision

For the immediate fix, prefer the small safe path:

- Fix `product_id` for verify-purchase subscription payments.
- Keep the initial Google subscription payment token-keyed.
- Make the initial `SUBSCRIPTION_PURCHASED` webhook dedupe/upsert into the same token-keyed row.
- Keep renewal rows order-keyed.
