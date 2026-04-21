# Creem Billing Test Execution Flows


This guide condenses the comprehensive `CREEM_BILLING_TESTPLAN.md` into logical end-to-end execution flows. By running through these narratives, you natively trigger and validate multiple test conditions at once.

## Creem Dashboard Setup

1. **Test Mode Enabled**: Ensure you are operating in **Test Mode** within the Creem Dashboard.
2. **API Keys**: Use the Test Mode Secret API Key to authenticate backend requests (e.g., generating checkout sessions).
3. **Webhook Configuration**: Configure webhooks in the dashboard to point to the backend testing environment (e.g., using ngrok or a local tunnel).
    - Ensure you copy the **Test Webhook Secret** used for HMAC-SHA256 signature verification.
4. **Test Products**: Create at least one One-Time product and one Subscription product/plan in the dashboard.

### Testing Tools & Data

- **Test Cards**: Use Creem-provided test cards to simulate payment outcomes.
  - `4242 4242 4242 4242` - Successful payment
  - `4000 0000 0000 0002` - Card declined (simulates `subscription.past_due`)
  - `4000 0000 0000 0069` - Expired card

### Backend Prerequisite: Metadata Binding
**Critical**: All Checkouts initiated by the backend **MUST** pass `metadata.user_id` (or equivalent identifier) to ensure webhooks can map transactions back to the correct internal user.


## 1. Subscription Flows

### 🌊 Flow 1.1: The Standard Lifecycle (Happy Path ➔ Cancel ➔ Resume ➔ Expire)
This simulates a normal user who signs up, enjoys the product, decides to cancel, changes their mind, and eventually leaves.
1. **User signs up** for a standard plan with a valid test card (`4242`).
   - *Covers*: **SUB-01** (Initial Subscription) and **ACC-01** (Access granted).
2. **Trigger/wait for renewal**.
   - *Covers*: **SUB-03** (Subscription Renewal).
3. **User goes to Portal and cancels**.
   - *Covers*: **SUB-06** (Scheduled Cancellation) and **ACC-02** (Verify they still have access until the period ends).
4. **User clicks "Resume Subscription"** before expiration.
   - *Covers*: **SUB-08** (Resume Scheduled Cancel).
5. **User cancels again**, and you fast-forward time past the end date.
   - *Covers*: **SUB-14** (Scheduled Period Ends) and **ACC-03** (Verify access is finally revoked).

### 💳 Flow 1.2: The Dunning Process (Failing Cards & Recovery)
This tests what happens when billing fails and how the system handles the "past due" states.
1. **Trigger a renewal with a failing test card** (`4000 0000 0000 0002`).
   - *Covers*: **SUB-04** (Payment Failure / Past Due) and **ACC-04** (Verify your grace period access logic).
2. **Branch A (Recovery):** User goes to the Portal and enters a valid card. 
   - *Covers*: **SUB-13** (Payment Recovery from Past Due).
3. **Branch B (Churn):** Run the flow again but let Creem exhaust all retry attempts.
   - *Covers*: **SUB-05** (Retries Exhausted / Expiration).

### 🛠️ Flow 1.3: The Admin Intervention
This simulates actions your customer support or admins take via the Creem Dashboard.
1. **User has an active subscription**.
2. **Admin hits "Pause"** in Dashboard.
   - *Covers*: **SUB-10** (Admin Pause). Verify access drops immediately.
3. **Admin hits "Resume"**.
   - *Covers*: **SUB-15** (Admin Resumes). Verify access is restored cleanly.
4. **Admin hits "Cancel Immediately"**.
   - *Covers*: **SUB-07** (Immediate Cancellation). Verify immediate access cutoff.
5. **Create a fresh subscription, then Admin issues a Refund**.
   - *Covers*: **SUB-12** (Subscription Refunded). Verify your business logic triggers (e.g., immediate revoke).

### 🚀 Flow 1.4: Trials & Plan Conversions
Testing how users interact with trial periods and changing their minds on plans.
1. **User signs up for a Free Trial** plan.
   - *Covers*: **SUB-02** (Trial Signup). 
2. **User goes to Portal and upgrades/downgrades** (e.g., switches from Monthly to Yearly) **during the trial period**.
   - *Covers*: **SUB-09** (Plan Upgrade/Downgrade). Verify the new plan is adopted but the user is not billed until the original trial period ends (if that is your business logic).
