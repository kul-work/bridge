use crate::db::database::set_local_app_id;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Payment record for audit trail
/// Currently stored but not actively queried. Struct construction is handled by SQLx FromRow.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Payment {
    pub id: Uuid,
    pub app_id: Uuid,
    pub external_user_id: String,
    pub provider: String,
    pub provider_transaction_id: String,
    pub subscription_id: Option<String>,
    pub product_id: Option<String>,
    pub amount_cents: i64,
    pub currency: String,
    pub status: String,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct PaymentHistoryEntry {
    pub id: Uuid,
    pub external_user_id: String,
    pub subscription_id: Option<String>,
    pub provider: String,
    pub provider_transaction_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct GooglePlaySubscriptionAckCandidate {
    pub payment_id: Uuid,
    pub subscription_id: String,
    pub purchase_token: String,
    pub claim_token: Uuid,
}

#[derive(Debug, Clone, FromRow)]
pub struct GooglePlayProductAckCandidate {
    pub payment_id: Uuid,
    pub product_id: String,
    pub purchase_token: String,
    pub claim_token: Uuid,
}

async fn begin_app_tx<'a>(
    pool: &'a sqlx::PgPool,
    app_id: Uuid,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, crate::error::BridgeError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;
    Ok(tx)
}

#[allow(clippy::too_many_arguments)]
pub async fn record_payment_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    provider_transaction_id: &str,
    subscription_id: Option<&str>,
    product_id: Option<&str>,
    amount_cents: i64,
    currency: Option<&str>,
    status: &str,
) -> Result<(), crate::error::BridgeError> {
    record_payment_with_purchase_token_tx(
        tx,
        app_id,
        external_user_id,
        provider,
        provider_transaction_id,
        None,
        false,
        subscription_id,
        product_id,
        amount_cents,
        currency,
        status,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn record_payment_with_purchase_token_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    provider_transaction_id: &str,
    provider_purchase_token: Option<&str>,
    ack_required: bool,
    subscription_id: Option<&str>,
    product_id: Option<&str>,
    amount_cents: i64,
    currency: Option<&str>,
    status: &str,
) -> Result<(), crate::error::BridgeError> {
    let currency = currency.map(str::trim).filter(|value| {
        !value.is_empty() && !value.eq_ignore_ascii_case("N/A") && !value.eq_ignore_ascii_case("UNKNOWN")
    });

    if currency.is_none() && !ack_required {
        tracing::info!(
            app_id = %app_id,
            provider,
            status,
            provider_transaction_id,
            "Skipping payment record without explicit currency"
        );
        return Ok(());
    };

    let amount_cents = (amount_cents >= 0).then_some(amount_cents);

    if amount_cents.is_none() && !ack_required {
        tracing::info!(
            app_id = %app_id,
            provider,
            status,
            provider_transaction_id,
            "Skipping payment record without explicit amount"
        );
        return Ok(());
    }

    let result = sqlx::query(
        "INSERT INTO pay.payments (app_id, external_user_id, provider, provider_transaction_id, provider_purchase_token, ack_required, subscription_id, product_id, amount_cents, currency, status, webhook_received_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW())
         ON CONFLICT (app_id, provider, provider_transaction_id)
         DO UPDATE SET
           status = CASE
             WHEN payments.status = 'refunded' AND EXCLUDED.status IN ('pending', 'success', 'cancelled')
               THEN payments.status
             WHEN payments.status = 'cancelled' AND EXCLUDED.status IN ('pending', 'success')
               THEN payments.status
             WHEN payments.status = 'success' AND EXCLUDED.status = 'pending'
               THEN payments.status
             ELSE EXCLUDED.status
           END,
           provider_purchase_token = COALESCE(EXCLUDED.provider_purchase_token, payments.provider_purchase_token),
           ack_required = payments.ack_required OR EXCLUDED.ack_required,
           subscription_id = COALESCE(EXCLUDED.subscription_id, payments.subscription_id),
           product_id = COALESCE(EXCLUDED.product_id, payments.product_id),
           amount_cents = COALESCE(EXCLUDED.amount_cents, payments.amount_cents),
           currency = COALESCE(EXCLUDED.currency, payments.currency),
           webhook_received_at = NOW()
         WHERE payments.external_user_id = EXCLUDED.external_user_id"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(provider)
    .bind(provider_transaction_id)
    .bind(provider_purchase_token)
    .bind(ack_required)
    .bind(subscription_id)
    .bind(product_id)
    .bind(amount_cents)
    .bind(currency)
    .bind(status)
    .execute(&mut **tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(crate::error::BridgeError::FraudDetected(
            format!(
                "Payment tx_id={} for app={} conflict: external_user_id mismatch",
                provider_transaction_id, app_id
            )
        ));
    }

    Ok(())
}

pub async fn get_payment_status_for_provider(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    provider_transaction_id: &str,
) -> Result<Option<String>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT status
         FROM pay.payments
         WHERE app_id = $1
           AND provider = $2
           AND (provider_transaction_id = $3 OR provider_purchase_token = $3)
         ORDER BY created_at ASC
         LIMIT 1"
    )
    .bind(app_id)
    .bind(provider)
    .bind(provider_transaction_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn get_payment_subscription_id_for_provider(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    provider_transaction_id: &str,
) -> Result<Option<String>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT subscription_id
         FROM pay.payments
         WHERE app_id = $1
           AND provider = $2
           AND (provider_transaction_id = $3 OR provider_purchase_token = $3)
         ORDER BY created_at ASC
         LIMIT 1"
    )
    .bind(app_id)
    .bind(provider)
    .bind(provider_transaction_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(row.and_then(|r| r.0))
}

pub async fn get_payment_currency_for_subscription(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    external_user_id: &str,
    subscription_id: &str,
) -> Result<Option<String>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT currency
         FROM pay.payments
         WHERE app_id = $1
           AND provider = $2
           AND external_user_id = $3
           AND subscription_id = $4
           AND currency != 'N/A'
           AND currency != 'UNKNOWN'
         ORDER BY created_at DESC
         LIMIT 1"
    )
    .bind(app_id)
    .bind(provider)
    .bind(external_user_id)
    .bind(subscription_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn update_payment_status_for_provider(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    provider_transaction_id: &str,
    new_status: &str,
) -> Result<(), crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    update_payment_status_for_provider_tx(
        &mut tx,
        app_id,
        provider,
        provider_transaction_id,
        new_status,
    )
    .await?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn update_payment_status_for_provider_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    provider: &str,
    provider_transaction_id: &str,
    new_status: &str,
) -> Result<(), crate::error::BridgeError> {
    sqlx::query(
        "UPDATE pay.payments
         SET status = $1,
             ack_required = CASE
                 WHEN provider = 'google_play' AND $1 = 'success' AND provider_purchase_token = $4
                   THEN true
                 ELSE ack_required
             END,
             webhook_received_at = NOW()
         WHERE app_id = $2
           AND provider = $3
           AND (provider_transaction_id = $4 OR provider_purchase_token = $4)"
    )
    .bind(new_status)
    .bind(app_id)
    .bind(provider)
    .bind(provider_transaction_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn get_payment_acknowledged_at(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    provider_transaction_id: &str,
) -> Result<Option<DateTime<Utc>>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row = sqlx::query_scalar(
        "SELECT acknowledged_at
         FROM pay.payments
         WHERE app_id = $1
           AND provider = $2
           AND (
             provider_transaction_id = $3
             OR (provider_purchase_token = $3 AND ack_required = true)
           )
         ORDER BY ack_required DESC, created_at ASC
         LIMIT 1",
    )
    .bind(app_id)
    .bind(provider)
    .bind(provider_transaction_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(row.flatten())
}

pub async fn mark_payment_acknowledged(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    provider_transaction_id: &str,
) -> Result<(), crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    sqlx::query(
        "UPDATE pay.payments
         SET acknowledged_at = COALESCE(acknowledged_at, NOW())
         WHERE app_id = $1
           AND provider = $2
           AND (
             provider_transaction_id = $3
             OR (provider_purchase_token = $3 AND ack_required = true)
           )",
    )
    .bind(app_id)
    .bind(provider)
    .bind(provider_transaction_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn mark_payment_acknowledged_with_claim(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    payment_id: Uuid,
    claim_token: Uuid,
) -> Result<bool, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let result = sqlx::query(
        "UPDATE pay.payments
         SET acknowledged_at = COALESCE(acknowledged_at, NOW()),
             ack_claim_token = NULL,
             ack_claimed_by = NULL,
             ack_claimed_until = NULL
         WHERE app_id = $1
           AND id = $2
           AND ack_claim_token = $3
           AND acknowledged_at IS NULL",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(claim_token)
    .execute(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected() == 1)
}

pub async fn refresh_payment_ack_claim(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    payment_id: Uuid,
    claim_token: Uuid,
    lease_secs: i64,
) -> Result<bool, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let result = sqlx::query(
        "UPDATE pay.payments
         SET ack_claimed_until = NOW() + ($4 * INTERVAL '1 second')
         WHERE app_id = $1
           AND id = $2
           AND ack_claim_token = $3
           AND acknowledged_at IS NULL",
    )
    .bind(app_id)
    .bind(payment_id)
    .bind(claim_token)
    .bind(lease_secs)
    .execute(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected() == 1)
}

pub async fn claim_google_play_subscription_ack_candidates(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    worker_id: &str,
    lease_secs: i64,
    limit: i64,
) -> Result<Vec<GooglePlaySubscriptionAckCandidate>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let rows = sqlx::query_as::<_, GooglePlaySubscriptionAckCandidate>(
        "WITH candidates AS (
             SELECT p.id
             FROM pay.payments p
             JOIN pay.subscriptions s
               ON s.app_id = p.app_id
              AND s.provider = p.provider
              AND s.purchase_token = p.provider_purchase_token
             WHERE p.app_id = $1
               AND p.provider = 'google_play'
               AND p.status = 'success'
               AND p.ack_required = true
               AND p.acknowledged_at IS NULL
               AND p.provider_purchase_token IS NOT NULL
               AND s.status IN ('active', 'past_due', 'cancelled', 'on_hold', 'paused')
               AND (
                   p.ack_claimed_until IS NULL
                   OR p.ack_claimed_until < NOW()
               )
             ORDER BY p.created_at ASC
             LIMIT $4
             FOR UPDATE SKIP LOCKED
         )
         UPDATE pay.payments p
         SET ack_claim_token = gen_random_uuid(),
             ack_claimed_by = $2,
             ack_claimed_until = NOW() + ($3 * INTERVAL '1 second')
         FROM candidates
         WHERE p.id = candidates.id AND p.app_id = $1
         RETURNING p.id AS payment_id,
                   (SELECT s.subscription_id
                    FROM pay.subscriptions s
                    WHERE s.app_id = p.app_id
                      AND s.provider = p.provider
                      AND s.purchase_token = p.provider_purchase_token
                    LIMIT 1) AS subscription_id,
                   p.provider_purchase_token AS purchase_token,
                   p.ack_claim_token AS claim_token",
    )
    .bind(app_id)
    .bind(worker_id)
    .bind(lease_secs)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(rows)
}

