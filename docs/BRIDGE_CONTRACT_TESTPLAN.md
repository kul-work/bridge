# Bridge Contract & Cross-App Isolation Test Plan

This document covers Bridge contract acceptance — the end-to-end path between Bridge and downstream apps (HouseHold, HiHa, future apps), with a focus on **cross-app tenant isolation**. Bridge is shared infrastructure; a regression in RLS or app-scoping would let one app's payment/webhook data leak to another app.

**Related**: [WEBHOOK_ARCHITECTURE.md](./WEBHOOK_ARCHITECTURE.md), [API_CONTRACT.md](./API_CONTRACT.md), [GOOGLE_PLAY_BILLING_TESTPLAN.md](./google/GOOGLE_PLAY_BILLING_TESTPLAN.md), [CREEM_BILLING_TESTPLAN.md](./creem/CREEM_BILLING_TESTPLAN.md).

---

## Prerequisites

- Bridge running with at least **two apps** registered in `pay.apps` (e.g., `household` and `hiha`).
- Each app has its own `webhook_ingress_token`, `webhook_callback_url`, and `api_keys`.
- RLS enabled (migrations `90_enable_row_level_security.sql`, `91_fix_rls_current_app_id_cast.sql`, `92_enable_checkout_idempotency_rls.sql`).
- `SECURITY DEFINER` functions in `pay` schema enforce `app_id` scoping via `current_app_id()`.

---

### A. Cross-App Tenant Isolation

| ID | Scenario | Steps | Expected Result | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **ISO-01** | **Subscription Visibility Across Apps** | 1. Seed Bridge with App A and App B.<br>2. Create a subscription for App A with `external_user_id="user_123"`.<br>3. Query `GET /api/v1/users/user_123/subscription-status` using App B's API key. | - HTTP 404 or empty response (not 200 with App A's subscription data).| - RLS policy on `pay.subscriptions` enforces `app_id = current_app_id()`.<br>- App B's API key resolves to App B's `app_id`; `pay.subscriptions` rows for App A are invisible.<br>- `SECURITY DEFINER` functions (`current_app_id()`) enforce scoping at the DB level. | **Highest-risk regression class**: one app seeing another app's subscription/payment state. Migration 90 enables RLS; migration 91 fixes the `current_app_id()` cast. |
| **ISO-02** | **Payment History Isolation Across Apps** | 1. Seed Bridge with App A and App B.<br>2. Create payments for App A with `external_user_id="user_456"`.<br>3. Query `GET /api/v1/payments?external_user_id=user_456` using App B's API key. | - HTTP 200 with empty payment list (not App A's payments). | - RLS on `pay.payments` enforces `app_id` scoping.<br>- No payment rows from App A visible to App B.<br>- Index `idx_pay_app_user` on `(app_id, external_user_id)` is scoped by RLS. | Same isolation class as ISO-01, applied to the `payments` table. |
| **ISO-03** | **Webhook Delivery Isolation Across Apps** | 1. Seed Bridge with App A and App B.<br>2. Ingest a webhook for App A (Google Play or Creem).<br>3. Query the admin webhook delivery list using App A's API key, then App B's. | - App A sees its own `webhook_delivery` row.<br>- App B sees zero rows for App A's webhook. | - RLS on `pay.webhook_provider` and `pay.webhook_delivery` enforces `app_id` isolation.<br>- `webhook_provider.app_id` and `webhook_delivery.app_id` are scoped by `current_app_id()`. | Tests that webhook forwarding state is also app-scoped, not just subscription/payment data. |
| **ISO-04** | **Webhook Ingress Token Cannot Resolve Wrong App** | 1. App A has `webhook_ingress_token=TOKEN_A`.<br>2. Send a provider webhook to `/webhooks/TOKEN_A/google_play` but sign it with App B's webhook secret. | - HTTP 404 (token resolves App A) or 401 (signature mismatch).<br>- Webhook not processed under App B. | - Ingress token `TOKEN_A` resolves to App A only.<br>- Signature verification uses App A's webhook secret, not App B's.<br>- No cross-app ingestion path exists. | Prevents token confusion attacks where a provider webhook for one app is routed to another app's context. |
| **ISO-05** | **Checkout Idempotency Key Isolation** | 1. App A creates a checkout with `idempotency_key="key_abc"`.<br>2. App B creates a checkout with the same `idempotency_key="key_abc"`. | - Both checkouts succeed independently (different `app_id` + same key is allowed).<br>- App B does NOT receive App A's cached checkout response. | - `checkout_idempotency` table has `UNIQUE (app_id, idempotency_key)` (migration 05, line 31).<br>- Idempotency is scoped per app, not globally. | Tests that idempotency keys cannot collide across apps. |

