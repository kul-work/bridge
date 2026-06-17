# RLS & DB Rights Check — 2026-06-17

## Scope

Checked the live PostgreSQL `appgen` database for Bridge's `pay` schema using metadata queries and limited runtime probes as `bridge_app`.

No application code was changed during this check.

## Roles observed

| Role | Login | Bypass RLS | Notes |
|---|---:|---:|---|
| `bridge_app` | yes | no | Runtime application role. RLS applies. |
| `bridge_admin` | yes | yes | Admin/migration role. Bypasses RLS. |
| `agent` | yes | yes | Inspection role used by MCP. Bypasses RLS. |

No memberships were observed between `agent`, `bridge_app`, and `bridge_admin`.

## RLS status

RLS is enabled on all tenant-scoped tables that have `app_id`:

- `pay.api_keys`
- `pay.checkout_idempotency`
- `pay.fraud_prevention`
- `pay.payments`
- `pay.provider_configs`
- `pay.subscriptions`
- `pay.webhook_delivery`
- `pay.webhook_provider`

RLS is not enabled on non-tenant/global tables:

- `pay.apps`
- `pay._sqlx_migrations`

This shape is broadly reasonable: tenant tables are RLS-protected, global/app registry and migration metadata are not.

## Policies observed

Most tenant policies use this expected shape:

```sql
app_id = pay.current_app_id()
```

with matching `WITH CHECK` for writes.

The following bootstrap `SELECT` policies also exist:

- `tenant_isolation_api_keys_bootstrap_select` on `pay.api_keys`
- `tenant_isolation_provider_configs_bootstrap_select` on `pay.provider_configs`
- `tenant_isolation_webhook_provider_bootstrap_select` on `pay.webhook_provider`
- `tenant_isolation_webhook_delivery_bootstrap_select` on `pay.webhook_delivery`

Their shape is:

```sql
pay.current_app_id() IS NULL OR app_id = pay.current_app_id()
```

This allows `bridge_app` to read all rows from those tables when `bridge.current_app_id` is unset.

## Runtime RLS probe

As `bridge_app`, with no app context set:

- `pay.current_app_id()` returned `NULL`.
- `pay.apps` returned both apps.
- Bootstrap-protected tables returned cross-app rows.

With `bridge.current_app_id = '43bd7125-87eb-4136-9605-6c5e524f1ab0'` (`hiha`):

- `api_keys`: 1 row
- `provider_configs`: 2 rows
- `webhook_provider`: 1 row
- `webhook_delivery`: 0 rows

With `bridge.current_app_id = '7eb51d51-af05-43d0-8a6a-41d772d6d953'` (`household`):

- `api_keys`: 1 row
- `provider_configs`: 2 rows
- `payments`: 1 row
- `subscriptions`: 1 row
- `webhook_provider`: 5 rows
- `webhook_delivery`: 3 rows
- `fraud_prevention`: 28 rows

Conclusion: tenant filtering works when `bridge.current_app_id` is set.

## Application context handling

The application sets the RLS app context via `set_local_app_id` in `src/db/database.rs`:

```rust
SELECT set_config('bridge.current_app_id', $1, true)
```

The third argument is `true`, so the setting is local to the current transaction.

References to `set_local_app_id` were found in:

- `src/db/api_keys.rs`
- `src/db/checkout_idempotency.rs`
- `src/db/payments.rs`
- `src/db/subscriptions.rs`
- `src/db/users.rs`
- `src/db/webhooks.rs`
- `src/ports/helpers.rs`

This is the correct general pattern, but any query to bootstrap-protected tables before setting app context can read across tenants.

## DB rights observed

### Schema rights

`pay` schema grants:

- `bridge_app`: `USAGE`, `CREATE`
- `bridge_admin`: `USAGE`, `CREATE`
- `postgres`: `USAGE`, `CREATE`
- `agent`: `USAGE`

`bridge_app` having `CREATE` on `pay` is over-privileged for a runtime role.

This was confirmed with a rolled-back transactional probe:

```sql
BEGIN;
CREATE TABLE pay._rls_rights_probe(id int);
ROLLBACK;
```

The create succeeded before rollback.

### Table rights

`bridge_app` has broad `SELECT`, `INSERT`, `UPDATE`, and `DELETE` rights across `pay` tables and views, including:

- `pay._sqlx_migrations`
- `pay.apps`
- `pay.api_keys`
- `pay.checkout_idempotency`
- `pay.fraud_prevention`
- `pay.payments`
- `pay.provider_configs`
- `pay.subscriptions`
- `pay.v_data_retention_stats`
- `pay.webhook_delivery`
- `pay.webhook_provider`

RLS reduces the blast radius on tenant tables, but this is not least-privilege.

`bridge_app` should not need access to `_sqlx_migrations`; migrations are run with `bridge_admin` when `ADMIN_DATABASE_URL` is configured.

### Default privileges

Default privileges grant broad rights on future objects:

- For future tables in `pay`, `bridge_app` receives `SELECT`, `INSERT`, `UPDATE`, `DELETE`.
- For future sequences in `pay`, `bridge_app` receives `USAGE`, `SELECT`.
- For future functions in `pay`, `bridge_app` receives `EXECUTE`.

This may be intentional for operational convenience, but it keeps future runtime privileges broad by default.

### Function rights

Functions observed in `pay`:

- `pay.current_app_id()`
- `pay.cleanup_old_webhook_provider()`
- `pay.cleanup_purged_fraud_prevention()`

