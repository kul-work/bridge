# Creem Billing Integration Test Plan

This document outlines comprehensive test scenarios for validating the Creem Billing integration in the Backend. It covers one-time payments, subscriptions, webhook handling, security verification, and edge cases required for a production release.

---

## Test Scenarios

### A. One-Time Payments (Non-Consumable)

**Reference**: [CREEM_ONE-TIME_LIFECYCLE-v1.0.md](./CREEM_ONE-TIME_LIFECYCLE-v1.0.md)

| ID | Scenario | Steps | Expected Frontend | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **OTP-01** | **Successful Purchase (Webhook)** | 1. Click "Buy".<br>2. Redirected to Creem Checkout.<br>3. Enter `4242` test card.<br>4. Complete payment. | - Redirected to `success_url`.<br>- Success message shown.<br>- Premium unlocked. | - Backend generates `POST /v1/checkouts` with `metadata.user_id`.<br>- Webhook `checkout.completed` received.<br>- Validates `creem-signature`.<br>- Extracts `metadata.user_id`.<br>- Updates `payments` table and grants entitlement. | Verifies the happy path using asynchronous webhooks as the primary source of truth. |
| **OTP-02** | **Sync Redirect Verification** | 1. Implement synchronous signature check on `success_url`.<br>2. Complete purchase as in OTP-01.<br>3. Block or delay webhook delivery for testing. | - Redirected to `success_url`.<br>- Immediate confirmation on page load. | - Backend reads `signature` query parameter from redirect.<br>- Verifies signature using API key.<br>- Grants entitlement synchronously if webhook hasn't fired yet. | Handles race conditions if the user lands on the success page before the webhook arrives. |
| **OTP-03** | **Refund Processed** | 1. Complete OTP-01.<br>2. Go to Creem Dashboard.<br>3. Refund the transaction. | - Upon app refresh, premium access is revoked. | - Webhook `payment.refunded` received.<br>- Validates signature.<br>- Looks up transaction in `payments` table.<br>- Updates status to `refunded` and revokes access. | Verified by `test-otp-02.sh`. |
| **OTP-04** | **Partially Refunded** | 1. Complete OTP-01.<br>2. Go to Creem Dashboard.<br>3. Partially refund the transaction. | - Premium access maintained or restricted per policy. | - Webhook `payment.partially_refunded` received.<br>- Updates `payments` status to `partially_refunded`. | Verified by `test-otp-04.sh`. |

---

### B. Subscriptions (Auto-Renewing)

**Reference**: [CREEM_SUBSCRIPTION_LIFECYCLE-v1.0.md](./CREEM_SUBSCRIPTION_LIFECYCLE-v1.0.md)

#### B.1 Subscription Lifecycle

| ID | Scenario | Steps | Expected Frontend | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **SUB-01** | **Initial Subscription (Active)** | 1. Select subscription plan.<br>2. Enter `4242` test card.<br>3. Complete checkout. | - Redirected to success URL.<br>- "Premium" status appears. | - Webhook `subscription.active` received on first sign-up.<br>- `metadata.user_id` parsed.<br>- Sub record created with `status='active'`.<br>- Premium access granted.<br>- Verify `subscription.paid` also handled for subsequent renewal cycles (see SUB-03). | Standard conversion of a new subscriber. Both `subscription.active` and `subscription.paid` must be handled independently. |
| **SUB-02** | **Free Trial Signup** | 1. Select plan with trial period.<br>2. Complete checkout (no initial charge). | - "Premium (Trial)" status appears.<br>- No charge shown. | - Webhook `subscription.trialing` received.<br>- Sub built with `status='trialing'`.<br>- Premium access granted. | User gets full access during trial. Next event will be `subscription.paid` or `subscription.past_due`. |
| **SUB-03** | **Subscription Renewal** | 1. Wait for billing cycle to renew (or mock event via CLI). | - Premium status maintained. | - Webhook `subscription.active` (renewal) or `subscription.paid` received.<br>- `current_period_end` extended in the database.<br>- Status remains `active`. | Verified by `test-sub-02.sh`. |
| **SUB-04** | **Payment Failure (Past Due)** | 1. Active sub renews with a failing card (`4000...0002`). | - In-app notification to update payment method. | - Webhook `subscription.past_due` received.<br>- Update DB status to `unpaid` / `past_due`.<br>- Depending on SLA, restrict access or trigger grace period logic. | Creem will retry according to platform settings. |
| **SUB-05** | **Retries Exhausted (Expiration)** | 1. Past due subscription exhausts all retries.<br>2. Creem expires the subscription. | - "Premium" badge disappears.<br>- Downgraded to free. | - Webhook `subscription.expired` received.<br>- Update DB status to `expired`.<br>- Premium access **revoked**. | Terminal state for failed payments. |

