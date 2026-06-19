# RLS & DB Rights Audit - 2026-06-19

Scope: Bridge `pay` schema in the local `appgen` database from `.env`, checked against the current Bridge code and migrations.

This note previously tracked open, production-relevant RLS/runtime-rights issues. The RLS hardening pass is now implemented in migrations `96` and `97`, with the matching Rust access-layer changes.

## Audit Target

Local `.env` target:

```env
DATABASE_URL=postgresql://bridge_app:postgres@localhost/appgen
ADMIN_DATABASE_URL=postgresql://bridge_admin:postgres@localhost/appgen
```

Relevant applied migrations:

- `94` - `harden bridge app privileges`
- `95` - `harden bootstrap select policies`
- `96` - `harden runtime rls privileges`
- `97` - `force rls on apps`
- `98` - `harden retention cleanup functions`

## Current Findings

No open production-relevant RLS findings remain from this audit.

## Closed Items

### `apps` RLS And Runtime Rights

Closed by migrations `96` and `97` plus Rust access-layer changes.

- `pay.apps` now has RLS enabled.
- `pay.apps` now has `FORCE ROW LEVEL SECURITY`.
- `bridge_app` has `SELECT` only on `pay.apps`; no runtime `INSERT`, `UPDATE`, or `DELETE`.
- Normal app lookup now runs in a transaction after `bridge.current_app_id` is set.
- Webhook-token bootstrap lookup uses `pay.get_app_by_webhook_token_bootstrap(...)`.

### `api_keys` Broad Bootstrap Policy

Closed by migration `96` plus Rust access-layer changes.

- `tenant_isolation_api_keys_bootstrap_select` is dropped.
- API-key auth now calls `pay.get_api_key_auth_candidates_bootstrap(key_prefix)`.
- Hash verification still happens in Rust.
- `last_used_at` is updated inside an app-scoped transaction.

### RLS Not Forced On Tenant Tables

Closed by migrations `96` and `97`.

`FORCE ROW LEVEL SECURITY` is enabled for:

- `pay.apps`
- `pay.api_keys`
- `pay.checkout_idempotency`
- `pay.fraud_prevention`
- `pay.payments`
- `pay.provider_configs`
- `pay.subscriptions`
- `pay.webhook_delivery`
- `pay.webhook_provider`

### `v_data_retention_stats` Tenant Leak

Closed by migration `96`.

- `bridge_app` no longer has `SELECT` on `pay.v_data_retention_stats`.
- The view remains admin-only unless a future app-scoped replacement is added.

### Broad Runtime DML Grants

Closed by migration `96`.

Current `bridge_app` table grants are intentionally limited to:

- `pay.apps`: `SELECT`
- `pay.api_keys`: `SELECT`, `UPDATE`
- `pay.provider_configs`: `SELECT`
- `pay.checkout_idempotency`: `SELECT`, `INSERT`
- `pay.payments`: `SELECT`, `INSERT`, `UPDATE`
- `pay.subscriptions`: `SELECT`, `INSERT`, `UPDATE`, `DELETE`
- `pay.webhook_provider`: `SELECT`, `INSERT`, `UPDATE`
- `pay.webhook_delivery`: `SELECT`, `INSERT`, `UPDATE`
- `pay.fraud_prevention`: `SELECT`, `INSERT`, `UPDATE`
- `pay.v_data_retention_stats`: no `bridge_app` access

Sequence access remains granted where runtime inserts need generated IDs. Runtime retention cleanup uses narrow `SECURITY DEFINER` functions instead of direct `DELETE` grants on `pay.webhook_provider` or `pay.fraud_prevention`.

### Default Privileges

Closed for the production migration owner by migration `96`.

Future table and function access for `bridge_app` should now be granted explicitly by migrations instead of inherited broadly by default privileges. Local databases that previously ran migrations as another owner, such as `postgres`, can still show old owner-specific default ACLs until rebuilt; that is local drift and is expected to disappear when the DB is recreated with the production migration role.

### PUBLIC Database Rights

No production-relevant RLS finding remains here.

The prior note about `PUBLIC CONNECT/TEMPORARY` is general PostgreSQL hardening, not a tenant RLS leak. It was intentionally left out of this RLS pass to avoid changing database-level connectivity semantics without a dedicated role inventory.

## Verification Performed

Local verification after applying migrations `96` and `97`:

- `cargo check` passed.
- `cargo test` passed: 97 tests.
- `bridge_app` with no `bridge.current_app_id` saw zero rows from tenant tables.
- `pay.apps` direct read returned zero rows without app context.
- No `pay` policies with `bootstrap` in the policy name remain.
- All listed tenant tables report `relforcerowsecurity = true`.
- `bridge_app` receives `permission denied` on `pay.v_data_retention_stats`.
- `pay.get_app_by_webhook_token_bootstrap(...)` returned the expected single row for a valid token.
- `pay.get_api_key_auth_candidates_bootstrap(...)` returned the expected single candidate for a valid key prefix.
- `pay.list_enabled_app_ids_bootstrap()` and `pay.list_app_summaries_bootstrap()` returned expected worker/admin rows.