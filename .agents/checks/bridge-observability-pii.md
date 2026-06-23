# Bridge Observability & PII Compliance Checker

Use when a Bridge change touches logging, error formatting, HTTP handlers, health/readiness checks, provider client API requests/responses, or database error boundaries.

## Check

- Tracing and log messages contain no customer email addresses or customer names.
- Tracing and log messages contain no raw purchase tokens, raw signatures, Bridge API keys, or HMAC secrets.
- Any purchase tokens logged/traced use `diagnostic_hash` (the first 12 characters of the SHA256 of the token) instead of `redact_with_prefix` or raw values.
- Reqwest/HTTP error paths scrub/redact the request URL if it contains raw purchase tokens.
- Raw provider response bodies (via `BPT-RAW` log target) are demoted to `debug` level to avoid leakage in production logs, and scrubbed of potential PII.
- Custom structured HTTP access logs output a single JSON line per request containing: `request_id`, `method`, `path`, `status`, `latency_ms`, `app_id` (if authenticated), and `error_code` (on failure).
- Database query failures, pool acquisition timeouts, and RLS session context setup failures log structured diagnostic context instead of plain text strings.
- Webhook ingress failures (signatures, duplicates, stale events) log structured context with `app_id`, `provider`, and `webhook_provider_id`.
- Background worker loops (reconciliation, price step-up, pause transitions, cleanup) write structured log events with correct correlation and job context.
- Webhook sub-delivery retries write structured logs detailing attempt count, and log an explicit `error!` alert when a job reaches its 3-strike limit and is marked `dead_lettered`.

## Evidence to collect

- Diff hunks touching logging statements (`info!`, `warn!`, `error!`, `debug!`), access log middleware, readiness checks, background workers, or provider API clients.
- Verify test output of `tests/pii-leak-regression-test.sh`.

## Output

```text
Observability and PII compliance verdict: PASS / FAIL / NOT APPLICABLE

Findings:
- ...

Evidence:
- files:
- tests:
```
