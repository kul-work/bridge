# Google Play Billing: Deferred Features

## Overview

These features require additional setup in Google Play Console, product configuration, or are not needed for the initial production release. They are deferred.

### Section A: One-Time Purchases

#### 1. Multi-Item Cart Support
Allow users to purchase multiple different products in a single transaction (e.g., Premium unlock + 100 coins).
- **Why deferred**: Backend likely sells single items (Premium unlock, ad removal, etc.). Multi-item carts would require bundled products.
- **When needed**: If business introduces bundle products (e.g., "Premium + 100 credits").
- **Implementation note**: ProductsV2 API already supports `productLineItem[]` array; backend can iterate and grant each item by `productId` and `quantity`. DB schema may need to store line items separately.
- **Test scenarios**: No deferred test code. Existing OTP-01 through OTP-RTDN-02 test only single-item purchases (cart support deferred to Phase 2).

#### 2. Partial Refunds
Allow users to refund a portion of a multi-item purchase instead of only full refunds.
- **Why deferred**: Business requirement is "full refunds or nothing".
- **When needed**: If partial refund support becomes a business policy.
- **Implementation note**: ProductsV2 API tracks `refundableQuantity` per line item. For single-item purchases, either full refund (0 refundable) or none.
- **Test scenarios**: No deferred test code. Existing OTP-05 and OTP-RTDN-02 test full refunds only (partial refunds deferred to Phase 2).

#### 3. 3-Day Acknowledgement Window Stress Testing
Add integration tests that verify the backend handles the edge case of acknowledging purchases near the 3-day deadline.
- **Why deferred**: Testing explicitly waiting 5 minutes per test is impractical. Current approach is better: verify acknowledgement happens immediately.
- **When needed**: Load testing or simulated delay injection if latency becomes a concern.
- **Implementation note**: Add integration test with mocked time or background job retry logic. Ensure acknowledgement handler has no artificial delays.
- **Test scenarios**: No deferred test code. Existing OTP-01 covers immediate acknowledgement only (stress testing edge cases deferred to Phase 2).

#### 4. Pre-Order Handling
Support for pre-ordered products that transition from PENDING to PURCHASED state when fulfilled.
- **Why deferred**: Not applicable to current product roadmap.
- **When needed**: If pre-orders are introduced (e.g., early access to new story content).
- **Implementation note**: Pre-orders use PENDING state initially. When fulfilled, Google sends `ONE_TIME_PRODUCT_PURCHASED` webhook with state transition PENDING → PURCHASED. Treat like OTP-04.
- **Test scenarios**: No deferred test code. Existing OTP-04 (slow card) can serve as pre-order proxy if feature is enabled (pre-order handling deferred to Phase 2).

#### 5. App-Side Token Restoration
Enable the app to query Google Play directly to fetch and restore purchase history instead of relying solely on backend records.
- **Why deferred**: Current design: Backend (`payments` table) is source of truth for entitlements. App queries `/api/v1/subscription-status` to restore premium status on app launch.
- **When needed**: If app requires offline-first token caching or "Restore Purchases" feature.
- **Implementation note**: Query backend, not Google Play directly. Reduces API calls and ensures consistency.
- **Note**: Per HLD, deprecated `BillingClient.queryPurchaseHistory()` should NOT be used.
- **Test scenarios**: No deferred test code. Feature requires no test implementation (app-side restoration deferred to Phase 2).

### Section B: Subscriptions

#### 1. Multiple Subscription Tiers
Support for multiple subscription tiers (e.g., "Basic", "Premium", "Enterprise") with upgrade/downgrade flows.
- **Why deferred**: Product setup required in Google Play Console.
- **When needed**: Once multi-tier pricing strategy is finalized.
- **Implementation note**: Create multiple base plans, implement `linkedPurchaseToken` handling, handle `cancel_reason: 2` (REPLACED) for old subscriptions, support upgrade/downgrade proration modes (e.g., `WITH_TIME_PRORATION`, `KEEP_EXISTING`).
- **Test scenarios**: `SUB-ADV-01` in test plan is marked "skip if tiers don't exist" → **SKIPPED** for this release.

#### 2. Prepaid Plans (Non-Renewing Subscriptions & Top-ups)
Non-renewing subscription plans where users purchase time upfront (e.g., "7-day pass", "30-day pass"). Multiple purchases (top-ups) accumulate time.
- **Why deferred**: Product setup required; this is a **payment model decision**, not just a technical feature. Requires product/business alignment.
- **When needed**: Once business model validates non-renewing revenue strategy.
- **Implementation note**: Configure non-renewing (prepaid) products in Google Play Console, enforce strict acknowledgment windows (≥1 week: 3 days, <1 week: half duration; failure triggers revocation and refund), track `linkedPurchaseToken` to previous purchases, accumulate `expiryTime` across top-ups, implement retry queue for failed acknowledgments.
- **Test scenarios**: `SUB-10`, `SUB-11`, `SUB-12` planned in test plan → **SKIPPED** for this release.

#### 3. Installment Subscriptions
Subscriptions with fixed commitment periods where users make monthly payments (e.g., "3-month commitment at \$9.99/month").
- **Why deferred**: Product setup required in Google Play Console.
- **When needed**: If targeting markets with strong installment payment adoption (e.g., India, Southeast Asia).
- **Implementation note**: Create installment plans with commitment details, track `installmentDetails.initialCommittedPaymentsCount` and `remainingCommittedPaymentsCount`, handle `SUBSCRIPTION_CANCELLATION_SCHEDULED` webhook (distinct from `SUBSCRIPTION_CANCELED`), monitor `pendingCancellation` object state during commitment, support developer-initiated cancellation API calls with `cancellationType` (USER_REQUESTED_STOP_RENEWAL vs DEVELOPER_REQUESTED_STOP_PAYMENTS).
- **Test scenarios**: `SUB-13` planned in test plan → **SKIPPED** for this release.

#### 4. Promo Codes
Promotional codes that grant free or discounted subscriptions (one-time codes or custom codes redeemable in Play Store or in-app).
- **Why not applicable**: App does not use promotional codes for subscriptions.
- **When needed**: If marketing strategy introduces promotional campaigns with code-based redemption.
- **Implementation note**: Requires setting up promo codes in Google Play Console, handling redemption before app install, while app is in foreground, and in multi-window mode. See Google docs: https://developer.android.com/google/play/billing/test#promo
- **Test scenarios**: `SUB-PROMO-01`, `SUB-PROMO-02`, `SUB-PROMO-03` reserved in test plan → **SKIPPED** for this release (feature not applicable).
