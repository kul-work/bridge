use crate::error::BridgeError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub app_id: Uuid,
    pub external_user_id: String,
    pub subscription_id: String,
    pub provider: String,
    pub purchase_token: Option<String>,
    pub status: String,
    pub current_period_end: Option<DateTime<Utc>>,
    pub auto_renewing: Option<bool>,
    pub payment_state: Option<i32>,
    pub cancel_reason: Option<i32>,
    pub provider_customer_id: Option<String>,
    pub version: i32,
    pub last_event_time: i64,
}

pub async fn get_subscription(
    pool: &PgPool,
    app_id: Uuid,
    subscription_id: &str,
) -> Result<Subscription, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions WHERE app_id = $1 AND subscription_id = $2"
    )
    .bind(app_id)
    .bind(subscription_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ValidationError("Subscription not found".to_string()))
}

pub async fn get_user_subscriptions(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<Subscription>, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "SELECT * FROM pay.subscriptions 
         WHERE app_id = $1 AND external_user_id = $2 
         ORDER BY created_at DESC 
         LIMIT $3 OFFSET $4"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn create_subscription(
    pool: &PgPool,
    app_id: Uuid,
    external_user_id: &str,
    subscription_id: &str,
    provider: &str,
    status: &str,
) -> Result<Subscription, BridgeError> {
    sqlx::query_as::<_, Subscription>(
        "INSERT INTO pay.subscriptions 
        (app_id, external_user_id, subscription_id, provider, status) 
        VALUES ($1, $2, $3, $4, $5) 
        RETURNING *"
    )
    .bind(app_id)
    .bind(external_user_id)
    .bind(subscription_id)
    .bind(provider)
    .bind(status)
    .fetch_one(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}
