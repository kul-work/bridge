# Bridge GPBI Tests

Comprehensive testing suite for validating the Google Play Billing system integration in the Bridge central payment service.

## Location
`tests/gpbi/`

## Migrated Tests

| Test ID | Name | Description |
| :--- | :--- | :--- |
| **SUB-01** | Initial Purchase | Verifies `/api/v1/verify-purchase` and record creation. |
| **SUB-02** | Renewal | Verifies extension of `current_period_end` on renewal webhook. |
| **SUB-03** | Cancellation | Verifies status transition to `cancelled` on cancellation webhook. |
| **SUB-05** | Expiration | Verifies status transition to `expired` on expiration webhook. |
| **SUB-09** | Revocation (Refund) | Verifies status `revoked` and payment `refunded` on voided purchase webhook. |
| **WHK-01** | Invalid Signature | Verifies rejection of webhooks with bad authorization headers. |

## Usage

### Prerequisites
1.  **Bridge Server Running**: `cargo run`
2.  **Mock APIs Enabled**: Set `MOCK_EXTERNAL_APIS=true` in `.env`.
3.  **Signature Verification (Optional)**: If testing security (`WHK-01`), set `verify_webhook_signature: true` in `pay.provider_configs` for the test app. Otherwise, set to `false` for easy local simulation.

### Running a Test
```bash
cd tests/gpbi
# Run SUB-01
./test-sub-01.sh
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