pub async fn claim_google_play_product_ack_candidates(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    worker_id: &str,
    lease_secs: i64,
    limit: i64,
) -> Result<Vec<GooglePlayProductAckCandidate>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let rows = sqlx::query_as::<_, GooglePlayProductAckCandidate>(
        "WITH candidates AS (
             SELECT id
             FROM pay.payments
             WHERE app_id = $1
               AND provider = 'google_play'
               AND status = 'success'
               AND ack_required = true
               AND acknowledged_at IS NULL
               AND provider_purchase_token IS NOT NULL
               AND product_id IS NOT NULL
               AND subscription_id IS NULL
               AND (
                   ack_claimed_until IS NULL
                   OR ack_claimed_until < NOW()
               )
             ORDER BY created_at ASC
             LIMIT $4
             FOR UPDATE SKIP LOCKED
         )
         UPDATE pay.payments p
         SET ack_claim_token = gen_random_uuid(),
             ack_claimed_by = $2,
             ack_claimed_until = NOW() + ($3 * INTERVAL '1 second')
         FROM candidates
         WHERE p.id = candidates.id AND p.app_id = $1
         RETURNING p.id AS payment_id,
                   p.product_id,
                   p.provider_purchase_token AS purchase_token,
                   p.ack_claim_token AS claim_token",
    )
    .bind(app_id)
    .bind(worker_id)
    .bind(lease_secs)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(rows)
}

