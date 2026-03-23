use crate::error::BridgeError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, FromRow};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WebhookProvider {
    pub id: Uuid,
    pub app_id: Uuid,
    pub provider: String,
    pub provider_webhook_id: String,
    pub event_type: String,
    pub subscription_id: Option<String>,
    pub purchase_token: Option<String>,
    pub payload: serde_json::Value,
    pub processed: bool,
    pub timestamp_epoch_ms: Option<i64>,
    pub suppressed: bool,
    pub suppressed_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub id: Uuid,
    pub app_id: Uuid,
    pub webhook_provider_id: Uuid,
    pub forward_attempts: i32,
    pub forwarded: bool,
    pub forwarded_at: Option<DateTime<Utc>>,
    pub last_http_status: Option<i32>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Get webhook provider by ID
pub async fn get_webhook_provider(pool: &PgPool, id: Uuid) -> Result<WebhookProvider, BridgeError> {
    sqlx::query_as::<_, WebhookProvider>(
        "SELECT * FROM pay.webhook_provider WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ValidationError("Webhook not found".to_string()))
}

/// Mark webhook as suppressed (stale event)
pub async fn suppress_webhook(
    pool: &PgPool,
    webhook_id: Uuid,
    reason: &str,
) -> Result<(), BridgeError> {
    sqlx::query(
        "UPDATE pay.webhook_provider SET suppressed = true, suppressed_reason = $1 WHERE id = $2"
    )
    .bind(reason)
    .bind(webhook_id)
    .execute(pool)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to suppress webhook: {}", e)))?;
    Ok(())
}

/// Get webhook delivery by ID
pub async fn get_webhook_delivery(pool: &PgPool, id: Uuid) -> Result<WebhookDelivery, BridgeError> {
    sqlx::query_as::<_, WebhookDelivery>(
        "SELECT * FROM pay.webhook_delivery WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ValidationError("Webhook delivery not found".to_string()))
}

/// Update webhook delivery after forward attempt
pub async fn update_webhook_delivery_attempt(
    pool: &PgPool,
    delivery_id: Uuid,
    http_status: Option<i32>,
    error: Option<String>,
    forwarded: bool,
) -> Result<(), BridgeError> {
    sqlx::query(
        "UPDATE pay.webhook_delivery 
         SET forward_attempts = forward_attempts + 1,
             last_http_status = $1,
             last_error = $2,
             forwarded = $3,
             forwarded_at = CASE WHEN $3 THEN NOW() ELSE forwarded_at END,
             updated_at = NOW()
         WHERE id = $4"
    )
    .bind(http_status)
    .bind(error)
    .bind(forwarded)
    .bind(delivery_id)
    .execute(pool)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to update webhook delivery: {}", e)))?;
    Ok(())
}

/// List recent webhook deliveries for app (admin page)
pub async fn list_app_webhooks(
    pool: &PgPool,
    app_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<(WebhookDelivery, WebhookProvider)>, BridgeError> {
    // Get deliveries first
    let deliveries = sqlx::query_as::<_, WebhookDelivery>(
        "SELECT * FROM pay.webhook_delivery 
         WHERE app_id = $1
         ORDER BY created_at DESC
         LIMIT $2 OFFSET $3"
    )
    .bind(app_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    // For each delivery, get the provider webhook
    let mut results = Vec::new();
    for delivery in deliveries {
        if let Ok(provider) = get_webhook_provider(pool, delivery.webhook_provider_id).await {
            results.push((delivery, provider));
        }
    }

    Ok(results)
}

/// Count failed webhook deliveries for app
pub async fn count_failed_webhooks(pool: &PgPool, app_id: Uuid) -> Result<i64, BridgeError> {
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pay.webhook_delivery 
         WHERE app_id = $1 AND (forwarded = false OR last_http_status >= 400)"
    )
    .bind(app_id)
    .fetch_one(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;
    Ok(count.0)
}
