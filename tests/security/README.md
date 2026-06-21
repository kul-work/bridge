# Bridge Security Tests

Shell security tests for a running Bridge backend. These tests verify
cross-app tenant isolation at the API level: one app's API key must not be
able to read or mutate another app's payment, subscription, or user data.

Bridge's tenant isolation defense is `app_id`-scoped queries plus PostgreSQL
Row Level Security (`set_local_app_id` + `tenant_isolation_*` policies).
These tests exercise the real API path end-to-end, confirming that the
combined handler + RLS scoping holds across every payment-facing endpoint.
They do not isolate RLS from handler-level `WHERE app_id = $1` filters — a
direct RLS probe using the runtime `bridge_app` role would be needed for
that.

## Prerequisites

- **Bridge backend must be running** at `BRIDGE_API_URL` (default
  `http://localhost:5566`).
- Two registered apps exist in `pay.apps` with enabled API keys
  (default: `hiha` and `household`).
- `curl`, `jq`, `psql`, and `bash` are available.
- `globals.cfg` is configured, optionally via `tests/security/.env`.
- The `.env` file must contain:
  - `BRIDGE_APP_A_API_KEY` — API key for the data-owner app (default: `hiha`)
  - `BRIDGE_APP_B_API_KEY` — API key for the cross-app attacker (default: `household`)

## Running tests

```bash
cd tests/security
bash test-runner.sh
```

Optional scopes:

```bash
bash test-runner.sh --scope tenant
```

The `tenant` scope runs cross-app tenant isolation tests.

## Test list

| Script | Scope | What it verifies |
|---|---|---|
| `test-cross-app-read.sh` | `tenant` | App B's API key cannot read or mutate App A's data. App A (hiha) is seeded with a subscription + payment + webhook record for a disposable test user via direct DB insert. App B (household) then attempts: `GET /payments`, `GET /subscriptions`, `GET /subscriptions/:id`, `GET /users/:id/subscription-status`, `GET /users/:id/data-export` (including webhook records), and `POST /subscriptions/:id/cancel`. Every read must return empty results (not App A's data); the cancel must return 404 `subscription_not_found`. After the cancel attempt, App A's subscription is re-verified durably (status, auto_renewing, cancellation_initiated_at, revoked_at, version) and re-read via App A's API key. API key identity is verified by matching key prefix to expected app_id before any test runs. App A sanity checks confirm the seeded data is visible to App A before isolation assertions run. |

## Cleanup

Each test creates disposable data with IDs prefixed `security_` and removes
all seeded subscriptions, payments, and webhook records on exit via direct
`DELETE` scoped to the specific `app_id` and seeded markers.
