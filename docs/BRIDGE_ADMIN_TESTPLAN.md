# Bridge Admin Interface Test Plan

This document covers the Bridge admin interface — manual webhook retry, scheduler triggers, admin authentication, and CSP enforcement. These scenarios are **not** provider-specific; they apply regardless of whether the source webhook came from Google Play or Creem.

**Related**: [GOOGLE_PLAY_BILLING_TESTPLAN.md](./google/GOOGLE_PLAY_BILLING_TESTPLAN.md), [CREEM_BILLING_TESTPLAN.md](./creem/CREEM_BILLING_TESTPLAN.md), [WEBHOOK_ARCHITECTURE.md](./WEBHOOK_ARCHITECTURE.md).

---

## Prerequisites

- Bridge running with `ADMIN_CLERK_ISSUER`, `ADMIN_CLERK_ORG_ID` (optional), and `ADMIN_CLERK_AUTHORIZED_PARTIES` configured.
- At least one app registered in `pay.apps` with a valid `webhook_callback_url`.
- At least one dead-lettered `webhook_delivery` row (see precondition in ADMIN-WHK-01).
- Clerk admin user with a valid JWT for the configured admin instance.

---

### A. Admin Webhook Retry

| ID | Scenario | Steps | Expected Result | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **ADMIN-WHK-01** | **Dead-Lettered Webhook Manually Retried** | 1. Seed a dead-lettered `webhook_delivery` row (3 failed forward attempts, `dead_lettered=true`, `dead_letter_reason` set).<br>2. Call `POST /admin/webhooks/:webhook_id/retry` with a valid admin JWT.<br>3. Verify the delivery row transitions and the downstream app receives the callback. | - HTTP 200 OK.<br>- App callback URL receives exactly one signed POST.<br>- Delivery row reflects `forwarded=true`, `dead_lettered=false`. | - `webhook_delivery.dead_lettered` cleared, `forward_attempts` reset or incremented per implementation.<br>- `forwarded=true`, `forwarded_at` set.<br>- **No duplicate `payments` or `subscriptions` rows** created by the retry — idempotency via `webhook_provider.provider_webhook_id` and `pay.webhook_delivery` unique constraint on `webhook_provider_id`.<br>- Audit log entry recorded with admin actor `sub`. | Tests that operator-initiated retry unblocks stuck deliveries without double-processing. |
| **ADMIN-WHK-02** | **Admin Retry Does Not Reopen Already-Forwarded Delivery** | 1. Find a `webhook_delivery` row with `forwarded=true` (already successfully delivered).<br>2. Call `POST /admin/webhooks/:webhook_id/retry` with a valid admin JWT.<br>3. Verify the delivery is NOT re-sent. | - HTTP 200 OK (or 409 Conflict, per implementation).<br>- No second callback POST to the app. | - `webhook_delivery.forwarded` remains `true`; `forward_attempts` unchanged.<br>- Downstream app receives zero additional callbacks.<br>- Audit log records the rejected retry attempt. | Regression test for the bug fixed in commit `de0fad7` ("Prevent admin retry from reopening forwarded deliveries"). |

---

### B. Admin Scheduler Trigger

| ID | Scenario | Steps | Expected Result | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **ADMIN-JOB-01** | **Manual trigger-jobs Is Idempotent** | 1. Ensure the webhook retry scheduler is idle (no pending deliveries).<br>2. Call `POST /admin/trigger-jobs` with a valid admin JWT.<br>3. Call `POST /admin/trigger-jobs` again immediately.<br>4. Verify no duplicate forward passes were created. | - Both calls return 200 OK.<br>- No extra `webhook_delivery` rows or duplicate forward attempts. | - `trigger_jobs` handler runs the scheduler pass; if nothing is pending, it completes without side effects.<br>- Second call is a no-op (or deduplicates by checking in-flight state).<br- Audit log records both calls with admin actor `sub`. | Tests that the manual trigger endpoint is safe to call repeatedly without causing duplicate work. |

---

### C. Admin Authentication

