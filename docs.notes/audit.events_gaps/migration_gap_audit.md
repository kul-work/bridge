# Legacy HiHa to Tyde Bridge + HiHa Migration Audit

## Overview
This audit maps the business logic, lifecycle events, and side effects from the legacy HiHa webhook and subscription event handling (`C:\share\hiha`) to the new architecture utilizing Tyde Bridge (`C:\share\tyde\bridge`) and Tyde HiHa (`C:\share\tyde\hiha`).

## Architectural Ownership Map

| Responsibility | Legacy HiHa | New Architecture (Intended) | New Architecture (Actual / Gap) |
| :--- | :--- | :--- | :--- |
| **Webhook Ingestion & Normalization** | HiHa Webhooks | Bridge (`webhooks::processor`) | Fully owned by Bridge. Normalization and deduping logic is properly isolated. |
| **Database State (Subscriptions)** | HiHa DB (`users`, `subscriptions`) | Bridge DB (`subscriptions`) | Bridge tracks granular states. HiHa only tracks simplified `is_premium` access in its `users` table via callback sync. |
| **Lifecycle Notifications (Emails)** | HiHa (`send_email_mock`) | Bridge (`services::google_play::notifications`) | **Unowned/Broken**: Bridge has the logic but fails to invoke it in the event loop. |
| **User Experience (UX) Flags** | HiHa App Context | HiHa via Bridge API polling | **Broken Integration**: HiHa parses Bridge's API responses incorrectly, resulting in dropped UX flags. |
| **Webhook Callbacks (Bridge to HiHa)**| N/A | Bridge Forwarder -> HiHa `webhooks.rs` | Functional, but HiHa's handler discards many granular events and falls back to simple logging. |

---

## Gap Analysis (Ordered by Severity)

### 1. Critical: API Schema Mismatch Breaking All Premium UX Flags
Tyde HiHa relies on polling the Bridge API to surface important UX flags to the user (e.g., payment failures, price step-ups, grace periods). However, there is a fundamental mismatch between the Bridge response and the HiHa parser.

**Code-Level Details:**
*   **Bridge (`tyde/bridge/src/handlers/subscriptions.rs`)**: 
    The `GET /api/v1/subscriptions` endpoint returns a `ListSubscriptionsResponse` containing a `"subscriptions"` array of `SubscriptionDetail` objects. This lightweight struct does *not* include advanced fields like `google_requires_price_step_up_consent`, `payment_failure_message`, etc. Only the `GET /api/v1/subscriptions/{id}` endpoint returns `SubscriptionDetailFull` with these fields.
*   **Tyde HiHa (`tyde/hiha/src/services/bridge_client.rs` & `handlers/content.rs`)**:
    `bridge_client.get_subscription_status` hits the list endpoint (`/api/v1/subscriptions`), but `handlers::get_subscription_status` expects a flat object with root-level fields:
    ```rust
    let payment_failure_notification = result.get("payment_failure_notification").and_then(|v| v.as_bool());
    let google_requires_price_step_up_consent = result.get("google_requires_price_step_up_consent").and_then(|v| v.as_bool());
    ```
**Impact:** `result.get(...)` fails silently (evaluating to `None`) for all UX fields. Users will never see payment failure notifications, price increase consent prompts, or pause states in the frontend app.

### 2. High: Missing Lifecycle Notification Emails
The legacy system sent emails to users during critical lifecycle events, such as when a payment failed (`payment.failed`) or a subscription was revoked (`payment.refunded`).

**Code-Level Details:**
*   **Legacy HiHa**: Explicitly dispatched emails for `payment.failed`, `price_changed`, and `purchase.voided` in `webhooks/events/...`.
*   **Tyde Bridge**: Contains comprehensive email templates and notification functions in `C:\share\tyde\bridge\src\services\google_play\notifications.rs` (e.g., `send_email_payment_failed`, `send_email_revoked`, `send_email_price_step_up`).
*   **The Gap**: A full codebase audit (`grep_search`) reveals that **these notification functions are never invoked**. The event dispatcher (`tyde/bridge/src/webhooks/processor/event_handlers.rs`) updates the database and forwards the webhook to HiHa, but drops the email notification side-effects entirely. 

### 3. Medium: Loss of Granular Provider State in Downstream Client (HiHa)
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
*   **The Gap**: Because Bridge fails to propagate granular UX flags (see Gap 1), and HiHa's webhook receiver ignores non-binary states, advanced flows like "Paused" or "On Hold" are treated the exact same as "Cancelled", potentially creating a confusing user experience where the app instructs a user to buy a new subscription instead of updating their payment method.

### 4. Low: Dropped Cancellation Context & Survey Feedback
When users cancelled subscriptions, Google Play survey feedback was previously collected and utilized.

**Code-Level Details:**
*   **Tyde Bridge**: The `enrich_google_play_fields` function successfully extracts `google_cancellation_context` and `google_cancellation_feedback` and saves them during `apply_subscription_transition`.
*   **The Gap**: These fields are omitted from both `SubscriptionDetail` and `SubscriptionDetailFull` serialization in `tyde/bridge/src/handlers/subscriptions.rs`. As a result, the data is siloed in the Bridge database and inaccessible to administrators or the Tyde HiHa backend for churn analysis.

## Recommended Remediation Steps

1. **Fix API Integration (HiHa + Bridge)**: 
   * Modify Bridge's `GET /api/v1/subscriptions` to return a `SubscriptionDetailFull` object (or a dedicated proxy summary struct) so HiHa can access UX flags.
   * Modify Tyde HiHa's `handlers::get_subscription_status` to parse the `subscriptions` array correctly, mapping the properties of the *active* subscription to the frontend response.
2. **Implement Notification Dispatcher**: Wire the existing notification functions in `bridge/src/services/google_play/notifications.rs` into the event processors in `bridge/src/webhooks/processor/event_handlers.rs`. Ensure they are only dispatched for initial state changes (to prevent email spam during webhook retries).
3. **Expose Cancellation Metadata**: Add `google_cancellation_context` and `google_cancellation_feedback` to the API structs in `bridge/src/handlers/subscriptions.rs`.
