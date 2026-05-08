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
- **Context**: Bridge records financial failures for any provider, but the user-facing acknowledgment flow was not ported.
- **Missing Behavior**:
    - HiHa logs the callback but does not create notification audit rows or persist a local "failure alert" flag.
    - Tyde HiHa does not register the old `POST /api/v1/notifications/payment-failure/acknowledge` endpoint.
- **Action**: Implement local notification storage and the acknowledgment endpoint in HiHa.

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