pub async fn lookup_user_by_purchase_token_payment(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    purchase_token: &str,
) -> Result<Option<String>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT external_user_id
         FROM pay.payments
         WHERE app_id = $1
           AND provider = $2
           AND (provider_transaction_id = $3 OR provider_purchase_token = $3)
         ORDER BY created_at ASC
         LIMIT 1"
    )
    .bind(app_id)
    .bind(provider)
    .bind(purchase_token)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn lookup_product_id_by_purchase_token_payment(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    purchase_token: &str,
) -> Result<Option<String>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT COALESCE(product_id, subscription_id)
         FROM pay.payments
         WHERE app_id = $1
           AND provider = $2
           AND (provider_transaction_id = $3 OR provider_purchase_token = $3)
           AND COALESCE(product_id, subscription_id) IS NOT NULL
         ORDER BY created_at ASC
         LIMIT 1"
    )
    .bind(app_id)
    .bind(provider)
    .bind(purchase_token)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn get_user_payments(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    external_user_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Payment>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let payments = sqlx::query_as::<_, Payment>(
        "SELECT id, app_id, external_user_id, provider, provider_transaction_id, subscription_id, product_id,
                COALESCE(amount_cents, -1) AS amount_cents, COALESCE(currency, 'UNKNOWN') AS currency,
                status
         FROM pay.payments 
         WHERE app_id = $1 AND external_user_id = $2 
         ORDER BY webhook_received_at DESC 
         LIMIT $3 OFFSET $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(payments)
}

pub async fn count_user_payments(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    external_user_id: &str,
) -> Result<i64, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let total: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pay.payments WHERE app_id = $1 AND external_user_id = $2",
    )
    .bind(app_id)
    .bind(external_user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(total.0)
}

