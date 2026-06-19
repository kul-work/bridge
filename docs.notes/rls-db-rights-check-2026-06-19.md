# RLS & DB Rights Audit - 2026-06-19

Scope: Bridge `pay` schema in the local `appgen` database from `.env`, checked against the current Bridge code and migrations.

This note lists the current audit result after the bootstrap-policy hardening work. It does not repeat historical items that are fixed by the current migration set.

## Audit Target

Local `.env` target:

```env
DATABASE_URL=postgresql://bridge_app:postgres@localhost/appgen
ADMIN_DATABASE_URL=postgresql://bridge_admin:postgres@localhost/appgen
```

This is not a Railway/production audit.

## Migration State

The local database reports these relevant migrations as applied:

- `94` - `harden bridge app privileges`
- `95` - `harden bootstrap select policies`

## Summary

No active runtime tenant RLS leak was found.

The old broad bootstrap issue is fixed locally:

- `pay.provider_configs` no longer has the broad bootstrap `SELECT` policy.
- `pay.webhook_provider` no longer has the broad bootstrap `SELECT` policy.
- `pay.webhook_delivery` no longer has the broad bootstrap `SELECT` policy.
- Only `pay.api_keys` keeps bootstrap `SELECT`, which is intentional for API-key authentication.

Remaining items are production-relevant privilege-hardening concerns, not active tenant-row leaks:

1. `bridge_app` still has broad table DML grants on many `pay` objects.
2. Default privileges still grant broad future rights to `bridge_app`.
3. `PUBLIC` has database-level `CONNECT` and `TEMPORARY`.

## Current Good Baseline

- Runtime role `bridge_app` is subject to RLS.
- Tenant tables have RLS enabled.
- Tenant policies use `app_id = pay.current_app_id()` with matching write checks.
- `pay.provider_configs`, `pay.webhook_provider`, and `pay.webhook_delivery` no longer expose all rows when app context is unset.
- `pay._sqlx_migrations` is no longer granted to `bridge_app`.
- Audited `pay` functions no longer grant `EXECUTE` to `PUBLIC`.
- Narrow `SECURITY DEFINER` helpers exist for true pre-context webhook lookups.

## Finding 2 Recheck - Bootstrap Policy Leak

Status: fixed locally, assuming migration `95` is applied.

As `bridge_app` with no `bridge.current_app_id`, observed row counts were:

```text
api_keys             2
provider_configs     0
webhook_provider     0
webhook_delivery     0
payments             0
subscriptions        0
fraud_prevention     0
checkout_idempotency 0
```

That is the expected shape. `api_keys` remains readable for authentication bootstrap. Provider config and webhook tables now fail closed without app context.

Migration `95` replaces the broad provider/webhook bootstrap policies with narrow helper functions:

- `pay.get_webhook_provider_app_id_bootstrap(uuid)`
- `pay.get_webhook_delivery_app_id_bootstrap(uuid)`
- `pay.get_webhook_provider_bootstrap(uuid)`
- `pay.get_webhook_delivery_bootstrap(uuid)`
- `pay.webhook_delivery_exists_bootstrap(uuid)`

These functions are `SECURITY DEFINER`, owned by `bridge_admin`, and executable only by `bridge_app` and `bridge_admin`.

Residual note: `get_webhook_provider_bootstrap` and `get_webhook_delivery_bootstrap` can return one full row by UUID without app context. This is much narrower than the old all-rows bootstrap policy. A stricter future design could resolve only `app_id`, then read the row under normal RLS.

## Remaining Hardening Item 1 - Broad DML Grants

Severity: medium hardening issue.

`bridge_app` still has broad table privileges across many `pay` objects:

```text
SELECT, INSERT, UPDATE, DELETE
```

Observed on objects including:

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

RLS limits tenant-scoped rows, but this is still not least privilege. Do a runtime SQL mapping pass before narrowing these grants table-by-table.

## Remaining Hardening Item 2 - Default Privileges

Severity: medium hardening issue.

Default privileges in schema `pay` still grant broad future access to `bridge_app`.

Observed shape:

```text
future tables:    bridge_app gets SELECT, INSERT, UPDATE, DELETE
future functions: bridge_app gets EXECUTE
future sequences: bridge_app gets USAGE / SELECT
```

This can reintroduce broad runtime access on future objects even after current grants are tightened.

Recommended direction: tighten default privileges only after deciding how future migrations will grant required runtime access explicitly.

## Remaining Hardening Item 3 - PUBLIC Database Rights

Severity: low/medium hardening issue.

The `appgen` database grants `PUBLIC`:

```text
CONNECT
TEMPORARY
```

This is PostgreSQL's common default. It is not an active tenant RLS leak, but strict production hardening can replace it with explicit grants to known roles.

## Recommended Priority

1. Keep migration `95` applied before considering Finding 2 closed in any other environment.
2. Map runtime SQL and reduce `bridge_app` table grants.
3. Tighten default privileges so future objects do not regain broad runtime access.
4. Optionally tighten `PUBLIC CONNECT/TEMPORARY` if all legitimate database roles are known.

## Verification Checklist

After rebuilding or applying these migrations in another environment, verify:

```sql
SELECT tablename, policyname
FROM pg_policies
WHERE schemaname = 'pay'
  AND policyname LIKE '%bootstrap%'
ORDER BY tablename, policyname;
```

Expected: only `tenant_isolation_api_keys_bootstrap_select`.

As `bridge_app` with no app context, tenant data tables should return zero rows except `pay.api_keys`.

As `bridge_app` with a valid `bridge.current_app_id`, tenant tables should return only rows for that app.

---

## Re-Audit Findings - 2026-06-19 (Crush)

