# RLS & DB Rights Audit - 2026-06-19

Scope: Bridge `pay` schema in the local `appgen` database from `.env`, checked against the current Bridge code and migrations.

This is not a Railway/production audit. This note lists only currently open, production-relevant issues. Fixed, invalid, and dev-only findings are intentionally omitted.

## Audit Target

Local `.env` target:

```env
DATABASE_URL=postgresql://bridge_app:postgres@localhost/appgen
ADMIN_DATABASE_URL=postgresql://bridge_admin:postgres@localhost/appgen
```

Relevant applied migrations:

- `94` - `harden bridge app privileges`
- `95` - `harden bootstrap select policies`

## Current Findings

### Critical - `apps` Has No RLS And Stores Tenant Secrets

`pay.apps` holds `webhook_callback_secret` and `webhook_ingress_token`, but RLS is disabled and `bridge_app` has broad access.

Risk:

- `bridge_app` can read every tenant's webhook credentials.
- Those credentials are enough to forge callbacks or inject fake provider webhooks across apps.
- Runtime write access is also too broad; app provisioning should stay admin-owned.

Fix:

```sql
ALTER TABLE pay.apps ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_apps_bootstrap_select ON pay.apps
    FOR SELECT
    TO bridge_app
    USING (pay.current_app_id() IS NULL OR id = pay.current_app_id());

REVOKE INSERT, UPDATE, DELETE ON pay.apps FROM bridge_app;
```

Keep only the runtime access that is actually needed for app/bootstrap lookup.

### High - `api_keys` Bootstrap Policy Still Reads All Keys When Context Is Unset

`tenant_isolation_api_keys_bootstrap_select` uses:

```sql
pay.current_app_id() IS NULL OR app_id = pay.current_app_id()
```

That is needed for initial API-key authentication, but it means any direct pool query to `api_keys` without tenant context sees all app keys.

Fix: replace the blanket bootstrap policy with a narrow `SECURITY DEFINER` lookup function that returns only the columns needed for authentication by key hash. This should follow the same pattern as migration `95`'s webhook bootstrap helpers.

### High - RLS Is Not Forced On Tenant Tables

`relforcerowsecurity = false` on the RLS-enabled tenant tables. Runtime is not the owner today, so this is not an active tenant leak, but `FORCE ROW LEVEL SECURITY` prevents a future owner change from silently bypassing RLS.

Fix:

```sql
ALTER TABLE pay.api_keys FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.checkout_idempotency FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.fraud_prevention FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.payments FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.provider_configs FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.subscriptions FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_delivery FORCE ROW LEVEL SECURITY;
ALTER TABLE pay.webhook_provider FORCE ROW LEVEL SECURITY;
```

### Medium - `v_data_retention_stats` Bypasses Tenant Semantics

`pay.v_data_retention_stats` aggregates `webhook_provider` and `fraud_prevention` without an `app_id` filter. `bridge_app` can read cross-tenant counts/dates through the view even though the base tables are RLS-protected.

Risk is lower because this is aggregate metadata, not raw tenant rows, but it is still a tenancy leak channel.

Fix: either add `WHERE app_id = pay.current_app_id()` to each branch of the view, or revoke `bridge_app` access and keep it admin-only.

### Medium - Remaining Broad Runtime DML Grants

Apart from the separately called-out `apps` and `v_data_retention_stats` issues, `bridge_app` still has broad table privileges on many runtime objects.

This is not the same as the `apps` secret leak and not the same as the view leak. This item is about least-privilege cleanup after mapping actual runtime SQL.

Objects to map and narrow include:

- `pay.api_keys`
- `pay.checkout_idempotency`
- `pay.fraud_prevention`
- `pay.payments`
- `pay.provider_configs`
- `pay.subscriptions`
- `pay.webhook_delivery`
- `pay.webhook_provider`

Fix: map runtime SQL by table and reduce grants table-by-table. Do not blindly revoke until checkout, purchase verification, subscription actions, webhook ingress, forwarding, retry, reconciliation, and admin reads are covered.

### Medium - Default Privileges Reintroduce Broad Runtime Access

Default privileges in schema `pay` still grant broad future access to `bridge_app`.

Observed shape:

```text
future tables:    bridge_app gets SELECT, INSERT, UPDATE, DELETE
future functions: bridge_app gets EXECUTE
future sequences: bridge_app gets USAGE / SELECT
```

Fix: tighten default privileges after deciding how future migrations will grant required runtime access explicitly.

### Low/Medium - PUBLIC Database Rights

The `appgen` database grants `PUBLIC`:

```text
CONNECT
TEMPORARY
```

This is PostgreSQL's common default. It is not an active tenant RLS leak, but strict production hardening can replace it with explicit grants to known roles.

## Recommended Action Order

1. Enable RLS on `apps` and remove runtime write grants there.
2. Replace the `api_keys` blanket bootstrap policy with a narrow `SECURITY DEFINER` auth lookup.
3. Force RLS on the tenant tables.
4. Fix or restrict `v_data_retention_stats`.
5. Map runtime SQL and reduce remaining broad `bridge_app` table grants.
6. Tighten default privileges for future objects.
7. Optionally tighten `PUBLIC CONNECT/TEMPORARY` if all legitimate database roles are known.

## Verification Checklist

After applying future hardening migrations in another environment, verify:

```sql
SELECT tablename, policyname
FROM pg_policies
WHERE schemaname = 'pay'
ORDER BY tablename, policyname;
```

As `bridge_app` with no app context, tenant data tables should return zero rows except whichever narrow bootstrap lookup is intentionally used for authentication.

As `bridge_app` with a valid `bridge.current_app_id`, tenant tables should return only rows for that app.
