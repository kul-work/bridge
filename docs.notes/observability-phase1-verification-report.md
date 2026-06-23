# Bridge Observability Phase 1 Verification Report

Date: 2026-06-23

Source checklist: `docs.notes/observability-launch-checklist.md`

## Summary

Phase 1 is locally verified for the implemented observability surfaces that can be exercised in this workspace.

Remaining local gaps:

- DB-down readiness and DB failure logging were not fault-probed by restarting Bridge with a broken database configuration.
- Production log aggregation, alert routing, and runbook execution were not verified locally.
- Real external provider outage behavior was not exercised; provider-failure coverage is based on local/simulated test paths.

## Test Inventory

- Rust: `cargo check`, `cargo test`
- PII: `tests/pii-leak-regression-test.sh`
- Security: `tests/security/test-runner.sh`
- CTI: `tests/cti/run-all-iso-tests.sh`, `tests/cti/run-all-contract-tests.sh`
- Admin: `tests/admin/test-runner.sh --clear`
- GPBI: `tests/gpbi/test-runner.sh --scope infra`, `tests/gpbi/test-net-05.sh`
- Creem: `tests/creem/test-runner.sh --scope whk`, `--scope net`, `--scope err`, `tests/creem/test-net-03.sh`
- Syntax: `bash -n tests/*.sh tests/admin/*.sh tests/creem/*.sh tests/cti/*.sh tests/gpbi/*.sh tests/security/*.sh`

## Results

| Check | Result | Evidence |
|---|---:|---|
| `cargo check` | PASS | Built with `CARGO_TARGET_DIR=C:\share\tyde\target-bridge`. |
| `cargo test` | PASS | 111 unit/lib/main tests passed; 10 `tests/creem_webhook_tests.rs` tests passed; DB-backed `db::webhooks::tests::manual_retry_reset_does_not_reopen_forwarded_deliveries` ran as `ok`, not skip-only. |
| Shell syntax check | PASS | All checked shell scripts parsed with `bash -n`. |
| `tests/pii-leak-regression-test.sh` | PASS | No email, sentinel token, or callback secret leakage detected in captured cargo-test logs. |
| `tests/security/test-runner.sh` | PASS | 1/1 cross-app tenant isolation test passed. |
| `tests/cti/run-all-iso-tests.sh` | PASS | 5/5 isolation tests passed. |
| `tests/cti/run-all-contract-tests.sh` | PASS | 6/6 endpoint contract tests passed. |
| `tests/admin/test-runner.sh --clear` | PASS | 8/8 admin tests passed, including dead-letter retry and admin audit logging. |
| `tests/gpbi/test-runner.sh --scope infra` | PASS | Infrastructure coverage passed, including Bridge-to-app delivery verification with delivery `t|200|`. |
| `tests/creem/test-runner.sh --scope whk` | PASS | 6/6 webhook tests passed. |
| `tests/creem/test-runner.sh --scope err` | PASS | 2/2 error tests passed. |
| `tests/creem/test-runner.sh --scope net` | PASS | Network coverage passed, including Creem Bridge-to-app delivery verification with delivery `t|200|`. |

## Runtime Probes

| Probe | Result | Evidence |
|---|---:|---|
| `GET http://127.0.0.1:5566/health` | PASS | Returned 200 with `status:"healthy"` and version `0.4.0`. |
| `GET http://127.0.0.1:5566/ready` | PASS | Returned 200 with `database:"ok"` and `enabled_provider_configs:4`. |
| Invalid API key with `x-request-id: obs-phase1-probe-bridge` | PASS | Returned 401; server log contained the same request id, `status=401`, and `error_code="unauthorized"`. |
| Callback dependency for NET tests | PASS | `localhost:3000` was `C:\share\tyde\hiha\target\debug\hiha.exe`; `GET /health` returned 200 with version `0.12.3`. |

## Checklist Mapping

| Checklist row | Status | Local verification |
|---|---:|---|
| Health checks | VERIFIED | `/health` and `/ready` are implemented in `src/handlers/mod.rs`; live probes returned 200; readiness reported DB ok and 4 enabled provider configs. |
| Request IDs | VERIFIED | `src/middleware/observability.rs` accepts or creates `x-request-id`, stores it in request extensions, returns it in responses, and logs it. Probe confirmed propagation. |
| Structured access logs | VERIFIED | Access logs include request id, method, matched path, status, latency, app id when authenticated, and error code on failures. Live logs confirmed success and 401/404/400 paths. |
| Database failures | UNVERIFIED LOCALLY | DB-backed flows and DB-ok readiness were exercised. DB-down readiness/logging was not fault-probed by restarting Bridge with a broken DB URL. |
| Webhook ingress failures | VERIFIED | GPBI LOG/WHK tests, Creem WHK tests, and contract unsigned/signed probes exercised signature and malformed-webhook rejection. Logs include request ids and `webhook_error` error codes. |
| Provider RPC failures | PARTIAL | Local/simulated retry, idempotency, error, and ACK logging paths were exercised. Real external provider outage behavior was not exercised. |
| Subscription lifecycle | VERIFIED | GPBI ACC/WHK/API/NOTIF and Creem WHK/ERR paths exercised activation, renewals, refunds, payment failure notification, unknown events, duplicates, and idempotency. |
| Background workers | PARTIAL | Source has structured `background_worker` spans for webhook retry, reconciliation, price step-up, pause scheduler, and cleanup. Admin `trigger-jobs` passed. Full scheduled interval behavior was not waited out. |
| Webhook sub-deliveries | VERIFIED | Admin dead-letter retry tests passed. GPBI and Creem Bridge-to-app delivery tests passed with HTTP 200 delivery. |
| PII-safe correlation | VERIFIED | `diagnostic_hash` is used across verify-purchase, ingress, scheduler, and forwarding diagnostics. Live logs show token/correlation hashes. |
| PII leakage remediation | VERIFIED WITH CAVEAT | PII regression passed. Caveat: debug forwarding diagnostics include outbound canonical payload fields such as `external_user_id` and callback URL; response body is omitted and purchase tokens are hashed or null. |
| Troubleshooting runbook | UNVERIFIED LOCALLY | The checklist references runbook coverage, but production log aggregation and alert-routing steps were not executed locally. |

## Source Evidence

- Health/readiness: `src/handlers/mod.rs`, `src/db/readiness.rs`
- Request/access logs: `src/middleware/observability.rs`
- Webhook ingress: `src/webhooks/ingress.rs`
- Webhook forwarding and dead-letter handling: `src/webhooks/forwarding.rs`, `src/db/webhooks.rs`
- Background workers: `src/webhooks/scheduler.rs`
- Purchase-token hashing: `src/application/verify_purchase.rs`, `src/webhooks/ingress.rs`, `src/webhooks/forwarding.rs`, `src/webhooks/scheduler.rs`
- PII regression: `tests/pii-leak-regression-test.sh`

## Verdict

Phase 1 is locally validated except for DB-down failure behavior, production log aggregation/alert routing, and real external provider outage behavior.
