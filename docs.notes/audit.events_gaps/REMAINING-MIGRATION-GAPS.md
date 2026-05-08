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
- **Context**: This is a Google-specific regulatory flow for price increases.
- **Missing Behavior**:
    - No `accept` or `decline` routes are registered in `tyde/hiha/src/main.rs`.
    - The app cannot currently facilitate the user's "Action Required" for price increases.
- **Action**: Decide if HiHa should host these routes (calling Bridge cancel/resume endpoints) or if users must be redirected to the Google Play Store.

## 4. OTP Refund/Revoke Classification
- **Affected Providers**: **All Providers** (identified via Google Play Audit)
- **Context**: HiHa needs to distinguish between One-Time Purchase (OTP) and subscription refunds to ensure lifetime entitlements are handled correctly.
- **Missing Behavior**:
    - Generic `payment.refunded` events in HiHa's webhook receiver currently map exclusively to subscription "Inactive" semantics.
- **Action**: Add OTP-specific event classification and revocation logic to the HiHa entitlement service.

---

## Technical Debt / Missing Tests
- **Coverage Required**: 
    - **Google Play**: High priority for complex lifecycle events (Step-up, Drift).
    - **Creem**: High priority for verifying normalization of standard events (Success, Failure, Refund) and ensuring HiHa side-effects match.