#### B.2 Cancellation & Alteration

| ID | Scenario | Steps | Expected Frontend | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **SUB-06** | **Scheduled Cancellation** | 1. User visits Customer Portal.<br>2. Cancels subscription (at period end). | - Premium badge remains.<br>- Shows message: "Cancels on [Date]". | - Webhook `subscription.scheduled_cancel` received.<br>- Update `auto_renewing=false` in DB.<br>- Status stays `active`.<br>- Access **retained** until period end. | Default behavior. User gets what they paid for until the cycle ends. |
| **SUB-07** | **Immediate Cancellation** | 1. Admin cancels subscription in Dashboard immediately. | - Premium badge disappears immediately. | - Webhook `subscription.canceled` received.<br>- Update status to `canceled`.<br>- Access **revoked** immediately. | Strict cancellation. |
| **SUB-08** | **Resume Scheduled Cancellation** | 1. User with scheduled cancel (SUB-06) visits Portal.<br>2. Clicks "Resume Subscription" (backend calls `POST /v1/subscriptions/{id}/resume`). | - "Cancels on" message disappears.<br>- Premium status normal. | - Webhook `subscription.active` received confirming resume.<br>- `auto_renewing=true` restored in DB.<br>- Status confirmed `active`. | Must occur before the period end date. Resume endpoint: `POST /v1/subscriptions/{id}/resume`. |
| **SUB-09** | **Plan Upgrade/Downgrade** | 1. User visits Portal.<br>2. Changes from Monthly to Yearly. | - Next invoice reflects change.<br>- Plan details updated. | - Webhook `subscription.update` received.<br>- Update `product_id`, billing cycle, and amount in DB. | Proration depends on dashboard configuration. |
| **SUB-10** | **Admin Pause** | 1. Admin pauses subscription via Dashboard/API. | - Premium badge disappears immediately. | - Webhook `subscription.paused` received.<br>- Status updated to `paused`.<br>- Access **revoked**. | Billing halts; access halts. |

#### B.3 Incomplete Checkouts, Recovery & Refunds

| ID | Scenario | Steps | Expected Frontend | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **SUB-11** | **Incomplete Checkout — 3DS Completed** | 1. Select subscription plan.<br>2. Enter a 3DS test card.<br>3. Pause at the 3DS authentication prompt.<br>4. Complete the 3DS challenge. | - Redirected to success URL after 3DS pass.<br>- "Premium" status appears. | - Subscription transitions from `incomplete` to `active`.<br>- Webhook `subscription.active` received.<br>- DB record created with `status='active'`.<br>- Premium access granted. | Subscription starts in `incomplete` state when 3DS or additional auth is required. No backend action on `incomplete` itself; wait for `subscription.active`. |
| **SUB-12** | **Subscription Payment Refunded** | 1. Complete SUB-01 (active subscription).<br>2. Go to Creem Dashboard.<br>3. Issue a refund on the latest subscription payment. | - Upon app refresh, premium access is revoked (per business policy). | - Webhook `refund.created` received.<br>- Signature verified.<br>- Backend determines correct action per refund policy (revoke immediately or flag for review).<br>- DB updated accordingly (e.g., status `refunded` or flagged).<br>- Premium access revoked if policy requires. | Distinct from OTP-04 (one-time refund). For subscriptions the access decision depends on business policy — define the policy before testing. |
| **SUB-13** | **Payment Recovery from Past Due** | 1. Put subscription in `past_due` state (SUB-04).<br>2. User visits Customer Portal and updates to a valid card (`4242`).<br>3. Creem retries and processes the payment successfully. | - In-app "payment failed" notice disappears.<br>- "Premium" badge reinstated. | - Webhook `subscription.paid` received after successful retry.<br>- DB status updated back to `active`.<br>- `current_period_end_date` extended.<br>- Premium access fully reinstated. | Tests the recovery path: `past_due` → `active`. Critical to ensure grace period state is cleared and access is properly restored. |
| **SUB-14** | **Scheduled Cancellation Period Ends (Expiry)** | 1. Put subscription in scheduled cancel state (SUB-06).<br>2. Advance time past the `current_period_end_date` (or mock the event). | - "Cancels on" date passes.<br>- Premium badge disappears. | - Webhook `subscription.expired` received when billing period ends.<br>- DB status updated to `expired`.<br>- Premium access revoked. | Validates the natural terminal transition of a scheduled cancel: `scheduled_cancel` → `expired`. SUB-06 only tests the setup; this tests the conclusion. |
| **SUB-15** | **Admin Resumes Paused Subscription** | 1. Put subscription in `paused` state (SUB-10).<br>2. Admin resumes the subscription via Dashboard or API. | - Premium badge reappears. | - Webhook `subscription.active` received after admin resume.<br>- DB status updated to `active`.<br>- Premium access restored. | Validates the `paused` → `active` recovery path. Ensure entitlement is restored cleanly with no gaps or duplicates. |