pub async fn list_user_payments_keyset(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    external_user_id: &str,
    limit: i64,
    after_created_at: Option<DateTime<Utc>>,
    after_id: Option<Uuid>,
) -> Result<Vec<PaymentHistoryEntry>, crate::error::BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;
    let rows = if let (Some(created_at), Some(id)) = (after_created_at, after_id) {
        sqlx::query_as::<_, PaymentHistoryEntry>(
            r#"
            SELECT
                id, external_user_id, subscription_id, provider, provider_transaction_id,
                COALESCE(amount_cents, -1) AS amount_cents, COALESCE(currency, 'UNKNOWN') AS currency, status, created_at
            FROM pay.payments
            WHERE app_id = $1 AND external_user_id = $2
              AND (created_at, id) < ($3, $4)
            ORDER BY created_at DESC, id DESC
            LIMIT $5
            "#,
        )
        .bind(app_id)
        .bind(external_user_id)
        .bind(created_at)
        .bind(id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?
    } else {
        sqlx::query_as::<_, PaymentHistoryEntry>(
            r#"
            SELECT
                id, external_user_id, subscription_id, provider, provider_transaction_id,
                COALESCE(amount_cents, -1) AS amount_cents, COALESCE(currency, 'UNKNOWN') AS currency, status, created_at
            FROM pay.payments
            WHERE app_id = $1 AND external_user_id = $2
            ORDER BY created_at DESC, id DESC
            LIMIT $3
            "#,
        )
        .bind(app_id)
        .bind(external_user_id)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?
    };

    tx.commit()
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(rows)
}

pub async fn adopt_stale_payment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    stale_payment_window_secs: i64,
) -> Result<(), crate::error::BridgeError> {
    // Merge old records with mismatched transaction IDs for this user/subscription
    // that are currently 'pending' or 'failed' but should be part of this subscription.
    // This is specific to Creem's behavior described in §13.
    let interval = format!("{} seconds", stale_payment_window_secs);
    sqlx::query(
        "UPDATE pay.payments 
         SET subscription_id = $1, status = 'success'
         WHERE app_id = $2 
           AND external_user_id = $3 
           AND provider = 'creem' 
           AND subscription_id IS NULL 
           AND status IN ('pending', 'failed')
           AND created_at > NOW() - $4::interval"
    )
    .bind(subscription_id)
    .bind(app_id)
    .bind(external_user_id)
    .bind(&interval)
    .execute(&mut **tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(())
}