| ID | Scenario | Steps | Expected Result | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **ADMIN-AUTH-01** | **Admin Endpoint Rejects JWT Without Matching azp** | 1. Configure `ADMIN_CLERK_AUTHORIZED_PARTIES=https://admin.bridge.example.com`.<br>2. Obtain a valid Clerk JWT from the correct issuer but with a different `azp` claim (or no `azp`).<br>3. Call `POST /admin/trigger-jobs` with this JWT. | - HTTP 401 or 403 returned.<br>- No admin action performed. | - `admin_auth` middleware checks `azp` claim against `ADMIN_CLERK_AUTHORIZED_PARTIES`.<br>- Mismatch or missing `azp` → rejection logged with truncated token details (no raw JWT in logs).<br>- Scheduler not triggered. | Tests the `azp` enforcement added in commit `9b1a221`. `ADMIN_CLERK_ORG_ID` is optional (requires paid Clerk), but `azp` enforcement is mandatory when configured. |
| **ADMIN-AUTH-02** | **Admin Endpoint Rejects JWT From Wrong Issuer** | 1. Obtain a valid JWT from a different Clerk instance (e.g., the HouseHold production Clerk).<br>2. Call `POST /admin/trigger-jobs` with this JWT.<br>3. Verify rejection. | - HTTP 401 returned.<br>- No admin action performed. | - `admin_auth` middleware verifies `iss` claim against `ADMIN_CLERK_ISSUER`.<br>- Mismatch → rejection logged.<br>- JWKS fetch never attempted for unknown issuer. | Prevents cross-service JWT reuse. |
| **ADMIN-AUTH-03** | **Admin Endpoint Rejects Missing Bearer Token** | 1. Call `POST /admin/trigger-jobs` without an `Authorization` header.<br>2. Verify rejection. | - HTTP 401 returned. | - Middleware short-circuits before any admin logic.<br>- No database queries. | Baseline auth check. |

---

### D. Admin Dashboard CSP

| ID | Scenario | Steps | Expected Result | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **ADMIN-CSP-01** | **CSP Blocks External Scripts, Allows Clerk/Captcha Styles** | 1. Request `GET /admin` (or the admin dashboard route) with a valid admin JWT.<br>2. Inspect the `Content-Security-Policy` response header.<br>3. Verify inline scripts from non-allowed origins are blocked by the browser CSP.<br>4. Verify Clerk authentication styles and captcha flows load correctly. | - `Content-Security-Policy` header present.<br>- `script-src` restricts to allowed origins only.<br>- `style-src` permits Clerk and captcha provider styles.<br>- Dashboard renders and functions correctly. | - CSP header set by `handlers::admin` per commit `37c8bae`.<br>- No `unsafe-inline` on `script-src` unless strictly required for template initialization.<br>- `connect-src` allows Clerk JWKS endpoint and backend API only. | Tests the CSP hardening from commits `24b6616` and `37c8bae`. |

---

### E. Admin Mutation Audit Logging

| ID | Scenario | Steps | Expected Result | Backend Validation | Notes |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **ADMIN-AUDIT-01** | **Admin Actions Are Recorded in Audit Log** | 1. Perform any admin mutation (retry webhook, trigger-jobs, update app notes).<br>2. Inspect structured logs for audit entry. | - Audit log entry present with admin actor `sub`, action type, target ID, and timestamp. | - Audit fields: `actor` (admin `sub`), `action` (retry_webhook / trigger_jobs / update_notes), `target_id`, `timestamp`.<br>- No sensitive data (tokens, secrets) in audit log.<br>- Actor limits enforced: rate-limited per admin user to prevent abuse. | Tests actor limits and audit logging from commit `745843a`. |

---

## Acceptance Criteria for Production

- ✅ **ADMIN-WHK-01**: Dead-lettered webhook can be manually retried; no duplicate DB entries.
- ✅ **ADMIN-WHK-02**: Already-forwarded delivery is not reopened by admin retry.
- ✅ **ADMIN-JOB-01**: `POST /admin/trigger-jobs` is idempotent when scheduler is idle.
- ✅ **ADMIN-AUTH-01**: JWT with mismatched `azp` is rejected.
- ✅ **ADMIN-AUTH-02**: JWT from wrong issuer is rejected.
- ✅ **ADMIN-AUTH-03**: Missing bearer token is rejected.
- ✅ **ADMIN-CSP-01**: CSP allows Clerk/captcha flows, blocks unauthorized scripts.
- ✅ **ADMIN-AUDIT-01**: All admin mutations produce audit log entries with actor identification.