---

### C. Webhook & Verification Integrity

**Security Requirement**: All webhooks must be verified using the `creem-signature` HMAC-SHA256 hash.

| ID | Scenario | Steps | Expected Result | Backend Behavior | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **WHK-01** | **Valid Signature Acceptance** | 1. Send legitimate webhook payload with correct `creem-signature`. | - HTTP 200/204 OK. | - Signature passes validation.<br>- Payload processed and state updated. | Baseline for healthy communication. |
| **WHK-02** | **Invalid Signature Rejection** | 1. Send legitimate JSON payload but alter the `creem-signature` header.<br>2. Or alter the JSON body while keeping the valid signature. | - HTTP 400/401 Unauthorized. | - Signature validation routine fails.<br>- Logs: "Webhook signature verification failed."<br>- Request discarded safely. | Protects against spoofed payment notifications. |
| **WHK-03** | **Duplicate Delivery (Idempotency)** | 1. Trigger realistic webhook (`checkout.completed`).<br>2. Re-send the exact payload via cURL/Postman. | - First returns 200, updates DB.<br>- Second returns 200, no duplicate state. | - Uses webhook ID or event ID to ensure idempotent processing.<br>- If ID exists, skip processing. | Creem may retry requests if network is unstable; apps must process exactly once. |
| **WHK-04** | **Unknown Event Type** | 1. Send webhook with an unexpected/future `type` (e.g., `new_feature.enabled`). | - HTTP 200 OK. | - Backend parses payload, ignores unknown type.<br>- No state change, no errors. | Verified by `test-whk-04.sh`. |
| **WHK-05** | **Webhook Normalization** | 1. Send `checkout.completed` (recurring). | - HTTP 200 OK. | - Normalizer maps `checkout.completed` + `recurring` → `active` subscription. | Verified by `test-whk-05.sh`. |

---

### D. Network & Race Conditions

| ID | Scenario | Steps | Expected Behavior | Backend State | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **NET-01** | **Webhook Retry & Backoff** | 1. Trigger checkout success.<br>2. Force backend to return HTTP 500 for webhooks temporarily.<br>3. Restore backend to HTTP 200. | - User sees success (via sync redirect if implemented).<br>- Webhook initially fails, then succeeds on Creem retry. | - Creem retries after 30s, 1m, 5m, 1h.<br>- Once backend returns 200, DB updates properly. | Tests robustness of the system against internal backend outages. |
| **NET-02** | **Concurrent Sync & Webhook** | 1. User lands on `success_url` simultaneously as webhook arrives. | - Only one entitlement granted. | - DB uses transactions/upserts to prevent duplicate rows. | Race condition handling for fulfillment. |
| **NET-03** | **Bridge-to-App Delivery Verification** | 1. Trigger Creem webhook event.<br>2. Wait for async webhook forwarding.<br>3. Verify `pay.webhook_delivery` record shows success. | - Webhook successfully forwarded to downstream app.<br>- Backend logs 2xx response from app. | - Background worker processes `webhook_queue`.<br>- Calls app callback URL with canonical payload.<br>- Updates `webhook_delivery` table with `forwarded=true`. | **End-to-End Integrity**: Ensures Bridge successfully relays Creem events to the final destination. |

