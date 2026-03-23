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


