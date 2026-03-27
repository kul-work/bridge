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