---

### D.2 Dead-Letter and Admin Retry (Cross-Provider)

These scenarios are shared with the [Google Play testplan](../google/GOOGLE_PLAY_BILLING_TESTPLAN.md) and the [Bridge Admin testplan](../BRIDGE_ADMIN_TESTPLAN.md). They are listed here for completeness so the Creem acceptance can verify the full lifecycle including operator intervention.

| ID | Scenario | Steps | Expected Result | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **DLQ-01** | **Webhook Forwarding Exhausts Retries → Dead-Lettered** | 1. Configure app with a `webhook_callback_url` that returns HTTP 500 for all requests.<br>2. Trigger a legitimate Creem webhook (e.g., `checkout.completed`).<br>3. Wait for all retry attempts to exhaust (3 attempts per `WEBHOOK_ARCHITECTURE.md`).<br>4. Query `webhook_delivery` table. | - `webhook_delivery` row has `dead_lettered=true`, `dead_lettered_at` NOT NULL, `dead_letter_reason` set.<br>- `forward_attempts=3`, `forwarded=false`.<br>- `last_http_status=500`, `last_error` contains response detail. | - `pay.webhook_delivery` row transitions through retry cycle: `forward_attempts` 0→1→2→3, then `dead_lettered=true`.<br>- `webhook_provider` row remains `processed=true` (ingestion succeeded, forwarding failed).<br>- No `payments`/`subscriptions` duplication from retry attempts. | **DB-side dead-letter assertion**. Complements NET-01 (which covers the *retry/backoff* case). This covers the *exhaustion* case where the app is consistently unreachable. See migration `04_create_webhooks.sql` columns `dead_lettered`, `dead_lettered_at`, `dead_letter_reason`. |
| **DLQ-02** | **Dead-Lettered Webhook Recovered via Admin Retry** | 1. Complete DLQ-01 (dead-lettered delivery exists).<br>2. Fix the app callback URL (make it return 200).<br>3. Call `POST /admin/webhooks/:webhook_id/retry` with valid admin JWT (see [Bridge Admin testplan](../BRIDGE_ADMIN_TESTPLAN.md) ADMIN-WHK-01).<br>4. Verify delivery succeeds. | - HTTP 200 from admin endpoint.<br>- App receives signed callback.<br>- `webhook_delivery.forwarded=true`, `dead_lettered=false`. | - Retry clears `dead_lettered` and resets forwarding state.<br>- Idempotency maintained: no duplicate `payments`/`subscriptions` rows.<br>- Audit log records admin actor. | Tests operator recovery path. Full scenario details in [BRIDGE_ADMIN_TESTPLAN.md](../BRIDGE_ADMIN_TESTPLAN.md). |

---

### E. Access Control & Entitlement Logic

| ID | Scenario | Steps | Expected Behavior | Backend Logic | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **ACC-01** | **Premium Access Granted for Active States** | 1. Test subscription in each "active" state: `active`, `trialing`, `paid`.<br>2. Call `/api/v1/subscription-status` (or equivalent endpoint) to check entitlement.<br>3. Attempt to access a premium feature. | - Access granted.<br>- Premium feature works without error. | - Backend implements centralized access check function.<br>- If state is `active`, `trialing`, or `paid`, return `is_premium=true`. | Confirms that normal and trial users can access features. |
| **ACC-02** | **Premium Access Retained During Scheduled Cancel** | 1. Put subscription in `scheduled_cancel` state (subset of active).<br>2. Ensure current date is BEFORE the `current_period_end_date`.<br>3. Attempt to access premium feature. | - Access granted.<br>- Premium feature works. | - Status is `active` but `auto_renewing=false`.<br>- Centralized logic checks `current_time < current_period_end_date` and grants access. | User gets what they paid for until the cycle ends. |
| **ACC-03** | **Premium Access Revoked for Blocked States** | 1. Test subscription in each "blocked" state: `expired`, `paused`, `canceled` (immediate), `refunded`.<br>2. Call status endpoint.<br>3. Attempt to access premium feature. | - Access denied.<br>- Error shown stating subscription is inactive.<br>- Premium feature returns 403 or equivalent. | - Backend implements revocation logic.<br>- If state matches blocked states, return `is_premium=false`. | Immediate cut-off for terminal states. |
| **ACC-04** | **Premium Access During Past Due (Grace Period)** | 1. Put subscription in `past_due` state.<br>2. Attempt to access premium feature. | - Access granted (if grace period applies) OR<br>- Access denied (if strict cut-off).<br>- Notification to update payment shown. | - Backend handles `past_due` according to business logic.<br>- If grace period is active, grant access but flag for payment notification. | Need to define exact business logic for `past_due` customers (whether they get a grace period or immediate restriction). |