---

### B. Bridge Contract Acceptance (Endpoint Shape Conformance)

These scenarios verify that Bridge's API contract matches what downstream apps expect. They are the **staging acceptance** counterpart to the local mock compatibility tests.

| ID | Scenario | Steps | Expected Result | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **CONTRACT-01** | **Checkout Endpoint Shape** | 1. Call `POST /api/v1/payment/checkout` from HouseHold with valid product config.<br>2. Verify response shape. | - `201 Created` with checkout URL and metadata.<br>- Response includes `checkout_url`, `provider`, `product_id`. | - Bridge returns the checkout session URL from the provider (Creem or Google Play).<br>- `metadata.user_id` is set from the authenticated Clerk ID. | Validates the endpoint contract that HouseHold's `BRG-02` test depends on. |
| **CONTRACT-02** | **Purchase Registration Shape** | 1. Call `POST /api/v1/purchase/register` from HouseHold with subscription pre-registration fields.<br>2. Verify response shape. | - `200 OK` with registration confirmation. | - Bridge stores `(external_user_id, purchase_token, subscription_id)` mapping.<br>- Response matches documented API contract. | Validates the pre-registration path that Google Play `SUB-01` depends on. |
| **CONTRACT-03** | **Verify Purchase Shape** | 1. Call `POST /api/v1/verify-purchase` from HouseHold with a purchase token and subscription ID.<br>2. Verify response shape. | - `200 OK` with verification status and entitlement fields. | - Bridge calls provider API, stores payment/subscription records, returns normalized status. | Validates the verification path that Google Play `ACK-01` and Creem `OTP-01` depend on. |
| **CONTRACT-04** | **Subscription Status Shape** | 1. Call `GET /api/v1/users/{external_user_id}/subscription-status` from HouseHold.<br>2. Verify response shape. | - `200 OK` with `is_premium`, lifecycle fields, provider-specific details. | - Response includes `is_premium`, `status`, `current_period_end`, `auto_renewing`, and provider-specific fields per the API contract. | Validates the primary entitlement contract that HouseHold's `BRG-01` and Google Play `ACC-01`/`ACC-02` depend on. |
| **CONTRACT-05** | **Signed Callback Delivery** | 1. Trigger a provider webhook (Google Play or Creem).<br>2. Verify Bridge forwards a signed callback to the app's `webhook_callback_url`. | - App receives POST with `X-Pay-Signature`, `X-Pay-Timestamp`, `X-Pay-Event-Id` headers.<br>- Payload matches the canonical webhook format. | - HMAC-SHA256 signature over the payload using the app's shared secret.<br>- `X-Pay-Event-Id` matches `webhook_provider.provider_webhook_id` for idempotency.<br>- App returns 200/204 to acknowledge. | Validates the end-to-end forwarding path from provider → Bridge → app. Complements NET-05 in the Google/Creem testplans. |
| **CONTRACT-06** | **Signed Email Lookup** | 1. Bridge calls `POST /internal/bridge/email-lookup` on HouseHold with a signed request.<br>2. Verify HouseHold returns the user's email. | - `200 OK` with `{ "email": "user@example.com" }`.<br>- Unsigned request rejected with `400 Bad Request`. | - HouseHold verifies `X-Pay-Signature` HMAC before responding.<br>- Email lookup is internal/guarded — not a public endpoint.<br>- Post-HMAC rate limit (`RATE_LIMIT_BRIDGE_EMAIL_LOOKUP`) applies. | Validates the signed email lookup path between Bridge and HouseHold. Complements HouseHold `BRG-03`/`BRG-03A`. |

---

## Acceptance Criteria for Production

- ✅ **ISO-01**: Subscription data is not visible across app boundaries (RLS enforced).
- ✅ **ISO-02**: Payment history is not visible across app boundaries (RLS enforced).
- ✅ **ISO-03**: Webhook delivery state is not visible across app boundaries (RLS enforced).
- ✅ **ISO-04**: Webhook ingress token resolves to the correct app only; cross-app signature mismatch rejected.
- ✅ **ISO-05**: Checkout idempotency keys are scoped per app, not globally.
- ✅ **CONTRACT-01 through CONTRACT-06**: All Bridge API endpoint shapes match the documented contract when run against the real staging Bridge service (not only local mocks).
- ✅ **Signed Callbacks**: All callbacks to downstream apps carry valid `X-Pay-Signature` HMAC headers.
- ✅ **Signed Email Lookup**: Internal email lookup endpoint is guarded by HMAC and post-HMAC rate limiting.