# Code Review: `092b8ec9e4d4ed632f99e59bd30698988ee126a1`

Commit: `TESTS (security): Add cross-app tenant isolation test for Bridge`

Verdict: **Needs changes before relying on this security suite.**

Risk areas reviewed:

- Cross-app tenant isolation
- API-key-to-app scoping
- PostgreSQL RLS coverage claims
- Payment/subscription read paths
- Subscription cancel mutation path
- Live DB and provider side effects

## Summary

The commit is a useful first API-level smoke test, but it does not yet prove the stated security claims. The current suite can pass with a misconfigured attacker API key, does not verify durable state after the cross-app cancel attempt, overclaims RLS coverage, and has a possible real-provider side-effect path if tenant lookup regresses.

## Blocking findings

### 1. App B is never proven to be App B

The script resolves `APP_A_ID` and `APP_B_ID`, but only checks that `BRIDGE_APP_A_API_KEY` and `BRIDGE_APP_B_API_KEY` are non-empty.

Evidence:

- `tests/security/test-cross-app-read.sh:126-133` resolves app IDs and checks only that both API key variables are set.
- `tests/security/test-cross-app-read.sh:165-167` seeds only App A data.
- `tests/security/test-cross-app-read.sh:197-210` expects App B to see empty payment results.
- `tests/security/test-cross-app-read.sh:218-245` expects App B to see empty/no-premium subscription results.

Why this matters:

If `BRIDGE_APP_B_API_KEY` belongs to any third app with no data, the suite passes while reporting that the `household` app was tested against `hiha` data.

Required fix:

- Verify each API key authenticates as the expected app ID, or
- Seed App B positive-control data for the same `external_user_id` with distinct markers, then assert:
  - App A sees only App A markers.
  - App B sees only App B markers.
  - App B never sees App A markers.

### 2. The cancel/write isolation claim is not verified durably

The cross-app cancel check only asserts the HTTP response, then cleanup immediately removes the seeded rows.

Evidence:

- `tests/security/test-cross-app-read.sh:277-286` asserts `404` and `subscription_not_found` for App B cancel.
- `tests/security/test-cross-app-read.sh:292-299` cleans up immediately afterward.
- The real cancel flow loads a subscription, calls the provider, and then mutates local subscription state in `src/application/subscription_actions.rs:49-86`.

Why this matters:

A regression could mutate App A's subscription and still return an error response. The current test would miss that data corruption.

Required fix:

Before cleanup, query App A's seeded subscription by `(app_id, external_user_id, subscription_id, provider)` and assert at minimum:

- `status = 'active'`
- `auto_renewing = true`
- `cancellation_initiated_at IS NULL`
- `revoked_at IS NULL`
- Preferably `version` unchanged

Also re-read through App A's API key to prove App A still sees an active subscription after App B's failed cancel attempt.

### 3. The cancel test can hit a real provider if tenant lookup regresses

The seeded subscription uses `provider='google_play'`, and the cancel flow calls the provider before mutating local DB state.

Evidence:

- `tests/security/test-cross-app-read.sh:165` inserts a Google Play subscription fixture.
- `src/application/subscription_actions.rs:69-81` retrieves provider config and calls `provider_api::cancel_subscription(...)` before local state mutation.

Why this matters:

If cross-app lookup regresses, this security test can attempt a real Google Play cancellation or fail on provider configuration instead of producing a clean tenant-isolation failure. This is especially risky because the suite is documented as a live-backend shell test.

Required fix:

- Require mock provider mode or an explicit destructive-test opt-in before running the cancel mutation test, or
- Use a test-only provider setup for the mutation case that cannot call external services.

### 4. The README overclaims RLS coverage

The README says the tests confirm both RLS and handler-level `auth.app_id` scoping. The current suite mainly verifies API-level behavior through handlers that pass `auth.app_id` into app-scoped queries.

Evidence:

- `tests/security/README.md:7-10` claims the tests confirm RLS and handler-level scoping.
- `migrations/91_fix_rls_current_app_id_cast.sql:47-66` defines tenant policies for `pay.payments`, `pay.provider_configs`, and `pay.subscriptions` for role `bridge_app`.
- `migrations/91_fix_rls_current_app_id_cast.sql:87-95` forces RLS on tenant-scoped tables.
- The test connects directly as `BRIDGE_DB_USER`, defaulting to `bridge_admin` in `tests/security/globals.cfg:18-24`, and does not prove the running backend uses the runtime RLS role.

