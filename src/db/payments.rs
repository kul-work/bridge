use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Payment record for audit trail
/// Currently stored but not actively queried. Struct construction is handled by SQLx FromRow.
#[allow(dead_code)]
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Payment {
    pub id: Uuid,
    pub app_id: Uuid,
    pub external_user_id: String,
    pub provider: String,
    pub provider_transaction_id: String,
    pub subscription_id: Option<String>,
    pub product_id: Option<String>,
    pub amount_cents: i32,
    pub currency: String,
    pub status: String,
}

pub async fn record_payment_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    provider: &str,
    provider_transaction_id: &str,
    subscription_id: Option<&str>,
    amount_cents: i32,
    status: &str,
) -> Result<(), crate::error::BridgeError> {
    let result = sqlx::query(
        "INSERT INTO pay.payments (app_id, external_user_id, provider, provider_transaction_id, subscription_id, amount_cents, status, webhook_received_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
         ON CONFLICT (app_id, provider, provider_transaction_id)
         DO UPDATE SET
           status = EXCLUDED.status,
           subscription_id = COALESCE(EXCLUDED.subscription_id, payments.subscription_id),
           amount_cents = CASE WHEN EXCLUDED.amount_cents > 0 THEN EXCLUDED.amount_cents ELSE payments.amount_cents END,
           webhook_received_at = NOW()
         WHERE payments.external_user_id = EXCLUDED.external_user_id"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(provider)
    .bind(provider_transaction_id)
    .bind(subscription_id)
    .bind(amount_cents)
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

pub async fn get_payment_status(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider_transaction_id: &str,
) -> Result<Option<String>, crate::error::BridgeError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT status FROM pay.payments WHERE app_id = $1 AND provider_transaction_id = $2"
    )
    .bind(app_id)
    .bind(provider_transaction_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(row.map(|r| r.0))
}

pub async fn update_payment_status(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider_transaction_id: &str,
    new_status: &str,
) -> Result<(), crate::error::BridgeError> {
    sqlx::query(
        "UPDATE pay.payments SET status = $1, webhook_received_at = NOW() WHERE app_id = $2 AND provider_transaction_id = $3"
    )
    .bind(new_status)
    .bind(app_id)
    .bind(provider_transaction_id)
    .execute(pool)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn lookup_user_by_purchase_token_payment(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    purchase_token: &str,
) -> Result<Option<String>, crate::error::BridgeError> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT external_user_id FROM pay.payments WHERE app_id = $1 AND provider_transaction_id = $2 LIMIT 1"
    )
    .bind(app_id)
    .bind(purchase_token)
    .fetch_optional(pool)
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
    sqlx::query_as::<_, Payment>(
        "SELECT * FROM pay.payments 
         WHERE app_id = $1 AND external_user_id = $2 
         ORDER BY webhook_received_at DESC 
         LIMIT $3 OFFSET $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))
}

pub async fn adopt_stale_payment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
) -> Result<(), crate::error::BridgeError> {
    // Merge old records with mismatched transaction IDs for this user/subscription
    // that are currently 'pending' or 'failed' but should be part of this subscription.
    // This is specific to Creem's behavior described in §13.
    sqlx::query(
        "UPDATE pay.payments 
         SET subscription_id = $1, status = 'success'
         WHERE app_id = $2 
           AND external_user_id = $3 
           AND provider = 'creem' 
           AND subscription_id IS NULL 
           AND status IN ('pending', 'failed')
           AND created_at > NOW() - INTERVAL '24 hours'"
    )
    .bind(subscription_id)
    .bind(app_id)
    .bind(external_user_id)
    .execute(&mut **tx)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    Ok(())
}