Fresh pass against the live local `appgen` database. Confirms the prior audit's "no active runtime tenant RLS leak" baseline for the 8 RLS-enabled tenant tables, and surfaces additional issues the prior pass did not call out. The prior sections above remain the historical record; findings below are the current state.

### Posture Snapshot

| Table | RLS | Forced | Owner | Scopes by |
|---|---|---|---|---|
| `apps` | NO | - | bridge_admin | none |
| `api_keys` | YES | NO | bridge_admin | `app_id = pay.current_app_id()` |
| `checkout_idempotency` | YES | NO | bridge_admin | `app_id` |
| `fraud_prevention` | YES | NO | bridge_admin | `app_id` |
| `payments` | YES | NO | bridge_admin | `app_id` |
| `provider_configs` | YES | NO | bridge_admin | `app_id` |
| `subscriptions` | YES | NO | bridge_admin | `app_id` |
| `webhook_delivery` | YES | NO | bridge_admin | `app_id` |
| `webhook_provider` | YES | NO | bridge_admin | `app_id` |
| `_sqlx_migrations` | NO | - | bridge_admin | - (no `bridge_app` grant) |

Tenancy model: `pay.current_app_id()` reads GUC `bridge.current_app_id`, set per-transaction via `set_config(..., true)` in `src/db/database.rs:80`. Transaction-local, so no pool-bleed risk as long as tenant queries run inside that tx. Policies target role `bridge_app` only.

### CRITICAL - `apps` has no RLS and stores tenant secrets

`pay.apps` holds `webhook_callback_secret` (signing secret for callbacks to client apps) and `webhook_ingress_token` (authenticates inbound provider webhooks), yet RLS is disabled and `bridge_app` holds `arwd` (full SELECT/INSERT/UPDATE/DELETE). Any code path running as `bridge_app` can read every tenant's webhook credentials - sufficient to forge callbacks or inject fake provider webhooks across all apps. Write access (`awd`) is also over-granted: the runtime only needs SELECT here for bootstrap lookups; app provisioning is admin-only.

This was not flagged in the prior audit, which listed `pay.apps` only under "broad DML grants" without noting the secret columns or the absent RLS.

Fix: enable RLS on `apps` with a policy mirroring `api_keys`' bootstrap pattern (`current_app_id() IS NULL OR app_id = current_app_id()`), and drop `bridge_app`'s INSERT/UPDATE/DELETE on `apps` - leave SELECT only. Provisioning stays on `bridge_admin`.

### HIGH - `api_keys` bootstrap policy still leaks all keys when GUC unset

`tenant_isolation_api_keys_bootstrap_select` is `current_app_id() IS NULL OR app_id = current_app_id()`. Required for initial auth, but it means any query hitting `api_keys` outside a tenant-scoped transaction sees every app's API keys. The transaction-local GUC in `database.rs:80` mitigates bleed, but this is a latent footgun: a new code path that queries `api_keys` directly off the pool (not via `set_local_app_id` in a tx) silently degrades to full exposure with no error.

The prior audit treated this as "intentional for API-key authentication" and closed. Reopening because the same narrow-function pattern already used for webhook bootstrap (migration `95`) is the better shape here too.

Fix: keep the bootstrap hole but narrow it - replace the permissive SELECT policy with a dedicated `SECURITY DEFINER` lookup function (owner `bridge_admin`, `search_path=pay,pg_temp`) that selects only the columns needed for auth (e.g. `id, app_id, key_hash`) by `key_hash`/`ingress_token`, not a blanket permissive SELECT on the table.

### HIGH - RLS not FORCEd on any table

`relforcerowsecurity = false` on all 8 tenant tables. Runtime is not the owner today, so this is not an active tenant leak. But FORCE is cheap defense-in-depth: if a future migration or `ALTER OWNER` to `bridge_app` slips in, RLS would be silently bypassed for the owner without it.

Fix: `ALTER TABLE ... FORCE ROW LEVEL SECURITY` on all tenant tables.

### MEDIUM - `v_data_retention_stats` view bypasses RLS semantics

View is owned by `bridge_admin` with `arwd` to `bridge_app`. It aggregates `webhook_provider` and `fraud_prevention` without any `app_id` filter, so `bridge_app` gets cross-tenant counts/dates via the view even though the base tables are RLS-protected. Low sensitivity (counts/timestamps only) but it is a tenancy leak channel.

Fix: add `WHERE app_id = pay.current_app_id()` to each branch, or restrict view SELECT to `bridge_admin` only.

### LOW - `_sqlx_migrations` no RLS

Migration metadata is admin-owned. `bridge_app` has no grant here - good.

### INFO - Bootstrap SECURITY DEFINER functions are well-scoped

The 5 `*_bootstrap` functions are `SECURITY DEFINER`, owned by `bridge_admin`, with `search_path=pay,pg_temp` (anti-search-path-injection), and EXECUTE is granted to `bridge_app` only (not PUBLIC). They take a single UUID and return a single row. These are the correct pattern for the pre-tenant-context webhook ingress flow. No issue - noted as the model to replicate for the `api_keys` fix above.

### Recommended Action Order (updated)

1. Enable RLS + policy on `apps`; drop `bridge_app` write privileges on it. (critical, stops the credential leak)
2. Replace `api_keys` blanket bootstrap policy with a `SECURITY DEFINER` lookup function.
3. `FORCE ROW LEVEL SECURITY` on all 8 tenant tables.
4. Fix `v_data_retention_stats` tenancy filter or revoke `bridge_app` SELECT.
5. (Carried from prior audit) Map runtime SQL and reduce `bridge_app` table grants table-by-table.
6. (Carried from prior audit) Tighten default privileges so future objects do not regain broad runtime access.
