# Bridge Launch Observability Checklist

Keep paid observability minimal at launch. The goal is enough structured backend visibility to troubleshoot launch issues from logs without adding unnecessary tooling, while strictly enforcing PII compliance for payments.

## Current Code Audit

Status after auditing the Bridge backend codebase:

| Area | Status | Notes |
|---|---|---|
| 1. Health checks | Implemented | Liveness at `/health`. Readiness `/ready` asserts DB connectivity and returns 503 if enabled provider count is 0. |
| 2. Request IDs | Implemented | Custom HTTP middleware generates UUIDs or validates safe correlation-ids. |
| 3. Structured HTTP access logs | Implemented | Access log middleware prints single structured logs, extracting MatchedPath via route_layer. |
| 4. DB failure logging | Implemented | DB pool acquisition, queries, and readiness checks log structured context on failure. |
| 5. Webhook ingress failures | Implemented | Logs structured webhook ingress events, signature verification results, and idempotency status. |
| 6. Provider RPC failures | Implemented | API/client errors log and return sanitized/hashed details (raw response bodies and raw tokens are omitted/redacted). |
| 7. Subscription lifecycle | Implemented | State transitions and stale-event suppression decisions are logged with correlation IDs. |
| 8. Background worker jobs | Implemented | Background workers (reconciliation, price step-up, pause scheduler, cleanup) use structured ticks/logs. |
| 9. Webhook sub-deliveries | Implemented | Webhook forwarding logs retry attempts and emits structured `error!` dead-letter alerts on permanent failure. |
| 10. PII-safe correlation | Implemented | Enforces denylist; replaced suffix-exposing `redact_with_prefix` with `diagnostic_hash` for purchase tokens. |
| 11. PII leakage audit | Remedied | Completed PII leakage audit, resolved all leakage risks (emails scrubbed, tokens hashed, app callback bodies omitted). |
| 12. Troubleshooting runbook | Missing | No operational runbook at `docs/TROUBLESHOOTING.md` exists yet. |

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
