# Bridge Launch Observability Checklist

Keep paid observability minimal at launch. The goal is enough structured backend visibility to troubleshoot launch issues from logs without adding unnecessary tooling, while strictly enforcing PII compliance for payments.

## Current Code Audit

Status after auditing the Bridge backend codebase:

| Area | Status | Notes |
|---|---|---|
| 1. Health checks | Liveness Only | `/health` returns status healthy but does not check DB readiness or provider config availability. |
| 2. Request IDs | Missing | Bridge has no HTTP middleware to extract or generate request IDs. A local `trace_id` is only generated inside `verify_purchase` for the local `BpTrace`. |
| 3. Structured HTTP access logs | Missing | Standard tower-http `TraceLayer` is used, which writes unstructured logs. Custom structured HTTP access logs with `request_id`, latency, and `app_id` are missing. |
| 4. DB failure logging | Missing | SQLx errors are mapped directly to `BridgeError::DbError` strings and returned, but are not logged with structured diagnostic context at the query site. |
| 5. Webhook ingress failures | Unstructured | Webhook signature/duplicate failures log plain text warnings/errors (e.g., `Creem webhook signature verification failed`) without structured `app_id` or `request_id`. |
| 6. Provider RPC failures | Unstructured | Google Play / Creem HTTP API failures log raw response bodies or error strings without structured `provider`, `app_id`, or `subscription_id` spans/fields. |
| 7. Subscription lifecycle | Unstructured | State transition checks (stale suppression, restarts) write plain text warnings/infos in `event_handlers.rs` without consistent structure or correlation fields. |
| 8. Background worker jobs | Unstructured | Reconciliation, cleanup, price step-up, and pause scheduler workers log plain text startup/failure events without structured job context or retry tracing. |
| 9. Webhook sub-deliveries | Partially Implemented | Outbound forwarding logs attempt count and status, but lacks structured fields in production (only debug mode logs them) and misses explicit dead-letter logs. |
| 10. PII-safe correlation | Non-Compliant | Logs use `redact_with_prefix` (exposing last 8 characters of purchase tokens) instead of secure `diagnostic_hash`. `BPT-RAW` logs dump raw response bodies. |
| 11. PII leakage audit | Audit Done | Audit revealed three major leakage risks: partially redacted purchase tokens in logs, raw response bodies from Google/Creem APIs, and potential email/PII echo on errors. |
| 12. Troubleshooting runbook | Missing | No operational runbook at `docs/TROUBLESHOOTING.md` exists for Bridge. |

---

## Must Have Checklist Items

### 1. Health & Readiness Checks
- Maintain liveness check at `/health`.
- Add readiness check at `/ready` that:
  - Asserts main database connection pool is healthy (performs `SELECT 1`).
  - Verifies basic provider configurations are loaded and valid.
- Clear error logs on startup, bind, or readiness failures.

### 2. Request IDs in Middleware
- Add a custom middleware to:
  - Extract `x-request-id` from incoming request headers or generate a new UUID v4.
  - Insert the `request_id` into response headers.
  - Ensure all request handling occurs inside a tracing span associated with the `request_id`.

### 3. Structured HTTP Access Logs
- Custom access log middleware writing a single structured JSON line per request with:
  - `request_id`
  - `method`
  - `path`
  - `status` (HTTP response code)
  - `latency_ms`
  - `app_id` (if API key authenticated)
  - `error_code` (if the request failed)

### 4. Useful DB Failure Logging
- Log structured messages on database failures:
  - Connection failures and pool initialization errors.
  - Pool acquisition timeouts.
  - Query timeouts or statement execution errors.
  - RLS session context (`bridge.current_app_id`) setup failures.

### 5. Webhook Ingress Failure Visibility
- Log structured webhook ingress events:
  - Webhook received with `app_id`, `provider`, `event_type`, `event_id`.
  - Signature verification failures (without raw signature data).
  - Idempotency deduplication/stale-event suppression.
  - JSON payload validation or schema matching failures.
  - Forwarding/enqueue failures.

### 6. Provider RPC Failure Visibility
- Log Google Play and Creem API failures:
  - HTTP non-success status code, auth token failures, timeout, rate-limiting.
  - Structured fields containing `provider`, `app_id`, `subscription_id`, and `request_id`.
  - **Forbidden**: Do not log raw HTTP response bodies or raw purchase tokens on failure.

### 7. Subscription Lifecycle Tracing
- Log state transitions for subscriptions with structured fields:
  - Old state -> New state transitions.
  - Stale-event suppression decisions.
  - Refund, revoke, pause, resume, defer, and price-consent updates.
  - Correlation fields: `app_id`, `subscription_id`, `provider`, `external_user_id` (opaque).

### 8. Background Worker Visibility
- Trace jobs run by background workers:
  - Reconciliation: Drift detection, corrective actions, email warning failures.
  - Price step-up: Consent expiry detection and auto-cancellations.
  - Pause scheduler: Pause state transitions and orphaned pending cleanup.
  - Cleanup: Webhook logs and fraud prevention database cleanup status.

### 9. Webhook Sub-delivery & Retry Visibility
- Detailed visibility into the 3-strike delivery strategy:
  - Job received, current attempt count (1 of 3, etc.), delay/backoff.
  - Outbound HTTP response status or connection errors.
  - **Dead-lettering**: Log a structured `error!` alert immediately when a delivery exhausts all 3 attempts and is permanently marked `dead_lettered`.

### 10. PII-Safe Correlation Fields (The Denylist)
- Enforce strict logging PII denylist:
  - **No** customer email addresses or customer names.
  - **No** raw purchase tokens or raw webhook signatures.
  - **No** Bridge API keys or HMAC secrets.
  - **No** raw provider bodies in production log targets.
- Replace `redact_with_prefix` in logging with `diagnostic_hash` (first 12 chars of SHA256) for purchase tokens.

### 11. PII Leakage Remediation
- Modify `BPT-RAW` logs in `google_play/client.rs` to only log at `debug` level and strip any potential sensitive fields.
- Wrap Creem error bodies to prevent raw validation error echo from leaking client email addresses.
- Ensure all returned error payloads are scrubbed of sensitive fields before formatting.

### 12. Operational Troubleshooting Runbook
- Write `docs/TROUBLESHOOTING.md` covering:
  - Startup/bind/readiness failure resolution.
  - Database connection & RLS authentication troubleshooting.
  - Webhook signature or decryption issues.
  - Provider RPC connectivity, timeouts, or credential updates.
  - Webhook delivery backlog/dead-letter queue manual recovery steps.
  - Incident response runbook for accidental PII leakage in logs.