All showed `PUBLIC EXECUTE`, plus explicit grants to `bridge_admin` and `bridge_app`.

`PUBLIC` does not have `USAGE` on the `pay` schema, which limits exploitability, but the cleaner posture is to revoke `PUBLIC EXECUTE` and grant only the roles that need each function.

### Database rights

The `appgen` database grants `PUBLIC`:

- `CONNECT`
- `TEMPORARY`

This is PostgreSQL's common default, but for strict production hardening it can be revoked and replaced with explicit role grants.

## Findings

### Finding 1 — Runtime role can create objects in `pay`

Severity: medium/high hardening issue.

`bridge_app` can create tables/functions/etc. in the payment schema. Runtime roles should generally not have schema `CREATE` in an application-owned schema.

Recommended fix:

```sql
REVOKE CREATE ON SCHEMA pay FROM bridge_app;
```

Keep `USAGE`:

```sql
GRANT USAGE ON SCHEMA pay TO bridge_app;
```

### Finding 2 — Bootstrap policies allow cross-tenant reads when app context is unset

Severity: medium/high, depending on whether all relevant code paths reliably set app context before reads.

The bootstrap policy shape:

```sql
pay.current_app_id() IS NULL OR app_id = pay.current_app_id()
```

allows all rows to be read by `bridge_app` when `bridge.current_app_id` is unset.

This is probably necessary for API-key lookup, because the API key identifies the app before the app context is known. It is riskier on provider/webhook tables unless there is a proven bootstrap need.

Recommended fix:

- Keep bootstrap `SELECT` on `pay.api_keys` if needed for API-key authentication.
- Remove bootstrap `SELECT` from `pay.provider_configs`, `pay.webhook_provider`, and `pay.webhook_delivery` unless code proves it is required.

```sql
DROP POLICY IF EXISTS tenant_isolation_provider_configs_bootstrap_select
ON pay.provider_configs;

DROP POLICY IF EXISTS tenant_isolation_webhook_provider_bootstrap_select
ON pay.webhook_provider;

DROP POLICY IF EXISTS tenant_isolation_webhook_delivery_bootstrap_select
ON pay.webhook_delivery;
```

### Finding 3 — Runtime role has broad DML on all objects

Severity: medium hardening issue.

`bridge_app` has broad table privileges, including on `_sqlx_migrations`, `apps`, and `v_data_retention_stats`.

Recommended immediate fix:

```sql
REVOKE ALL ON TABLE pay._sqlx_migrations FROM bridge_app;
```

Recommended later fix: map actual runtime SQL operations and reduce grants table-by-table. Do this as a separate phase to avoid breaking live flows blindly.

### Finding 4 — Functions are executable by `PUBLIC`

Severity: low/medium hardening issue.

`PUBLIC EXECUTE` exists for `pay.current_app_id()` and cleanup functions. Schema `USAGE` limits practical access, but explicit grants are cleaner.

Recommended fix:

```sql
REVOKE EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION pay.current_app_id() FROM PUBLIC;

GRANT EXECUTE ON FUNCTION pay.current_app_id() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.current_app_id() TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() TO bridge_admin;
```

Only grant cleanup functions to `bridge_app` too if the runtime service actually calls them under the runtime role.

### Finding 5 — Admin and inspection roles bypass RLS

Severity: expected but sensitive.

`bridge_admin` and `agent` have `BYPASSRLS`. This is acceptable for migrations/admin/inspection if credentials are tightly controlled. The runtime `bridge_app` role does not bypass RLS.

## Proposed phase 1 migration

Recommended small first hardening migration:

```sql
REVOKE CREATE ON SCHEMA pay FROM bridge_app;

REVOKE ALL ON TABLE pay._sqlx_migrations FROM bridge_app;

DROP POLICY IF EXISTS tenant_isolation_provider_configs_bootstrap_select
ON pay.provider_configs;

DROP POLICY IF EXISTS tenant_isolation_webhook_provider_bootstrap_select
ON pay.webhook_provider;

DROP POLICY IF EXISTS tenant_isolation_webhook_delivery_bootstrap_select
ON pay.webhook_delivery;

REVOKE EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION pay.current_app_id() FROM PUBLIC;

GRANT EXECUTE ON FUNCTION pay.current_app_id() TO bridge_app;
GRANT EXECUTE ON FUNCTION pay.current_app_id() TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_old_webhook_provider() TO bridge_admin;
GRANT EXECUTE ON FUNCTION pay.cleanup_purged_fraud_prevention() TO bridge_admin;
```

## Verification recommended after phase 1

After applying the migration, verify:

1. App startup still runs migrations using `bridge_admin`.
2. API-key authentication still works.
3. Checkout/payment/subscription/webhook flows still work.
4. A `bridge_app` session without `bridge.current_app_id` cannot read provider/webhook tenant rows.
5. A `bridge_app` session with `bridge.current_app_id` can still read/write only that tenant's rows.
6. `bridge_app` cannot create objects in `pay`.
7. `bridge_app` cannot access or mutate `pay._sqlx_migrations`.

## Bottom line

No evidence was found that normal tenant-scoped flows leak rows when `bridge.current_app_id` is correctly set.

However, the current DB posture has real hardening bugs/footguns:

- runtime schema `CREATE` permission,
- bootstrap policies that expose cross-tenant reads when app context is missing,
- broad runtime DML grants,
- unnecessary `PUBLIC EXECUTE` on functions.

The recommended approach is to apply the small phase 1 migration above, then do a second least-privilege pass after mapping actual runtime SQL usage.
