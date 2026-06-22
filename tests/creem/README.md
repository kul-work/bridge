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
| **Dead-Letter** | `run-all-dlq-tests.sh` | Dead-letter exhaustion and admin retry recovery |
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
| **WHK-02** | Invalid Signature Rejection | Verifies that invalid signatures are rejected and DB state unchanged. Also checks verify_webhook_signature config. |
| **WHK-03** | Idempotency | Ensures duplicate webhooks don't create duplicate records. |
| **WHK-05** | Normalization | Maps provider-specific payloads to canonical Bridge states. |
| **WHK-06** | Verification Mode Override | Tests X-Webhook-Verification-Mode header bypass (requires MOCK_EXTERNAL_APIS=true). |
| **DLQ-01** | Dead-Letter Exhaustion | Verifies `webhook_delivery` transitions to `dead_lettered=true` after 3 failed attempts. |
| **DLQ-02** | Admin Retry Recovery | Verifies dead-lettered delivery is reset and already-forwarded delivery is NOT reopened. |
| **NET-02** | Race Conditions | Tests concurrent deliveries of the same event. |
| **NET-03** | Delivery Verification | Verifies Bridge → App callback forwarding success. |

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

### Signature Verification Control
Creem webhook signature verification can be controlled at multiple levels:
1. **DB config**: `verify_webhook_signature` in `pay.provider_configs.config` (defaults to `true`)
2. **Header override**: `X-Webhook-Verification-Mode: off/strict` — only works when `MOCK_EXTERNAL_APIS=true`
3. **Production**: Header override is ignored; only DB config applies

To run tests with signature bypass (local dev), set `MOCK_EXTERNAL_APIS=true` in the server's `.env`.

## Design Note
These tests simulate **Creem Webhooks** and **API calls** to verify Bridge's internal logic:
1.  Deterministic parsing and normalization.
2.  State integrity in the `pay` database schema.
3.  Ingress boundary security (signature verification).
4.  Race condition and retried delivery handling.
