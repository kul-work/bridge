# Bridge GPBI Tests

Comprehensive testing suite for validating the Google Play Billing system integration in the Bridge central payment service.

## Location
`tests/gpbi/`

## Directory Structure

```text
tests/gpbi/
├── .env                        # Local environment secrets (not in git)
├── globals.cfg                 # Global test configuration (URLs, tokens)
├── test-runner.sh              # Main entry point for running suites
├── run-all-*-tests.sh          # Suite runners (e.g., sub, otp, whk, err)
├── test-*.sh                   # Generic test scripts (sub, otp, acc, etc.)
├── cleanup-*.sh                # Cleanup scripts for specific entities
├── cleanup-nuclear.sh          # Resets all test data in DB
└── *report.json                # JSON report output from last run
```

## Key Test Suites

| Suite | Runner Script | Description |
| :--- | :--- | :--- |
| **Subscriptions** | `run-all-sub-tests.sh` | Core subscription lifecycle (Purchase, Renewal, Cancellation) |
| **One-Time Purchases** | `run-all-otp-tests.sh` | OTP verification and refund handling |
| **Webhooks** | `run-all-whk-tests.sh` | Ingress validation, signature checks, and idempotency |
| **Dead-Letter** | `run-all-dlq-tests.sh` | Dead-letter exhaustion and admin retry recovery |
| **Account Linking** | `run-all-acc-tests.sh` | Identity mapping and account reconciliation |
| **Error Handling** | `run-all-err-tests.sh` | Network failures, invalid data, and provider API errors |

## Tests Highlights

| Test ID | Name | Description |
| :--- | :--- | :--- |
| **SUB-01** | Initial Purchase | Verifies `/api/v1/verify-purchase` and record creation. |
| **OTP-01** | One-Time Purchase | Verifies purchase verification, payment record creation, and provider acknowledgment. |
| **SUB-02** | Renewal | Verifies extension of `current_period_end` on renewal webhook. |
| **SUB-03** | Cancellation | Verifies status transition to `cancelled` on cancellation webhook. |
| **SUB-05** | Expiration | Verifies status transition to `expired` on expiration webhook. |
| **SUB-09** | Revocation (Refund) | Verifies status `revoked` and payment `refunded` on voided purchase webhook. |
| **WHK-01** | Invalid Signature | Verifies rejection of webhooks with bad authorization headers. |
| **WHK-02** | Duplicate Webhook | Verifies idempotent handling (returns success, but no duplicate record). |
| **DLQ-01** | Dead-Letter Exhaustion | Verifies `webhook_delivery` transitions to `dead_lettered=true` after 3 failed attempts. |
| **DLQ-02** | Admin Retry Recovery | Verifies dead-lettered delivery is reset and already-forwarded delivery is NOT reopened. |
| **NET-05** | Delivery Verification | Verifies Bridge → App callback forwarding success. |

## Usage

### Prerequisites
1.  **Bridge Server Running**: `cargo run`
2.  **Mock APIs Enabled**: Set `MOCK_EXTERNAL_APIS=true` in `.env`.
3.  **Signature Verification (Optional)**: If testing security (`WHK-01`), set `verify_webhook_signature: true` in `pay.provider_configs` for the test app. Otherwise, set to `false` for easy local simulation.

### Running Tests
```bash
cd tests/gpbi

# Run a single test
./test-sub-01.sh

# Run an entire suite
./run-all-sub-tests.sh

# Run all suites via the main runner
./test-runner.sh
```

## Configuration
Update `globals.cfg` with your local environment settings:
- `APP_URL`: Bridge server URL (default: `http://localhost:3000`)
- `API_KEY`: App API key for authorized requests.
- `WEBHOOK_INGRESS_TOKEN`: Unique token for the app's webhook ingress path.
- `DATABASE_URL`: Connection string to the `appgen` database.

## Design Note
These tests focus on **Bridge-level** responsibilities:
1.  Correct parsing and normalization of provider events.
2.  Accurate state management in the `pay` schema.
3.  Security and idempotency at the ingress boundary.

App-level reactions (e.g., updating user entitlements in the main app) should be tested via integration tests between the app and Bridge callbacks.
