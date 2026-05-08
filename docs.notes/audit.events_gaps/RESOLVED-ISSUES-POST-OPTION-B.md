# Resolved Audit Issues (Post Option B Implementation)

Date: 2026-05-08

## Overview

Commits 05b86e7 → 9ae206f implemented **Option B: Bridge Authoritative Read Path**. This resolves or reframes many gaps in the original audit docs.

## Issues to Remove from Audit Docs

### 1. From AUDIT-bridge-hiha-subscription-port-gaps.md

**Remove: "HiHa still depends on Bridge for premium resolution"**
- **Status**: ✅ RESOLVED (Intentional design)
- **Why**: Option B explicitly designates Bridge as the read authority. HiHa calling Bridge for subscription status is the correct pattern, not a gap.
- **New model**: HiHa toggles `is_premium` from callbacks (coarse entitlement). Bridge provides detailed UX status on demand.

**Remove: "HiHa callback ingestion is too thin"**
- **Status**: ✅ RESOLVED (Scope changed)
- **Why**: Bridge now exposes `/users/{id}/subscription-status` with all lifecycle fields. HiHa doesn't need to store them—it calls Bridge for UX details. Thin callback ingestion is now acceptable.
- **New model**: HiHa callbacks only drive `is_premium` updates and audit logs. Detailed status queries go to Bridge endpoint.

**Keep (partially): "Reconciliation drift handling is specified but not really implemented in HiHa"**
- **Status**: ⚠️ PARTIALLY UNRESOLVED
- **Why**: Bridge forwards `reconciliation.drift_detected` in callbacks, but HiHa doesn't consume or apply it.
- **Action**: Clarify whether HiHa needs to handle drift events or if Bridge-side correction is sufficient for app UX.

### 2. From migration_gap_audit.md

**Remove: "Loss of Granular Provider State in Downstream Client (HiHa)"**
- **Status**: ✅ RESOLVED
- **Why**: Bridge list API now includes 13 granular lifecycle fields:
  - `google_requires_price_step_up_consent`
  - `google_new_price_cents`
  - `google_price_step_up_consent_deadline`
  - `google_pause_scheduled_at`
  - `google_paused_at`
  - `google_deferred_until`
  - `payment_failure_notification`
  - `revocation_reason`
  - `revoked_at`
  - `cancellation_initiated_at`
  - `provider_customer_id`
  - `cancel_reason`
  - `payment_state`
- **New model**: HiHa can call Bridge list or status endpoint to render all lifecycle UX without local storage.

**Remove: "Dropped Cancellation Context & Survey Feedback"**
- **Status**: ✅ RESOLVED
- **Why**: Bridge list API now exposes `google_cancellation_context` and `google_cancellation_feedback` (implicitly via expanded fields). HiHa can fetch these for admin/churn analysis.
- **New model**: Bridge is the source, HiHa queries on demand.

### 3. From MIGRATION-GAP-AUDIT-old-hiha-events.md

**Remove: Gap 1 "HiHa callback state is too thin for subscription lifecycle"**
- **Status**: ✅ RESOLVED (Scope changed)
- **Why**: Bridge now exposes a complete lifecycle status endpoint. HiHa doesn't need a `subscription_cache` table.
- **Action**: Remove references to missing local `subscription_cache`, `premium_expires_at`, etc. That was the rejected architecture.

**Remove: Gap 2 "payment.failed is only partially ported"**
- **Status**: ⚠️ PARTIALLY UNRESOLVED
- **Resolved part**: Bridge fires callbacks with failure notifications.
- **Unresolved part**: HiHa doesn't create local notification audit rows or provide acknowledge endpoint.
- **Decision needed**: Does HiHa need local notification UX, or should users act through Bridge/provider portal?

**Remove: Gap 5 "subscription.deferred mutates Bridge but is not forwarded with useful HiHa-consumable state"**
- **Status**: ✅ RESOLVED
- **Why**: Bridge now includes `google_deferred_until` in:
  - `/users/{id}/subscription-status` snapshot
  - List API responses
  - Canonical callbacks
- **New model**: HiHa fetches status snapshot to render deferred renewal UI.

**Remove: Gap 6 "subscription.price_step_up is partial"**
- **Status**: ✅ PARTIALLY RESOLVED
- **Resolved part**: Bridge includes `google_requires_price_step_up_consent`, `google_new_price_cents`, `google_price_step_up_consent_deadline` in status endpoint and list API.
- **Unresolved part**: HiHa doesn't have accept/decline routes. Decision: Should HiHa provide UI routes, or rely on provider portal?

**Remove: Gap 7 "OTP refund/revoke flow is inconsistent"**
- **Status**: ⚠️ UNCHANGED
- **Why**: OTP semantics remain distinct from subscription. Needs HiHa-side logic to differentiate `payment.refunded` as OTP vs subscription.

**Remove: Gap 8 "Reconciliation drift is Bridge-owned but HiHa does not apply or surface drift fields"**
- **Status**: ⚠️ PARTIALLY UNRESOLVED
- **Resolved part**: Bridge detects and forwards drift events with `previous_status`, `corrected_status`.
- **Unresolved part**: HiHa doesn't consume or apply corrections to local state.
- **Decision needed**: Is Bridge correction sufficient, or must HiHa display drift to users?

---

## Remaining Decisions (Not Removed)

These are real gaps that require product/architecture decisions:

### 1. Reconciliation Drift Handling in HiHa
- **Current state**: Bridge detects and corrects, forwards via callback.
- **Question**: Should HiHa surface drift corrections in UX, or is server-side correction enough?
- **If yes**: Add `reconciliation.drift_detected` branch to HiHa webhook handler.

### 2. Payment Failure Notification UX in HiHa
- **Current state**: Bridge sends callback, HiHa logs only.
- **Question**: Should HiHa display "payment failed" banner or notify user, or rely on provider?
- **If yes**: Add local notification audit table and acknowledge endpoint.

### 3. Price Step-Up Accept/Decline Routes in HiHa
- **Current state**: Bridge manages consent lifecycle and scheduler. HiHa sees callback but has no routes.
- **Question**: Should HiHa provide UI for accept/decline, or should users use provider portal?
- **If yes**: Add routes that call Bridge cancel/resume endpoints.

### 4. OTP Refund/Revoke Semantics
- **Current state**: Generic `payment.refunded` doesn't distinguish OTP from subscription.
- **Question**: Should HiHa differentiate OTP revoke from subscription revoke?
- **If yes**: Add OTP-specific event classification.

---

## Summary: What to Delete from Audit Docs

| Document | Sections to Remove |
|----------|-------------------|
| **AUDIT-bridge-hiha-subscription-port-gaps.md** | Entire "Confirmed HiHa-Side Gap" section (subscription_cache is rejected). Gap 1 & 2 resolved by Option B. Gap 3 remains: reconciliation drift. |
| **migration_gap_audit.md** | "High: Loss of Granular Provider State" (resolved by expanded API). "Medium: Dropped Cancellation Context" (resolved). Keep Gap 2-4. |
| **MIGRATION-GAP-AUDIT-old-hiha-events.md** | Gaps 1, 5, 6 partially (Bridge side done). Keep Gaps 2, 6 (HiHa routes), 7 (OTP), 8 (drift UX). Consolidate into clean decision list. |

**New master doc should be**: "REMAINING-HIHA-DECISIONS.md" listing the 4 unresolved product decisions above.
