# SQLx Migration Recovery - Codex Strategy

Status: operational strategy, not executed.

Goal: remove the bad branch migrations from the local/dev database without nuking app data.

Assumption: the unwanted migrations are from `webhook-atomicity-outbox-payload`, likely:

- `96_webhook_outbox_payload_primitives.sql`
- `97_subscription_scheduler_claims.sql`
- `98_subscription_reconciliation_claims.sql`
- `99_payment_acknowledgement_claims.sql`
- `100_subscription_status_check.sql`
- `101_subscription_status_check_grace_period.sql`

## Principle

Do not only delete rows from SQLx migration metadata. That makes SQLx forget the migrations, but it leaves schema objects behind.

Correct recovery order:

1. Confirm exactly which migration files were applied.
2. Inspect what schema objects currently exist.
3. Drop only schema objects introduced by those migrations.
4. Delete matching rows from `_sqlx_migrations`.
5. Run the app/migration startup again and verify it does not try to reconcile stale schema.

## Step 1 - Snapshot Before Touching Anything

Take a schema/data backup first.

Recommended local command shape:

```powershell
cmd /c "set PGPASSWORD=password && pg_dump -U postgres -h localhost -p 5432 -d appgen -n pay -Fc -f C:\share\tyde\bridge\bridge-pay-before-migration-recovery.dump"
```

Also capture current migration metadata:

```sql
SELECT version, description, installed_on, success, checksum
FROM _sqlx_migrations
ORDER BY version;
```

## Step 2 - Confirm Applied Bad Versions

Check whether versions `96` through `101` exist in `_sqlx_migrations`.

If a version is not present, do not include it in cleanup.

Also confirm whether the corresponding columns/constraints/indexes exist, because a failed or manually edited DB may be partially changed.

## Step 3 - Revert Schema Objects Safely

Use an idempotent cleanup SQL script. Run it once, inspect output, then rerun only if needed.

Candidate cleanup shape:

```sql
BEGIN;

SET search_path TO pay, public;

-- Migration 101 / 100
ALTER TABLE IF EXISTS subscriptions
    DROP CONSTRAINT IF EXISTS chk_subscriptions_status_allowed;

-- Migration 99
DROP INDEX IF EXISTS idx_payments_google_play_ack_claim;
ALTER TABLE IF EXISTS payments
    DROP COLUMN IF EXISTS ack_claim_owner,
    DROP COLUMN IF EXISTS ack_claim_expires_at;

-- Migration 98
DROP INDEX IF EXISTS idx_subs_reconciliation_claim;
ALTER TABLE IF EXISTS subscriptions
    DROP COLUMN IF EXISTS reconciliation_claim_owner,
    DROP COLUMN IF EXISTS reconciliation_claim_expires_at;

-- Migration 97
DROP INDEX IF EXISTS idx_subs_price_step_up_expiry_claim;
DROP INDEX IF EXISTS idx_subs_pause_scheduler_claim;
ALTER TABLE IF EXISTS subscriptions
    DROP COLUMN IF EXISTS scheduler_claim_owner,
    DROP COLUMN IF EXISTS scheduler_claim_expires_at;

-- Migration 96
DROP INDEX IF EXISTS idx_webhook_delivery_claimable_payload;
DROP INDEX IF EXISTS idx_webhook_provider_processing_claim;

ALTER TABLE IF EXISTS webhook_provider
    DROP CONSTRAINT IF EXISTS chk_webhook_provider_processing_status,
    DROP CONSTRAINT IF EXISTS chk_webhook_provider_processing_attempts_non_negative;

ALTER TABLE IF EXISTS webhook_delivery
    DROP COLUMN IF EXISTS canonical_payload,
    DROP COLUMN IF EXISTS claim_owner,
    DROP COLUMN IF EXISTS claim_expires_at;

ALTER TABLE IF EXISTS webhook_provider
    DROP COLUMN IF EXISTS processing_status,
    DROP COLUMN IF EXISTS processing_attempts,
    DROP COLUMN IF EXISTS next_processing_at,
    DROP COLUMN IF EXISTS last_processing_error,
    DROP COLUMN IF EXISTS processing_claim_owner,
    DROP COLUMN IF EXISTS processing_claim_expires_at;

COMMIT;
```

Important: migration `96` also replaced `pay.cleanup_old_webhook_provider()`. If the current `dev` migration history expects the old body, restore it from the current `dev` migration definition after dropping the branch-only objects.

## Step 4 - Delete SQLx Metadata Rows

Only after schema cleanup:

```sql
DELETE FROM _sqlx_migrations
WHERE version IN (96, 97, 98, 99, 100, 101);
```

If SQLx stores versions as `BIGINT`, this works as-is. If the table was created differently, inspect column types first.

## Step 5 - Verify Schema Is Back To Dev Shape

Run inspection queries:

```sql
SELECT table_name, column_name
FROM information_schema.columns
WHERE table_schema = 'pay'
  AND (
      column_name IN (
          'canonical_payload',
          'claim_owner',
          'claim_expires_at',
          'processing_status',
          'processing_attempts',
          'next_processing_at',
          'last_processing_error',
          'processing_claim_owner',
          'processing_claim_expires_at',
          'scheduler_claim_owner',
          'scheduler_claim_expires_at',
          'reconciliation_claim_owner',
          'reconciliation_claim_expires_at',
          'ack_claim_owner',
          'ack_claim_expires_at'
      )
  )
ORDER BY table_name, column_name;
```

Expected: zero rows, unless `dev` has since legitimately added one of these columns.

Check constraints:

```sql
SELECT conname
FROM pg_constraint
WHERE connamespace = 'pay'::regnamespace
  AND conname IN (
      'chk_webhook_provider_processing_status',
      'chk_webhook_provider_processing_attempts_non_negative',
      'chk_subscriptions_status_allowed'
  );
```

Expected: zero rows.

Check indexes:

```sql
SELECT indexname
FROM pg_indexes
WHERE schemaname = 'pay'
  AND indexname IN (
      'idx_webhook_delivery_claimable_payload',
      'idx_webhook_provider_processing_claim',
      'idx_subs_price_step_up_expiry_claim',
      'idx_subs_pause_scheduler_claim',
      'idx_subs_reconciliation_claim',
      'idx_payments_google_play_ack_claim'
  );
```

Expected: zero rows.

## Step 6 - Restart Or Run Check Against Current Dev

After cleanup:

- run current `dev` app startup/migrations;
- verify no migration mismatch;
- run focused webhook/payment smoke checks, especially OTP refund if branch code ever ran against this DB.

## Safer Alternative If Data Matters A Lot

If this DB has important local data and you want a lower-risk path:

1. Create a fresh DB from current `dev` migrations.
2. Export only business tables from the dirty DB.
3. Import into fresh DB.
4. Keep the dirty DB as backup until tests pass.

This is not a nuke of data, but it is cleaner than hand-reverting schema if many experimental migrations were applied.

## What Not To Do

- Do not delete `_sqlx_migrations` rows first and then run the app. SQLx may try to apply current files onto a schema that still has branch-only columns/constraints.
- Do not drop columns before checking whether current `dev` now uses them.
- Do not drop the whole `pay` schema unless you explicitly accept losing all Bridge payment/subscription/webhook data.
