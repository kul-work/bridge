# Bridge Security Tests

Shell security tests for a running Bridge backend. These tests verify
cross-app tenant isolation: one app's API key must not be able to read or
mutate another app's payment, subscription, or user data.

Bridge's sole tenant isolation defense is `app_id`-scoped queries plus
PostgreSQL Row Level Security (`set_local_app_id` + `tenant_isolation_*`
policies). These tests exercise the real API path end-to-end, confirming
RLS and handler-level `auth.app_id` scoping hold across every
payment-facing endpoint.

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
| `test-cross-app-read.sh` | `tenant` | App B's API key cannot read or mutate App A's data. App A (hiha) is seeded with a subscription + payment for a disposable test user via direct DB insert. App B (household) then attempts: `GET /payments`, `GET /subscriptions`, `GET /users/:id/subscription-status`, `GET /users/:id/data-export`, and `POST /subscriptions/:id/cancel`. Every read must return empty results (not App A's data); the cancel must return 404 `subscription_not_found`. App A sanity checks confirm the seeded data is visible to App A before isolation assertions run. |

## Cleanup

Each test creates disposable data with IDs prefixed `security_` and removes
all seeded subscriptions and payments on exit via direct `DELETE FROM
pay.subscriptions` / `pay.payments`.