---

### F. Error & Edge Cases

| ID | Scenario | Steps | Expected Behavior | Backend Response |
| :--- | :--- | :--- | :--- | :--- |
| **ERR-01** | **Missing Metadata.user_id** | 1. Create checkout via API manually, omitting metadata.<br>2. Complete checkout.<br>3. Monitor webhook. | - Payment goes through on Creem.<br>- Backend cannot assign entitlement. | - Backend receives webhook, fails to find `metadata.user_id`.<br>- Logs ERROR: "Orphaned payment detected."<br>- Returns 200 OK to Creem (so it doesn't retry), but alerts admin. |
| **ERR-02** | **Invalid Customer Portal Call** | 1. Call `POST /v1/customers/billing` (or `/v1/customer-portal`) with a non-existent `customer_id`. | - API returns error. | - Gracefully handles HTTP error from Creem SDK/API.<br>- Returns friendly error to user: "Error generating billing portal."<br>- Does NOT expose raw Creem error payload to the client. |

---

## Operational Logging & Monitoring

| Event | Log Level | Required Fields | Notes |
| :--- | :--- | :--- | :--- |
| **Checkout Created** | INFO | `user_id`, `product_id`, `checkout_id` | Audit trail for purchase intent. |
| **Webhook Received** | DEBUG/INFO | `event_type`, `webhook_id` | Track incoming traffic. |
| **Webhook Validated** | INFO | `event_type`, `user_id` | Core fulfillment traceability. |
| **Signature Failure** | WARN | IP Address, Headers | Monitor spoofing attempts. |
| **Entitlement Granted**| INFO | `user_id`, `plan_id` | Success metric. |
| **Entitlement Revoked**| INFO | `user_id`, `reason` (`expired` / `refunded`) | Debug downgrade complaints. |

*(Optional)* **Heartbeat Monitoring**: Implement a scheduled job that queries `creem subscriptions list --status past_due` to proactively alert the team about failing payments before `expired` webhooks are fired.

---

## Acceptance Criteria for Production

- ✅ **Checkout Creation**: All checkout sessions pass `metadata.user_id` properly.
- ✅ **One-Time Fulfillment**: OTP-01 through OTP-04 are thoroughly tested.
- ✅ **Subscription Logic**: SUB-01 through SUB-15 are verified against test cards (lifecycle, cancellations, incomplete checkouts, recovery, and refunds).
- ✅ **Cancellation Scenarios**: Both immediate (SUB-07) and scheduled (SUB-06, SUB-14) cancellations behave correctly regarding access, including period-end expiry.
- ✅ **Security**: `creem-signature` validation is strictly enforced; no open webhooks.
- ✅ **Idempotency**: Duplicate webhooks do not duplicate database records.
- ✅ **Portal Access**: Users can successfully access the Customer Portal to manage their plans.
- ✅ **Logging**: Security and transaction events are properly logged without exposing raw secrets.
- ✅ **Dead-Letter**: Webhook forwarding exhaustion produces `dead_lettered=true` with reason (DLQ-01).
- ✅ **Admin Retry**: Dead-lettered webhook recovered via admin retry without duplicate DB entries (DLQ-02); see [BRIDGE_ADMIN_TESTPLAN.md](../BRIDGE_ADMIN_TESTPLAN.md).
- ✅ **Admin Auth**: Admin endpoints reject mismatched `azp`, wrong issuer, and missing tokens; see [BRIDGE_ADMIN_TESTPLAN.md](../BRIDGE_ADMIN_TESTPLAN.md) acceptance criteria.
