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
- **Context**: This is a Google-specific regulatory flow for price increases. Users must explicitly accept or decline price changes within a deadline.
- **Current State**:
    - ✅ HiHa **correctly receives** `subscription.price_step_up` webhook from Bridge
    - ✅ HiHa **stores consent status** in `subscription_cache`: `google_requires_price_step_up_consent`, `google_price_step_up_consent_deadline`
    - ✅ Webhook handler (`src/handlers/webhooks.rs`) processes the event and logs it
    - ❌ **Missing**: User-facing routes to accept/decline are **not registered** in `src/main.rs`
    - ❌ **Missing**: No calls to Bridge's `resume` (accept) or `cancel` (decline) endpoints from HiHa
- **HiHa Spec Reference**:
    - `POST /api/v1/subscription/price-step-up/accept` — should call Bridge to resume subscription
    - `POST /api/v1/subscription/price-step-up/decline` — should call Bridge to cancel subscription
    - See `BEHAVIORAL_SPEC.md` §15 for expected behavior
- **Action**: Implement the two routes in HiHa to bridge user consent to Bridge's pause/resume APIs. Decide: call Bridge directly or redirect to Google Play Store UI?

## 4. OTP Refund/Revoke Classification
- **Affected Providers**: **All Providers** (Google Play, Creem)
- **Context**: HiHa must distinguish between One-Time Purchase (OTP) refunds and subscription refunds to ensure lifetime entitlements are revoked correctly.
- **Current State**:
    - ✅ HiHa **correctly handles** `purchase.one_time + completed` → grants lifetime premium
    - ✅ HiHa **correctly handles** `purchase.one_time + cancelled|refunded` → revokes premium (via `OneTimePurchaseRevoked` enum)
    - ❌ **Missing**: OTP refunds arriving as `payment.refunded` events are **misclassified as subscription refunds**
    - ❌ **Root cause**: Bridge normalizes OTP refunds into `payment.refunded` events:
        - **Google Play**: `ONE_TIME_PRODUCT_REFUNDED` → `payment.refunded` (not `purchase.one_time + refunded`)
        - **Creem**: `refund.created` (OTP) → `payment.refunded` (not `purchase.one_time + refunded`)
        - See `bridge/src/webhooks/processor/normalize.rs` for normalization logic
- **Classification Gap**:
    - HiHa currently treats **all** `payment.refunded` events as subscription inactivity
    - HiHa has **no payload field** to distinguish product type (e.g., `product_type`, `entitlement_kind`)
    - Result: OTP refunds are revoked correctly (outcome is right, semantics are wrong), but future multi-entitlement scenarios will break
- **Action** (Recommended by Oracle):
    - **Option A** (Preferred - Bridge side): Bridge should send OTP refunds as `purchase.one_time + status: refunded` instead of `payment.refunded`. This reuses HiHa's existing `OneTimePurchaseRevoked` logic.
    - **Option B** (HiHa side): Add explicit `product_type` or `entitlement_kind` field to Bridge's webhook payload so HiHa can classify based on data, not event type.

---

## Technical Debt / Missing Tests
- **Coverage Required**: 
    - **Google Play**: High priority for complex lifecycle events (Step-up, Drift).
    - **Creem**: High priority for verifying normalization of standard events (Success, Failure, Refund) and ensuring HiHa side-effects match.
