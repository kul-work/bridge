use crate::db::database::set_local_app_id;
use crate::error::BridgeError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

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
    pub recovery_claimed_at: Option<DateTime<Utc>>,
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
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;
    set_local_app_id(&mut tx, app_id).await?;
    Ok(tx)
}

async fn get_webhook_provider_app_id(pool: &PgPool, webhook_id: Uuid) -> Result<Uuid, BridgeError> {
    sqlx::query_scalar("SELECT pay.get_webhook_provider_app_id_bootstrap($1)")
        .bind(webhook_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?
        .ok_or_else(|| BridgeError::ValidationError("Webhook not found".to_string()))
}

async fn get_webhook_delivery_app_id(
    pool: &PgPool,
    delivery_id: Uuid,
) -> Result<Uuid, BridgeError> {
    sqlx::query_scalar("SELECT pay.get_webhook_delivery_app_id_bootstrap($1)")
        .bind(delivery_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?
        .ok_or_else(|| BridgeError::ValidationError("Webhook delivery not found".to_string()))
}

/// Get webhook provider by ID
pub async fn get_webhook_provider(pool: &PgPool, id: Uuid) -> Result<WebhookProvider, BridgeError> {
    sqlx::query_as::<_, WebhookProvider>("SELECT * FROM pay.get_webhook_provider_bootstrap($1)")
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
        "UPDATE pay.webhook_provider SET suppressed = true, suppressed_reason = $1 WHERE id = $2",
    )
    .bind(reason)
    .bind(webhook_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to suppress webhook: {}", e)))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

pub async fn mark_webhook_processed(pool: &PgPool, webhook_id: Uuid) -> Result<(), BridgeError> {
    let app_id = get_webhook_provider_app_id(pool, webhook_id).await?;
    let mut tx = begin_app_tx(pool, app_id).await?;

    sqlx::query("UPDATE pay.webhook_provider SET processed = true WHERE id = $1")
        .bind(webhook_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(())
}

/// Get webhook delivery by ID
pub async fn get_webhook_delivery(pool: &PgPool, id: Uuid) -> Result<WebhookDelivery, BridgeError> {
    sqlx::query_as::<_, WebhookDelivery>("SELECT * FROM pay.get_webhook_delivery_bootstrap($1)")
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
    sqlx::query_scalar("SELECT pay.webhook_delivery_exists_bootstrap($1)")
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
    let mut tx = begin_app_tx(pool, app_id).await?;

    let deliveries = sqlx::query_as::<_, WebhookDelivery>(
        "SELECT * FROM pay.webhook_delivery
         WHERE app_id = $1 AND forwarded = false AND dead_lettered = false AND forward_attempts < 3
         ORDER BY created_at ASC
         LIMIT $2",
    )
    .bind(app_id)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(deliveries)
}

pub async fn claim_unprocessed_webhook_providers(
    pool: &PgPool,
    app_id: Uuid,
    created_before: DateTime<Utc>,
    claim_expired_before: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<WebhookProvider>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let providers = sqlx::query_as::<_, WebhookProvider>(
        "WITH candidates AS (
             SELECT id
             FROM pay.webhook_provider
             WHERE app_id = $1
               AND processed = false
               AND suppressed = false
               AND created_at < $2
               AND (
                   recovery_claimed_at IS NULL
                   OR recovery_claimed_at < $3
               )
               AND NOT EXISTS (
                   SELECT 1
                   FROM pay.webhook_delivery wd
                   WHERE wd.webhook_provider_id = pay.webhook_provider.id
               )
             ORDER BY created_at ASC
             LIMIT $4
             FOR UPDATE SKIP LOCKED
         )
         UPDATE pay.webhook_provider wp
         SET recovery_claimed_at = NOW()
         FROM candidates
         WHERE wp.id = candidates.id
         RETURNING wp.*",
    )
    .bind(app_id)
    .bind(created_before)
    .bind(claim_expired_before)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(providers)
}

/// Reset a dead-lettered webhook delivery for manual retry.
/// Returns false if the delivery is no longer eligible by the time the reset runs.
pub async fn reset_webhook_delivery(pool: &PgPool, delivery_id: Uuid) -> Result<bool, BridgeError> {
    let app_id = get_webhook_delivery_app_id(pool, delivery_id).await?;
    let mut tx = begin_app_tx(pool, app_id).await?;

    let result = sqlx::query(
        "UPDATE pay.webhook_delivery
         SET forward_attempts = 0,
             dead_lettered = false,
             dead_lettered_at = NULL,
             dead_letter_reason = NULL,
             last_error = NULL,
             updated_at = NOW()
         WHERE id = $1
           AND app_id = $2
           AND forwarded = false
           AND dead_lettered = true",
    )
    .bind(delivery_id)
    .bind(app_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to reset webhook delivery: {}", e)))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(result.rows_affected() == 1)
}

/// Update webhook delivery after forward attempt
pub async fn update_webhook_delivery_attempt(
    pool: &PgPool,
    delivery_id: Uuid,
    http_status: Option<i32>,
    error: Option<String>,
    forwarded: bool,
) -> Result<WebhookDelivery, BridgeError> {
    let app_id = get_webhook_delivery_app_id(pool, delivery_id).await?;
    let mut tx = begin_app_tx(pool, app_id).await?;

    let delivery = sqlx::query_as::<_, WebhookDelivery>(
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
         WHERE id = $4
         RETURNING *",
    )
    .bind(http_status)
    .bind(error)
    .bind(forwarded)
    .bind(delivery_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(format!("Failed to update webhook delivery: {}", e)))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(delivery)
}

/// List recent webhook deliveries for app (admin page)
pub async fn list_app_webhooks(
    pool: &PgPool,
    app_id: Uuid,
    limit: i64,
    offset: i64,
) -> Result<Vec<(WebhookDelivery, WebhookProvider)>, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    // Get deliveries first
    let deliveries = sqlx::query_as::<_, WebhookDelivery>(
        "SELECT * FROM pay.webhook_delivery 
         WHERE app_id = $1
         ORDER BY created_at DESC
         LIMIT $2 OFFSET $3",
    )
    .bind(app_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    // For each delivery, get the provider webhook
    let mut results = Vec::new();
    for delivery in deliveries {
        let provider = sqlx::query_as::<_, WebhookProvider>(
            "SELECT * FROM pay.webhook_provider WHERE id = $1",
        )
        .bind(delivery.webhook_provider_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

        if let Some(provider) = provider {
            results.push((delivery, provider));
        }
    }

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(results)
}

pub async fn get_webhook_provider_for_delivery(
    pool: &PgPool,
    delivery_id: Uuid,
) -> Result<WebhookProvider, BridgeError> {
    let app_id = get_webhook_delivery_app_id(pool, delivery_id).await?;
    let mut tx = begin_app_tx(pool, app_id).await?;

    let provider = sqlx::query_as::<_, WebhookProvider>(
        "SELECT wp.*
         FROM pay.webhook_delivery wd
         JOIN pay.webhook_provider wp ON wp.id = wd.webhook_provider_id
         WHERE wd.id = $1 AND wd.app_id = $2",
    )
    .bind(delivery_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?
    .ok_or_else(|| BridgeError::ValidationError("Webhook delivery not found".to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(provider)
}

/// Count failed webhook deliveries for app
pub async fn count_failed_webhooks(pool: &PgPool, app_id: Uuid) -> Result<i64, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pay.webhook_delivery 
         WHERE app_id = $1 AND (forwarded = false OR last_http_status >= 400)",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(count.0)
}

pub async fn count_dead_lettered_webhooks(pool: &PgPool, app_id: Uuid) -> Result<i64, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pay.webhook_delivery
         WHERE app_id = $1 AND dead_lettered = true",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(count.0)
}

pub async fn count_retryable_failed_webhooks(
    pool: &PgPool,
    app_id: Uuid,
) -> Result<i64, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pay.webhook_delivery
         WHERE app_id = $1
           AND forwarded = false
           AND dead_lettered = false
           AND forward_attempts > 0",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(count.0)
}

pub async fn count_reconciliation_drift_callbacks(
    pool: &PgPool,
    app_id: Uuid,
) -> Result<i64, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)
         FROM pay.webhook_provider
         WHERE app_id = $1 AND event_type = 'reconciliation.drift_detected'",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(count.0)
}

/// Count all webhook deliveries for app
pub async fn count_app_webhooks(pool: &PgPool, app_id: Uuid) -> Result<i64, BridgeError> {
    let mut tx = begin_app_tx(pool, app_id).await?;

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pay.webhook_delivery 
         WHERE app_id = $1",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
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
    let mut tx = begin_app_tx(pool, app_id).await?;

    let records = sqlx::query_as::<_, WebhookRecord>(
        "SELECT provider, event_type, payload, created_at
         FROM pay.webhook_provider
         WHERE app_id = $1
           AND (subscription_id = ANY($2) OR purchase_token = ANY($3))
         ORDER BY created_at DESC",
    )
    .bind(app_id)
    .bind(subscription_ids)
    .bind(purchase_tokens)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(records)
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
             LIMIT 1",
        )
        .bind(app_id)
        .bind(provider)
        .bind(provider_webhook_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| BridgeError::DbError(format!("Failed to fetch existing webhook: {}", e)))?;
        (existing.0, false)
    };

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

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

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok(delivery)
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{Router, extract::State, http::StatusCode, routing::post};
    use sqlx::PgPool;
    use tokio::{net::TcpListener, task::JoinHandle};

    use super::*;
    use crate::webhooks::processor::CanonicalWebhookPayload;

    #[tokio::test]
    async fn manual_retry_reset_does_not_reopen_forwarded_deliveries() -> Result<(), Box<dyn Error>>
    {
        let Some(database) = test_database().await? else {
            eprintln!("skipping DB-backed webhook retry regression; set BRIDGE_TEST_DATABASE_URL");
            return Ok(());
        };

        let pool = database.pool();
        let (callback_url, callback_count, server) = spawn_callback_server().await?;
        let app_id = insert_test_app(pool, &callback_url).await?;
        let result =
            run_manual_retry_reset_regression(&database, pool, app_id, callback_count).await;

        delete_test_app(pool, app_id).await;
        server.abort();

        result
    }

    #[tokio::test]
    async fn provider_inbox_recovery_selectors_find_only_old_unprocessed_rows()
    -> Result<(), Box<dyn Error>> {
        let Some(database) = test_database().await? else {
            eprintln!("skipping DB-backed webhook inbox regression; set BRIDGE_TEST_DATABASE_URL");
            return Ok(());
        };

        let pool = database.pool();
        let app_id = insert_test_app(pool, "http://127.0.0.1:1/callback").await?;
        let result = run_provider_inbox_selector_regression(pool, app_id).await;

        delete_test_app(pool, app_id).await;

        result
    }

    async fn run_manual_retry_reset_regression(
        database: &crate::db::Database,
        pool: &PgPool,
        app_id: Uuid,
        callback_count: Arc<AtomicUsize>,
    ) -> Result<(), Box<dyn Error>> {
        let forwarded_delivery_id = insert_test_delivery(pool, app_id, 2, true, false).await?;
        let forwarded_before = super::get_webhook_delivery(pool, forwarded_delivery_id).await?;
        let reset_forwarded = super::reset_webhook_delivery(pool, forwarded_delivery_id).await?;
        let forwarded_after = super::get_webhook_delivery(pool, forwarded_delivery_id).await?;

        assert!(!reset_forwarded);
        assert!(forwarded_after.forwarded);
        assert_eq!(
            forwarded_after.forward_attempts,
            forwarded_before.forward_attempts
        );
        assert_eq!(forwarded_after.forwarded_at, forwarded_before.forwarded_at);
        assert_eq!(
            forwarded_after.dead_lettered,
            forwarded_before.dead_lettered
        );

        let dead_lettered_delivery_id = insert_test_delivery(pool, app_id, 3, false, true).await?;
        let reset_dead_lettered =
            super::reset_webhook_delivery(pool, dead_lettered_delivery_id).await?;
        let dead_lettered_after =
            super::get_webhook_delivery(pool, dead_lettered_delivery_id).await?;

        assert!(reset_dead_lettered);
        assert!(!dead_lettered_after.forwarded);
        assert_eq!(dead_lettered_after.forward_attempts, 0);
        assert!(!dead_lettered_after.dead_lettered);
        assert!(dead_lettered_after.dead_lettered_at.is_none());
        assert!(dead_lettered_after.dead_letter_reason.is_none());
        assert!(dead_lettered_after.last_error.is_none());

        let pending_delivery_id = insert_test_delivery(pool, app_id, 0, false, false).await?;
        let payload = test_canonical_payload();

        let (forward_result, retry_result) = tokio::join!(
            crate::webhooks::forwarding::forward_webhook(
                database,
                app_id,
                pending_delivery_id,
                payload,
            ),
            super::reset_webhook_delivery(pool, pending_delivery_id),
        );

        forward_result?;
        assert!(!retry_result?);
        assert_eq!(callback_count.load(Ordering::SeqCst), 1);

        let pending_after = super::get_webhook_delivery(pool, pending_delivery_id).await?;
        assert!(pending_after.forwarded);
        assert!(pending_after.forwarded_at.is_some());
        assert_eq!(pending_after.forward_attempts, 1);
        assert!(!pending_after.dead_lettered);
        assert_eq!(pending_after.last_http_status, Some(200));

        Ok(())
    }

    async fn run_provider_inbox_selector_regression(
        pool: &PgPool,
        app_id: Uuid,
    ) -> Result<(), Box<dyn Error>> {
        let old_unprocessed_id = insert_test_provider(pool, app_id, false, false).await?;
        make_provider_old(pool, old_unprocessed_id).await?;
        let fresh_unprocessed_id = insert_test_provider(pool, app_id, false, false).await?;
        let suppressed_id = insert_test_provider(pool, app_id, false, true).await?;
        let processed_missing_delivery_id = insert_test_provider(pool, app_id, true, false).await?;
        let processed_with_delivery_id = insert_test_delivery(pool, app_id, 0, false, false).await?;
        let processed_with_delivery = super::get_webhook_delivery(pool, processed_with_delivery_id).await?;

        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(300);
        let claim_expired_before = now - chrono::Duration::seconds(600);
        let unprocessed =
            super::claim_unprocessed_webhook_providers(pool, app_id, cutoff, claim_expired_before, 50).await?;
        let unprocessed_ids: Vec<Uuid> = unprocessed.iter().map(|webhook| webhook.id).collect();
        assert!(unprocessed_ids.contains(&old_unprocessed_id));
        assert!(!unprocessed_ids.contains(&fresh_unprocessed_id));
        assert!(!unprocessed_ids.contains(&suppressed_id));
        assert!(!unprocessed_ids.contains(&processed_missing_delivery_id));
        assert!(!unprocessed_ids.contains(&processed_with_delivery.webhook_provider_id));

        let second_claim =
            super::claim_unprocessed_webhook_providers(pool, app_id, cutoff, claim_expired_before, 50).await?;
        assert!(second_claim.is_empty());

        make_provider_claim_old(pool, old_unprocessed_id).await?;
        let reclaimed =
            super::claim_unprocessed_webhook_providers(pool, app_id, cutoff, claim_expired_before, 50).await?;
        let reclaimed_ids: Vec<Uuid> = reclaimed.iter().map(|webhook| webhook.id).collect();
        assert!(reclaimed_ids.contains(&old_unprocessed_id));

        Ok(())
    }

    async fn test_database() -> Result<Option<crate::db::Database>, Box<dyn Error>> {
        dotenvy::dotenv().ok();
        let admin_database_url = std::env::var("ADMIN_DATABASE_URL").ok();
        let environment = std::env::var("ENVIRONMENT")
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_production = matches!(environment.as_str(), "production" | "prod");
        let database_url = match std::env::var("BRIDGE_TEST_DATABASE_URL") {
            Ok(url) => Some(url),
            Err(_) if !is_production => admin_database_url
                .clone()
                .or_else(|| std::env::var("DATABASE_URL").ok()),
            Err(_) => None,
        };
        let Some(database_url) = database_url else {
            return Ok(None);
        };

        Ok(Some(
            crate::db::Database::new(&database_url, admin_database_url.as_deref()).await?,
        ))
    }

    async fn insert_test_app(pool: &PgPool, callback_url: &str) -> Result<Uuid, sqlx::Error> {
        let app_id = Uuid::new_v4();
        let slug = format!("manual-retry-reset-{}", app_id);

        sqlx::query(
            "INSERT INTO pay.apps (id, slug, display_name, webhook_callback_url, webhook_callback_secret)
             VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(app_id)
        .bind(slug)
        .bind("Manual Retry Reset Regression")
        .bind(callback_url)
        .bind("test_callback_secret")
        .execute(pool)
        .await?;

        Ok(app_id)
    }

    async fn insert_test_delivery(
        pool: &PgPool,
        app_id: Uuid,
        forward_attempts: i32,
        forwarded: bool,
        dead_lettered: bool,
    ) -> Result<Uuid, sqlx::Error> {
        let provider_id = Uuid::new_v4();
        let delivery_id = Uuid::new_v4();
        let forwarded_at = forwarded.then(Utc::now);
        let dead_lettered_at = dead_lettered.then(Utc::now);
        let dead_letter_reason = dead_lettered.then(|| "Retry limit exceeded".to_string());
        let last_http_status = if forwarded {
            Some(200)
        } else if dead_lettered {
            Some(500)
        } else {
            None
        };
        let last_error = dead_lettered.then(|| "previous delivery failure".to_string());

        sqlx::query(
            "INSERT INTO pay.webhook_provider
             (id, app_id, provider, provider_webhook_id, event_type, payload, processed)
             VALUES ($1, $2, $3, $4, $5, $6, true)",
        )
        .bind(provider_id)
        .bind(app_id)
        .bind("test_provider")
        .bind(format!("evt_{}", provider_id))
        .bind("subscription.test")
        .bind(serde_json::json!({ "test": true }))
        .execute(pool)
        .await?;

        sqlx::query(
            "INSERT INTO pay.webhook_delivery
             (id, app_id, webhook_provider_id, forward_attempts, forwarded, forwarded_at,
              dead_lettered, dead_lettered_at, dead_letter_reason, last_http_status, last_error)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(delivery_id)
        .bind(app_id)
        .bind(provider_id)
        .bind(forward_attempts)
        .bind(forwarded)
        .bind(forwarded_at)
        .bind(dead_lettered)
        .bind(dead_lettered_at)
        .bind(dead_letter_reason)
        .bind(last_http_status)
        .bind(last_error)
        .execute(pool)
        .await?;

        Ok(delivery_id)
    }

    async fn insert_test_provider(
        pool: &PgPool,
        app_id: Uuid,
        processed: bool,
        suppressed: bool,
    ) -> Result<Uuid, sqlx::Error> {
        let provider_id = Uuid::new_v4();

        sqlx::query(
            "INSERT INTO pay.webhook_provider
             (id, app_id, provider, provider_webhook_id, event_type, payload, processed, suppressed)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(provider_id)
        .bind(app_id)
        .bind("test_provider")
        .bind(format!("evt_{}", provider_id))
        .bind("subscription.test")
        .bind(serde_json::json!({ "test": true }))
        .bind(processed)
        .bind(suppressed)
        .execute(pool)
        .await?;

        Ok(provider_id)
    }

    async fn make_provider_old(pool: &PgPool, provider_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE pay.webhook_provider
             SET created_at = NOW() - INTERVAL '10 minutes'
             WHERE id = $1",
        )
        .bind(provider_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    async fn make_provider_claim_old(pool: &PgPool, provider_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE pay.webhook_provider
             SET recovery_claimed_at = NOW() - INTERVAL '20 minutes'
             WHERE id = $1",
        )
        .bind(provider_id)
        .execute(pool)
        .await?;

        Ok(())
    }

    async fn delete_test_app(pool: &PgPool, app_id: Uuid) {
        let _ = sqlx::query("DELETE FROM pay.apps WHERE id = $1")
            .bind(app_id)
            .execute(pool)
            .await;
    }

    async fn callback_handler(State(count): State<Arc<AtomicUsize>>) -> StatusCode {
        count.fetch_add(1, Ordering::SeqCst);
        StatusCode::OK
    }

    async fn spawn_callback_server()
    -> Result<(String, Arc<AtomicUsize>, JoinHandle<()>), Box<dyn Error>> {
        let count = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/callback", post(callback_handler))
            .with_state(count.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test callback server should stay healthy");
        });

        Ok((format!("http://{}/callback", address), count, server))
    }

    fn test_canonical_payload() -> CanonicalWebhookPayload {
        let now = Utc::now();

        CanonicalWebhookPayload {
            event_id: format!("test-event-{}", Uuid::new_v4()),
            event_type: "subscription.test".to_string(),
            timestamp: now.to_rfc3339(),
            timestamp_epoch_ms: now.timestamp_millis(),
            app_slug: "manual-retry-reset-regression".to_string(),
            product_id: None,
            subscription_id: None,
            external_user_id: Some("user_manual_retry_reset".to_string()),
            amount_cents: None,
            new_price_cents: None,
            auto_renewing: None,
            purchase_token: None,
            current_period_end: None,
            status: None,
            provider: "test_provider".to_string(),
            provider_event_id: format!("evt_{}", Uuid::new_v4()),
            previous_status: None,
            corrected_status: None,
            reconciliation_source: None,
            revocation_reason: None,
            cancellation_mode: None,
            google_price_step_up_consent_deadline: None,
            google_pause_scheduled_at: None,
            google_deferred_until: None,
            google_pending_price_change_new_price_cents: None,
            google_pending_price_change_currency: None,
            google_pending_price_change_mode: None,
            google_pending_price_change_state: None,
            google_pending_price_change_expected_at: None,
        }
    }
}
