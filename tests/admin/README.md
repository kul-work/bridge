# Bridge Admin Interface Test Suite

Automated test suite for the Bridge admin interface (manual webhook retry, scheduler triggers, admin authentication, CSP, and mutation audit logging).

These scenarios correspond to the test specifications in [BRIDGE_ADMIN_TESTPLAN.md](../../docs/BRIDGE_ADMIN_TESTPLAN.md).

## Directory Structure

```text
tests/admin/
├── README.md                 # This file
├── globals.cfg               # Shared test configuration & DB queries
├── mock_clerk.py             # Mock Clerk JWT & JWKS server in Python
├── test-runner.sh            # Master runner script orchestrating the suite
├── run-all-admin-tests.sh    # Test suite runner executing individual test scripts
├── cleanup-runner.sh         # Cleanup script for seeded test database entries
├── test-whk-01.sh            # Scenario: Dead-Lettered Webhook Manually Retried
├── test-whk-02.sh            # Scenario: Admin Retry Does Not Reopen Already-Forwarded Delivery
├── test-job-01.sh            # Scenario: Manual trigger-jobs Is Idempotent
├── test-auth-01.sh           # Scenario: Admin Endpoint Rejects JWT Without Matching azp
├── test-auth-02.sh           # Scenario: Admin Endpoint Rejects JWT From Wrong Issuer
├── test-auth-03.sh           # Scenario: Admin Endpoint Rejects Missing Bearer Token
├── test-csp-01.sh            # Scenario: CSP Blocks External Scripts, Allows Clerk/Captcha Styles
└── test-audit-01.sh          # Scenario: Admin Actions Are Recorded in Audit Log
```

## Prerequisites

1. **Bridge Server Running**:
   To test authentication and token validation, configure the admin Clerk test values in `tests/admin/.env`:
   ```bash
   ADMIN_CLERK_FRONTEND_API=http://localhost:5577
   ADMIN_CLERK_AUTHORIZED_PARTIES=https://admin.bridge.example.com
   ADMIN_CLERK_ORG_ID=org_test
   ```
   Then start Bridge from the repository root with `cargo run`. In local/test environments, Bridge loads these admin Clerk test values from `tests/admin/.env` when they are not already set in the process environment.
2. **Database Access**:
   Ensure `psql` is in your environment PATH and configured correctly in `tests/cti/.env` or via default settings (localhost:5432, user: bridge_admin).
3. **App Registered**:
   The database should have at least one enabled app in the `pay.apps` table (e.g. slug `hiha`).

## Usage

To run the entire suite:

```bash
cd tests/admin
./test-runner.sh --clear
```

The script will automatically:
1. Start the `mock_clerk.py` helper in the background.
2. Verify the Bridge server is online.
3. Clean up any existing stale test data.
4. Run all scenario test scripts.
5. Aggregate reports into `admin-suite-summary.json` and print the summary.
6. Stop the mock Clerk server cleanly.
