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

### Table rights

`bridge_app` has broad `SELECT`, `INSERT`, `UPDATE`, and `DELETE` rights across `pay` tables and views, including:

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

### Default privileges

Default privileges grant broad rights on future objects:

- For future tables in `pay`, `bridge_app` receives `SELECT`, `INSERT`, `UPDATE`, `DELETE`.
- For future sequences in `pay`, `bridge_app` receives `USAGE`, `SELECT`.
- For future functions in `pay`, `bridge_app` receives `EXECUTE`.

This may be intentional for operational convenience, but it keeps future runtime privileges broad by default.

### Database rights

The `appgen` database grants `PUBLIC`:

- `CONNECT`
- `TEMPORARY`

This is PostgreSQL's common default, but for strict production hardening it can be revoked and replaced with explicit role grants.

## Findings

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

### Finding 5 — Admin and inspection roles bypass RLS

Severity: expected but sensitive.

`bridge_admin` and `agent` have `BYPASSRLS`. This is acceptable for migrations/admin/inspection if credentials are tightly controlled. The runtime `bridge_app` role does not bypass RLS.

## Proposed next migration

Recommended bootstrap-policy hardening migration after the code paths are updated:

```sql
DROP POLICY IF EXISTS tenant_isolation_provider_configs_bootstrap_select
ON pay.provider_configs;

DROP POLICY IF EXISTS tenant_isolation_webhook_provider_bootstrap_select
ON pay.webhook_provider;

DROP POLICY IF EXISTS tenant_isolation_webhook_delivery_bootstrap_select
ON pay.webhook_delivery;
```

## Verification recommended after bootstrap-policy hardening

After applying the migration, verify:

1. App startup still runs migrations using `bridge_admin`.
2. API-key authentication still works.
3. Checkout/payment/subscription/webhook flows still work.
4. A `bridge_app` session without `bridge.current_app_id` cannot read provider/webhook tenant rows.
5. A `bridge_app` session with `bridge.current_app_id` can still read/write only that tenant's rows.

## Bottom line

No evidence was found that normal tenant-scoped flows leak rows when `bridge.current_app_id` is correctly set.

However, the current DB posture has real hardening bugs/footguns:

- bootstrap policies that expose cross-tenant reads when app context is missing,
- broad runtime DML grants.

The recommended approach is to update the bootstrap-dependent code paths, apply the bootstrap-policy migration above, then do a least-privilege pass after mapping actual runtime SQL usage.
