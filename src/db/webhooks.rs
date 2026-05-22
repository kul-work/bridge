use crate::db::database::set_local_app_id;
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
    pub dead_lettered: bool,
    pub dead_lettered_at: Option<DateTime<Utc>>,
    pub dead_letter_reason: Option<String>,
    pub last_http_status: Option<i32>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct WebhookDeliveryEnqueue {
    pub id: Uuid,
    pub created: bool,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct WebhookRecord {
    pub provider: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

async fn begin_app_tx<'a>(
    pool: &'a PgPool,
    app_id: Uuid,
) -> Result<sqlx::Transaction<'a, sqlx::Postgres>, BridgeError> {
    let mut tx = pool.begin().await.map_err(|e| BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;
    Ok(tx)
}

async fn get_webhook_provider_app_id(pool: &PgPool, webhook_id: Uuid) -> Result<Uuid, BridgeError> {
    sqlx::query_scalar("SELECT app_id FROM pay.webhook_provider WHERE id = $1")
        .bind(webhook_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?
        .ok_or_else(|| BridgeError::ValidationError("Webhook not found".to_string()))
}

async fn get_webhook_delivery_app_id(pool: &PgPool, delivery_id: Uuid) -> Result<Uuid, BridgeError> {
    sqlx::query_scalar("SELECT app_id FROM pay.webhook_delivery WHERE id = $1")
        .bind(delivery_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?
        .ok_or_else(|| BridgeError::ValidationError("Webhook delivery not found".to_string()))
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
/// Used for stale event suppression - called during webhook processing when
/// newer events have already been processed for the same subscription.
pub async fn suppress_webhook(
    pool: &PgPool,
    webhook_id: Uuid,
    reason: &str,
) -> Result<(), BridgeError> {
    let app_id = get_webhook_provider_app_id(pool, webhook_id).await?;
    let mut tx = begin_app_tx(pool, app_id).await?;

    sqlx::query(
        "UPDATE pay.webhook_provider SET suppressed = true, suppressed_reason = $1 WHERE id = $2"
    )
    .bind(reason)
    .bind(webhook_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to suppress webhook: {}", e)))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn mark_webhook_processed(
    pool: &PgPool,
    webhook_id: Uuid,
) -> Result<(), BridgeError> {
    let app_id = get_webhook_provider_app_id(pool, webhook_id).await?;
    let mut tx = begin_app_tx(pool, app_id).await?;

    sqlx::query("UPDATE pay.webhook_provider SET processed = true WHERE id = $1")
        .bind(webhook_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

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

pub async fn webhook_delivery_exists(
    pool: &PgPool,
    webhook_provider_id: Uuid,
) -> Result<bool, BridgeError> {
    sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1
             FROM pay.webhook_delivery
             WHERE webhook_provider_id = $1
         )"
    )
    .bind(webhook_provider_id)
    .fetch_one(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

pub async fn list_pending_webhook_deliveries(
    pool: &PgPool,
    app_id: Uuid,
    limit: i64,
) -> Result<Vec<WebhookDelivery>, BridgeError> {
    sqlx::query_as::<_, WebhookDelivery>(
        "SELECT * FROM pay.webhook_delivery
         WHERE app_id = $1 AND forwarded = false AND dead_lettered = false AND forward_attempts < 3
         ORDER BY created_at ASC
         LIMIT $2"
    )
    .bind(app_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

/// Update webhook delivery after forward attempt
pub async fn update_webhook_delivery_attempt(
    pool: &PgPool,
    delivery_id: Uuid,
    http_status: Option<i32>,
    error: Option<String>,
    forwarded: bool,
) -> Result<(), BridgeError> {
    let app_id = get_webhook_delivery_app_id(pool, delivery_id).await?;
    let mut tx = begin_app_tx(pool, app_id).await?;

    sqlx::query(
        "UPDATE pay.webhook_delivery 
         SET forward_attempts = forward_attempts + 1,
             last_http_status = $1,
             last_error = $2,
             forwarded = $3,
             forwarded_at = CASE WHEN $3 THEN NOW() ELSE forwarded_at END,
             dead_lettered = CASE
                 WHEN $3 THEN false
                 WHEN forward_attempts + 1 >= 3 THEN true
                 ELSE dead_lettered
             END,
             dead_lettered_at = CASE
                 WHEN $3 THEN NULL
                 WHEN forward_attempts + 1 >= 3 THEN NOW()
                 ELSE dead_lettered_at
             END,
             dead_letter_reason = CASE
                 WHEN $3 THEN NULL
                 WHEN forward_attempts + 1 >= 3 THEN COALESCE($2::TEXT, 'Retry limit exceeded')
                 ELSE dead_letter_reason
             END,
             updated_at = NOW()
         WHERE id = $4"
    )
    .bind(http_status)
    .bind(error)
    .bind(forwarded)
    .bind(delivery_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to update webhook delivery: {}", e)))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

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

pub async fn cleanup_old_webhook_provider(pool: &PgPool) -> Result<(), BridgeError> {
    sqlx::query("SELECT pay.cleanup_old_webhook_provider()")
        .execute(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn list_user_webhook_records(
    pool: &PgPool,
    app_id: Uuid,
    subscription_ids: &[String],
    purchase_tokens: &[String],
) -> Result<Vec<WebhookRecord>, BridgeError> {
    sqlx::query_as::<_, WebhookRecord>(
        "SELECT provider, event_type, payload, created_at
         FROM pay.webhook_provider
         WHERE app_id = $1
           AND (subscription_id = ANY($2) OR purchase_token = ANY($3))
         ORDER BY created_at DESC"
    )
    .bind(app_id)
    .bind(subscription_ids)
    .bind(purchase_tokens)
    .fetch_all(pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))
}

/// Create webhook provider record (idempotent via unique index)
/// Returns the webhook provider ID and a flag indicating if it was newly created
#[allow(clippy::too_many_arguments)]
pub async fn create_webhook_provider(
    pool: &PgPool,
    app_id: Uuid,
    provider: &str,
    provider_webhook_id: &str,
    event_type: &str,
    subscription_id: Option<String>,
    purchase_token: Option<String>,
    payload: serde_json::Value,
    timestamp_epoch_ms: Option<i64>,
) -> Result<(Uuid, bool), BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    // Try to insert; on conflict (idempotency), return existing without error
    let result = sqlx::query_as::<_, (Uuid,)>(
        "INSERT INTO pay.webhook_provider 
         (app_id, provider, provider_webhook_id, event_type, subscription_id, purchase_token, payload, timestamp_epoch_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT DO NOTHING
         RETURNING id"
    )
    .bind(app_id)
    .bind(provider)
    .bind(provider_webhook_id)
    .bind(event_type)
    .bind(subscription_id)
    .bind(&purchase_token)
    .bind(payload)
    .bind(timestamp_epoch_ms)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to create webhook provider: {}", e)))?;

    // If we got a result, it's new. Otherwise, fetch the existing one.
    let (webhook_id, is_new) = if let Some((id,)) = result {
        (id, true)
    } else {
        // Query for the existing webhook by the provider's delivery/message id.
        let existing: (Uuid,) = sqlx::query_as(
            "SELECT id FROM pay.webhook_provider 
             WHERE app_id = $1 AND provider = $2 AND provider_webhook_id = $3
             ORDER BY created_at ASC
             LIMIT 1"
        )
        .bind(app_id)
        .bind(provider)
        .bind(provider_webhook_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| BridgeError::DbError(format!("Failed to fetch existing webhook: {}", e)))?;
        (existing.0, false)
    };

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok((webhook_id, is_new))
}

/// Create webhook delivery record
pub async fn create_webhook_delivery(
    pool: &PgPool,
    app_id: Uuid,
    webhook_provider_id: Uuid,
) -> Result<WebhookDeliveryEnqueue, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let delivery: WebhookDeliveryEnqueue = sqlx::query_as(
        "INSERT INTO pay.webhook_delivery (app_id, webhook_provider_id, forward_attempts, forwarded)
         VALUES ($1, $2, 0, false)
         ON CONFLICT (webhook_provider_id) DO UPDATE
         SET updated_at = pay.webhook_delivery.updated_at
         RETURNING id, (xmax = 0) AS created"
    )
    .bind(app_id)
    .bind(webhook_provider_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to create webhook delivery: {}", e)))?;

    tx.commit().await.map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(delivery)
}
