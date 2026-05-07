# Legacy HiHa to Tyde Bridge + HiHa Migration Audit

## Overview
This audit maps the business logic, lifecycle events, and side effects from the legacy HiHa webhook and subscription event handling (`C:\share\hiha`) to the new architecture utilizing Tyde Bridge (`C:\share\tyde\bridge`) and Tyde HiHa (`C:\share\tyde\hiha`).

## Architectural Ownership Map

| Responsibility | Legacy HiHa | New Architecture (Intended) | New Architecture (Actual / Gap) |
| :--- | :--- | :--- | :--- |
| **Webhook Ingestion & Normalization** | HiHa Webhooks | Bridge (`webhooks::processor`) | Fully owned by Bridge. Normalization and deduping logic is properly isolated. |
| **Database State (Subscriptions)** | HiHa DB (`users`, `subscriptions`) | Bridge DB (`subscriptions`) | Bridge tracks granular states. HiHa only tracks simplified `is_premium` access in its `users` table via callback sync. |
| **Webhook Callbacks (Bridge to HiHa)**| N/A | Bridge Forwarder -> HiHa `webhooks.rs` | Functional, but HiHa's handler discards many granular events and falls back to simple logging. |

---

## Gap Analysis (Ordered by Severity)

### 1. High: Loss of Granular Provider State in Downstream Client (HiHa)
The transition to normalized `BridgeWebhookPayload` models collapses complex provider states into generic events, causing the HiHa application to lose contextual awareness of specific transitions.

**Code-Level Details:**
*   **Legacy HiHa**: `webhooks/processor::process_webhook_async_remaining` handled ~30 different granular events.
*   **Tyde Bridge to HiHa**: Bridge forwards generic canonical events like `subscription.updated` or `subscription.cancelled`.
*   **Tyde HiHa (`tyde/hiha/src/db/webhook_callbacks.rs`)**: The `premium_access_update` function collapses these callbacks into binary `is_premium` states:
    ```rust
    "subscription.cancelled" | "subscription.expired" | "subscription.paused" | "subscription.revoked" | "subscription.on_hold" | "payment.refunded" => {
        Some(PremiumAccessUpdate::SubscriptionInactive)
    }
    ```
*   **The Gap**: HiHa's webhook receiver ignores non-binary states, advanced flows like "Paused" or "On Hold" are treated the exact same as "Cancelled", potentially creating a confusing user experience where the app instructs a user to buy a new subscription instead of updating their payment method.

### 2. Medium: Dropped Cancellation Context & Survey Feedback
When users cancelled subscriptions, Google Play survey feedback was previously collected and utilized.

**Code-Level Details:**
*   **Tyde Bridge**: The `enrich_google_play_fields` function successfully extracts `google_cancellation_context` and `google_cancellation_feedback` and saves them during `apply_subscription_transition`.
*   **The Gap**: These fields are omitted from both `SubscriptionDetail` and `SubscriptionDetailFull` serialization in `tyde/bridge/src/handlers/subscriptions.rs`. As a result, the data is siloed in the Bridge database and inaccessible to administrators or the Tyde HiHa backend for churn analysis.

## Recommended Remediation Steps

1. **Expose Cancellation Metadata**: Add `google_cancellation_context` and `google_cancellation_feedback` to the API structs in `bridge/src/handlers/subscriptions.rs`.
2. **Expand HiHa Webhook Processing**: Update HiHa's webhook payload and handler to store granular provider state in a local cache, allowing for better frontend UX without polling Bridge.