3. **Wait for the trial to end**.
   - *Covers*: **SUB-03** (Subscription Renewal). Ensure they are billed for the *new* plan amount.
4. **User goes to Portal and upgrades/downgrades** (e.g., from Basic to Pro) **while on an active, paid plan**.
   - *Covers*: **SUB-09** (Plan Upgrade/Downgrade). Check if the next invoice/status adjusts correctly and proration is handled according to Dashboard settings.

### 🛡️ Flow 1.5: The 3DS Authentication (Edge Case)
Just a quick, isolated scenario to ensure strong customer authentication works.
1. **User checks out with a 3DS-required test card**.
2. **Stop at the prompt, then complete the challenge**.
   - *Covers*: **SUB-11** (Incomplete Checkout ➔ 3DS Completed).

---

## 2. One-Time Payments (OTP) Flows

### 🛍️ Flow 2.1: OTP Happy Path & Revocation
Tests a standard non-consumable purchase and a subsequent admin refund.
1. **User clicks "Buy" and checks out** with the `4242` test card.
   - *Covers*: **OTP-01** (Successful Purchase via Webhook) and **OTP-02** (Sync redirect verification if implemented). Check that premium access is unlocked immediately.
2. **Admin goes to the Creem Dashboard and issues a Refund**.
   - *Covers*: **OTP-03** (Refund Creation). Refresh the app to verify premium access is dynamically revoked.

---

## 3. Webhook Integrity & Security Flows

### 🔒 Flow 3.1: Payload Security & Forward Compatibility
Send webhook payloads manually (e.g., via Postman or cURL) to verify the backend drops malicious traffic but handles unknown future events smoothly.
1. **Send a legitimate JSON payload with a broken/invalid `creem-signature` header**.
   - *Covers*: **WHK-02** (Invalid Signature). Expect HTTP 400/401.
2. **Send a valid payload (correct signature) but with a fake event type** (e.g., `new_feature.enabled`).
   - *Covers*: **WHK-04** (Unknown Event Type). Expect HTTP 200 OK and no backend crash.

### 🔂 Flow 3.2: Webhook Idempotency
Ensures network retries don't duplicate state.
1. **Trigger a real webhook event** (like `checkout.completed`). Wait for the system to process it (HTTP 200).
2. **Resend the exact same payload** using your testing tool.
   - *Covers*: **WHK-03** (Duplicate Delivery). Ensure the database isn't updated twice and access doesn't trigger duplicate logs or entitlement grants.

---

## 4. Network & Race Conditions Flows

### ⏱️ Flow 4.1: Internal Outage & Retry Mechanisms
Verifies what happens if your backend temporarily goes down while someone checks out.
1. **Force your backend webhook endpoint to return HTTP 500**.
2. **User completes a successful payment**.
   - *Note*: User should still see the success page if synchronous UI redirects are implemented.
3. **Wait a few minutes, then restore your backend to return HTTP 200**.
   - *Covers*: **NET-01** (Webhook Retry & Backoff). Verify Creem eventually retries the event and the backend processes it successfully.

### 🏎️ Flow 4.2: Concurrent Racing
1. **Simulate firing the `success_url` loading (client-side) and webhook arrival (server-side) at the exact same millisecond**.
   - *Covers*: **NET-02** (Concurrent Sync & Webhook). Verify the system uses database constraints/transactions to avoid saving two payment records for the same transaction.

---

## 5. Edge Cases & Errors Flows

### ⚠️ Flow 5.1: Missing Metadata
Verifies how the system behaves when critical payment linkage metadata is lost or omitted.
1. **Create a checkout session manually (e.g., via Creem API CLI)** and do not include the `metadata.user_id`.
2. **Complete the payment**.
   - *Covers*: **ERR-01** (Missing Metadata.user_id). Monitor backend logs to ensure it logs an orphaned payment gracefully but returns 200 OK to Creem to clear the event.

### ❌ Flow 5.2: Invalid Portal Generation
1. **Trigger a backend call to generate the Creem Customer Portal** using a fake or malformed `customer_id`.
   - *Covers*: **ERR-02** (Invalid Customer Portal Call). Ensure the frontend gracefully shows a user-friendly error instead of an API crash.