Why this matters:

If RLS were disabled or bypassed while explicit `WHERE app_id = $1` query filters remained correct, the current suite could still pass.

Required fix:

Either narrow the README claim to “API-level tenant isolation smoke test,” or add a direct RLS smoke check using the runtime role:

1. Connect as `bridge_app`.
2. Set `bridge.current_app_id` to App B.
3. Query for App A markers without an `app_id` predicate and assert 0 rows.
4. Set `bridge.current_app_id` to App A.
5. Query for the same marker and assert 1 row.

### 5. `data-export` does not test webhook-record isolation

The `data-export` endpoint includes webhook records, but the test only validates subscriptions and payments.

Evidence:

- `src/handlers/users.rs:63-84` returns `webhook_records` in the data export payload.
- `src/db/webhooks.rs:307-320` lists webhook records through an app-scoped query.
- `tests/security/test-cross-app-read.sh:253-268` checks only `.subscriptions`, `.payments`, and marker strings for subscription/payment IDs.

Why this matters:

A webhook-record leak through data export would not be caught by the current test.

Required fix:

- Seed an App A `pay.webhook_provider` marker tied to the seeded subscription/payment token.
- Assert App B export has `webhook_records | length == 0` and does not contain the App A marker.
- If App B positive-control data is added, also seed an App B webhook marker and assert App B sees only its own webhook marker.

## Additional findings

### 6. The suite omits the protected subscription detail endpoint

The README claims every payment-facing endpoint is covered, but the suite does not cover `GET /api/v1/subscriptions/:subscription_id`.

Evidence:

- `src/main.rs:141-143` exposes `GET /subscriptions`, `GET /subscriptions/:subscription_id`, and `POST /subscriptions/:subscription_id/cancel` as separate protected routes.
- `tests/security/README.md:44` lists the tested routes and omits the detail endpoint.

Required fix:

Add:

- App A sanity: `GET /api/v1/subscriptions/$SUBSCRIPTION_ID?external_user_id=$TEST_USER_ID&provider=google_play` returns `200` and the App A marker.
- App B isolation: same request with App B key returns `404 subscription_not_found` and no App A marker.

Alternatively, narrow the README claim so it does not say every payment-facing endpoint is covered.

### 7. Live DB writes need a stronger safety gate and narrower cleanup

The script writes directly to the configured database and deletes by `external_user_id` across all apps.

Evidence:

- `tests/security/test-cross-app-read.sh:165-167` inserts live rows directly into `pay.subscriptions` and `pay.payments`.
- `tests/security/test-cross-app-read.sh:141-147` cleanup deletes from `pay.payments` and `pay.subscriptions` by `external_user_id` only.

Why this matters:

The generated user ID is unique, so collision risk is low, but this is still a destructive live-DB test with no explicit “I know this is a test database” guard.

Required fix:

- Require an opt-in such as `BRIDGE_SECURITY_TEST_ALLOW_DB_WRITES=1`.
- Refuse obvious production/staging API URLs or DB hosts unless explicitly overridden.
- Cleanup by exact seeded markers plus app IDs, for example `app_id`, `subscription_id`, `provider_transaction_id`, and `external_user_id`.

## Positive notes

- The test does include App A sanity checks, so the App B empty-result assertions are not completely vacuous for App A seeding.
- The generated fixture IDs include a timestamp and process ID, which reduces accidental collision risk.
- `git diff --check` passed for the reviewed commit.

## Review evidence checked

- Diff range: `092b8ec9e4d4ed632f99e59bd30698988ee126a1^..092b8ec9e4d4ed632f99e59bd30698988ee126a1`
- Added files:
  - `tests/security/README.md`
  - `tests/security/globals.cfg`
  - `tests/security/test-cross-app-read.sh`
  - `tests/security/test-runner.sh`
- Relevant existing files:
  - `src/main.rs`
  - `src/handlers/payments.rs`
  - `src/handlers/subscriptions.rs`
  - `src/handlers/subscriptions_actions.rs`
  - `src/handlers/users.rs`
  - `src/application/subscription_actions.rs`
  - `src/db/webhooks.rs`
  - `migrations/91_fix_rls_current_app_id_cast.sql`

## Checks run

- `git diff --check 092b8ec9e4d4ed632f99e59bd30698988ee126a1^ 092b8ec9e4d4ed632f99e59bd30698988ee126a1`

The live security suite was not run because it requires a running backend, DB credentials, app API keys, and currently has a possible real-provider side-effect path.
