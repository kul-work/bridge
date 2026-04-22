# Bridge Creem Tests

Comprehensive testing suite for validating the Creem Billing system integration in the Bridge central payment service.

## Location
`tests/creem/`

## Directory Structure

```text
tests/creem/
├── .env                        # Local environment secrets (not in git)
├── globals.cfg                 # Global test configuration (URLs, tokens)
├── test-runner.sh              # Main entry point for running suites
├── run-all-*-tests.sh          # Suite runners (e.g., sub, otp, whk, net, acc)
├── test-*.sh                   # Generic test scripts (sub, otp, net, whk, etc.)
├── cleanup-all-*.sh            # Batch cleanup scripts
├── cleanup-nuclear.sh          # Resets all test data in DB
└── *report.json                # JSON report output from last run
```

## Key Test Suites

| Suite | Runner Script | Description |
| :--- | :--- | :--- |
| **Subscriptions** | `run-all-sub-tests.sh` | Full lifecycle (Purchase, Renewal, Cancellation, Pause, Recovery) |
| **One-Time Purchases** | `run-all-otp-tests.sh` | OTP fulfillment and refund/failure handling |
| **Webhooks** | `run-all-whk-tests.sh` | Signature verification, normalization, and idempotency |
| **Access Control** | `run-all-acc-tests.sh` | Entitlement validation for active and blocked states |
| **Network & Errors** | `run-all-net-tests.sh` / `run-all-err-tests.sh` | Concurrent deliveries and malformed payload handling |

## Tests Highlights

| Test ID | Name | Description |
| :--- | :--- | :--- |
| **SUB-01** | Initial Subscription | Verifies `subscription.active` webhook and record creation. |
| **OTP-01** | Successful Purchase | Verifies `checkout.completed` for one-time payments. |
| **SUB-03** | Renewal | Verifies period extension on renewal webhooks. |
| **SUB-06** | Scheduled Cancel | Verifies `auto_renewing=false` and Grace Period access. |
| **WHK-01** | Valid Signature | Verifies HMAC-SHA256 signature acceptance. |
| **WHK-03** | Idempotency | Ensures duplicate webhooks don't create duplicate records. |
| **WHK-05** | Normalization | Maps provider-specific payloads to canonical Bridge states. |
| **NET-02** | Race Conditions | Tests concurrent deliveries of the same event. |

## Usage

### Prerequisites
1.  **Bridge Server Running**: Ensure the backend is active and reachable.
2.  **Environment Config**: Sourced via `globals.cfg`.
3.  **Database Access**: Connectivity for `psql` to validate state changes.

### Running Tests
```bash
cd tests/creem

# Run a single test
./test-sub-01.sh

# Run an entire suite
./run-all-sub-tests.sh

# Run all suites via the main runner
./test-runner.sh
```

## Configuration
Update `globals.cfg` with your local environment settings:
- `APP_URL`: Bridge server URL.
- `WEBHOOK_INGRESS_TOKEN`: Unique token for the app's Creem webhook ingress.
- `CREEM_WEBHOOK_SECRET`: The secret for HMAC validation.
- `BRIDGE_DB_*`: Database connection parameters.

## Design Note
These tests simulate **Creem Webhooks** and **API calls** to verify Bridge's internal logic:
1.  Deterministic parsing and normalization.
2.  State integrity in the `pay` database schema.
3.  Ingress boundary security (signature verification).
4.  Race condition and retried delivery handling.
