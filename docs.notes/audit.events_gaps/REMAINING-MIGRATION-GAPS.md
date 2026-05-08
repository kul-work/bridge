# Remaining Migration Gaps: Bridge + HiHa (Action Items)

**Date**: 2026-05-08

This document lists only the remaining technical and product gaps that require implementation or decision-making in the Tyde HiHa / Bridge architecture, categorized by provider impact.

---

## 1. Reconciliation Drift Handling in HiHa
- **Affected Providers**: **Google Play** (Primary)
- **Context**: Bridge identifies and corrects billing drift (mostly for Google Play due to polling requirements), then forwards `reconciliation.drift_detected`.
- **Missing Behavior**:
    - HiHa's webhook handler (`tyde/hiha/src/handlers/webhooks.rs`) lacks an actual state-correction branch for `reconciliation.drift_detected`.
    - HiHa callback payload structs omit critical corrective fields: `previous_status`, `corrected_status`, and `reconciliation_source`.
- **Action**: Implement a handler in HiHa to apply correction fields arriving from Bridge.

## 2. Payment Failure Notification & Acknowledgment
- **Affected Providers**: **All Providers** (Creem, Google Play)
- **Status**: **Fixed** (2026-05-08)
- **Context**: Bridge records financial failures for any provider and remains the canonical owner of the active `payment_failure_notification` flag.
- **Current State**:
    - HiHa registers the user-facing `POST /api/v1/notifications/payment-failure/acknowledge` endpoint
    - HiHa authenticates the user, rate-limits the request, and delegates canonical clearing to Bridge's `POST /api/v1/subscriptions/:subscription_id/acknowledge`
    - HiHa creates local `payment_failure` notification audit rows when Bridge forwards `payment.failed`
    - HiHa notification audit rows have `acknowledged_at` so user acknowledgment is tracked separately from email delivery status
    - HiHa subscription status returns a user-facing payment failure message when Bridge reports `payment_failure_notification = true`
- **Resolution**: Implemented in HiHa as an app-facing proxy/audit layer. Bridge remains the source of truth for the active payment failure flag; HiHa's local notification records are audit/UI history only.

## 3. Price Step-Up Consent Routes
- **Affected Providers**: **Google Play Only**
- **Status**: **Fixed** (2026-05-08)
- **Context**: This is a Google-specific regulatory flow for price increases. Users must explicitly accept or decline price changes within a deadline.
- **Current State**:
    - HiHa correctly receives `subscription.price_step_up` webhook from Bridge
    - HiHa stores consent status in `subscription_cache`: `google_requires_price_step_up_consent`, `google_price_step_up_consent_deadline`
    - Webhook handler (`src/handlers/webhooks.rs`) processes the event and logs it
    - HiHa registers user-facing accept/decline routes in `src/main.rs`
    - HiHa delegates accept/decline to Bridge's dedicated price step-up endpoints
    - Bridge `price-step-up/decline` calls Google Play cancellation before mutating Bridge state
    - HiHa Frontend opens Google Play subscription management for acceptance and calls HiHa for decline
- **HiHa Spec Reference**:
    - `POST /api/v1/subscription/price-step-up/accept` - opens Google Play consent flow on FE, then confirms/refreshes via HiHa -> Bridge
    - `POST /api/v1/subscription/price-step-up/decline` - calls HiHa -> Bridge dedicated decline endpoint, which cancels the Google Play subscription
    - See `BEHAVIORAL_SPEC.md` section 15 for expected behavior
- **Resolution**: Implemented across Bridge, HiHa backend, and HiHa Frontend. Google Play remains the authority for recording acceptance; Bridge owns provider-side cancellation on decline.

## 4. OTP Refund/Revoke Classification
- **Affected Providers**: **All Providers** (Google Play, Creem)
- **Status**: **Fixed** (2026-05-08)
- **Context**: HiHa must distinguish between One-Time Purchase (OTP) refunds and subscription refunds to ensure lifetime entitlements are revoked correctly.
- **Current State**:
    - ✅ HiHa **correctly handles** `purchase.one_time + completed` → grants lifetime premium
    - ✅ HiHa **correctly handles** `purchase.one_time + cancelled|refunded` → revokes premium (via `OneTimePurchaseRevoked` enum)
    - ✅ Bridge now emits OTP refunds as `purchase.one_time + status: refunded`
    - ✅ **Google Play**: `ONE_TIME_PRODUCT_REFUNDED` → internal `purchase.one_time_refunded` → callback `purchase.one_time + refunded`
    - ✅ **Creem**: explicit OTP `refund.created` payloads → internal `purchase.one_time_refunded` → callback `purchase.one_time + refunded`
    - ✅ **Creem subscriptions**: `refund.created` remains `payment.refunded`
- **Resolution**: Implemented Bridge-side using HiHa's existing `purchase.one_time + refunded` revoke path. No HiHa payload expansion is needed for this gap.

---

## Technical Debt / Missing Tests
- **Coverage Required**: 
    - **Google Play**: High priority for complex lifecycle events (Step-up, Drift).
    - **Creem**: High priority for verifying normalization of standard events (Success, Failure, Refund) and ensuring HiHa side-effects match.
