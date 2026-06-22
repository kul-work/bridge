# Bridge CTI Tests

Contract & Tenant Isolation test suite for Bridge. Verifies cross-app data isolation (RLS enforcement) and Bridge API endpoint shape conformance against downstream apps.

## Location
`tests/cti/`

## Directory Structure

```text
tests/cti/
├── .env                        # Local environment secrets (not in git)
├── globals.cfg                 # Global test configuration (two-app setup)
├── test-runner.sh              # Main entry point (not yet implemented)
├── run-all-iso-tests.sh        # Isolation suite runner
├── run-all-contract-tests.sh   # Contract suite runner
├── test-iso-*.sh               # Cross-app isolation test scripts
├── test-contract-*.sh          # Bridge endpoint shape conformance tests
├── cleanup-all-*.sh            # Cleanup scripts per suite
├── cleanup-runner.sh           # Master cleanup runner
└── *report.json                # JSON report output from last run
```

## Key Test Suites

| Suite | Runner Script | Description |
| :--- | :--- | :--- |
| **Isolation** | `run-all-iso-tests.sh` | Cross-app RLS enforcement for subscriptions, payments, webhook deliveries, ingress tokens, and idempotency keys |
| **Contract** | `run-all-contract-tests.sh` | Bridge API endpoint shape conformance (checkout, register, verify, status, callbacks, email lookup) |

## Tests Highlights

| Test ID | Name | Description |
| :--- | :--- | :--- |
| **ISO-01** | Subscription Visibility | App B cannot see App A's subscription data (RLS enforced). |
| **ISO-02** | Payment History Isolation | App B cannot see App A's payments (RLS enforced). |
| **ISO-03** | Webhook Delivery Isolation | App B cannot see App A's webhook delivery state (RLS enforced). |
| **ISO-04** | Ingress Token Resolution | Webhook ingress token resolves to App A only; cross-app signature mismatch rejected. |
| **ISO-05** | Idempotency Key Scoping | Checkout idempotency keys are per-app, not global. |
| **CONTRACT-01** | Checkout Endpoint Shape | `POST /api/v1/payment/checkout` returns correct response shape. |
| **CONTRACT-02** | Purchase Registration Shape | `POST /api/v1/purchase/register` returns correct response shape. |
| **CONTRACT-03** | Verify Purchase Shape | `POST /api/v1/verify-purchase` returns correct response shape. |
| **CONTRACT-04** | Subscription Status Shape | `GET /api/v1/users/{id}/subscription-status` returns correct response shape. |
| **CONTRACT-05** | Signed Callback Delivery | Bridge forwards signed callbacks with X-Pay-Signature headers. |
| **CONTRACT-06** | Signed Email Lookup | Internal email lookup endpoint is HMAC-guarded. |

## Prerequisites

1. **Bridge Server Running**: `cargo run` with `MOCK_EXTERNAL_APIS=true`
2. **Two Apps Registered**: Both App A (e.g., `hiha`) and App B (e.g., `household`) must exist in `pay.apps` with `enabled=true`
3. **API Keys**: Set `APP_A_API_KEY` and `APP_B_API_KEY` in `.env` (they cannot be fetched from DB)
4. **Database Access**: `psql` connectivity to the `appgen` database

## Usage

```bash
cd tests/cti

# Run isolation suite
./run-all-iso-tests.sh

# Run contract suite
./run-all-contract-tests.sh

# Run a single test
./test-iso-01.sh

# Clean up all test data
./cleanup-runner.sh
```

## Design Note

The isolation tests verify Bridge's multi-tenant RLS enforcement at the database level — the highest-risk regression class for shared payment infrastructure. The contract tests verify endpoint shapes match what downstream apps (HouseHold, HiHa) expect, serving as the staging acceptance counterpart to local mock tests.

## Testplan Reference

- [BRIDGE_CONTRACT_TESTPLAN.md](../../docs/BRIDGE_CONTRACT_TESTPLAN.md)
- [BRIDGE_ADMIN_TESTPLAN.md](../../docs/BRIDGE_ADMIN_TESTPLAN.md)