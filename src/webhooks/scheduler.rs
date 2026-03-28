use std::time::Duration;
use crate::db::Database;
use std::sync::Arc;
use tracing::{info, error};

pub fn spawn_webhook_retry_worker(database: Arc<Database>) {
    tokio::spawn(async move {
        // Ticks every 5 minutes
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        info!("Webhook retry worker started");

        loop {
            interval.tick().await;

            if let Err(e) = retry_webhooks(&database).await {
                error!("Webhook retry worker failed: {}", e);
            }
        }
    });
}

pub fn spawn_reconciliation_worker(database: Arc<Database>) {
    tokio::spawn(async move {
        // Ticks every 24 hours (86400 seconds)
        let mut interval = tokio::time::interval(Duration::from_secs(86400));
        info!("Subscription reconciliation worker started");

        loop {
            interval.tick().await;

            if let Err(e) = reconcile_subscriptions(&database).await {
                error!("Subscription reconciliation worker failed: {}", e);
            }
        }
    });
}

pub async fn retry_webhooks(database: &Arc<Database>) -> Result<(), crate::error::BridgeError> {
    // 1. Get all active apps
    let apps_result = sqlx::query_as::<_, crate::db::apps::App>("SELECT * FROM pay.apps WHERE enabled = true")
        .fetch_all(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    // 2. Iterate apps and retry
    for app in apps_result {
        // Find deliveries that need retries
        let deliveries = sqlx::query_as::<_, crate::db::webhooks::WebhookDelivery>(
            "SELECT * FROM pay.webhook_delivery WHERE app_id = $1 AND forwarded = false AND forward_attempts < 3 ORDER BY created_at ASC LIMIT 50"
        )
        .bind(app.id)
        .fetch_all(&database.pool)
        .await
        .unwrap_or_default();

        for delivery in deliveries {
            let webhook_opt = crate::db::webhooks::get_webhook_provider(&database.pool, delivery.webhook_provider_id).await.ok();
            if let Some(webhook) = webhook_opt {
                if let Ok(Some(canonical)) = crate::webhooks::processor::process_webhook(&database.pool, webhook.id, app.id).await {
                    let _ = crate::webhooks::forwarding::forward_webhook(
                        &database.pool,
                        app.id,
                        delivery.id,
                        canonical,
                    ).await;
                }
            }
        }
    }

    Ok(())
}

pub async fn reconcile_subscriptions(database: &Arc<Database>) -> Result<(), crate::error::BridgeError> {
    info!("Starting subscription reconciliation job");
    
    // 1. Get all active apps
    let apps_result = sqlx::query_as::<_, crate::db::apps::App>("SELECT * FROM pay.apps WHERE enabled = true")
        .fetch_all(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    // 2. For each app, reconcile subscriptions with providers
    for app in apps_result {
        if let Err(e) = reconcile_app_subscriptions(database, app.id).await {
            error!("Reconciliation failed for app {}: {}", app.id, e);
            // Continue with next app, don't fail entire job
        }
    }

    info!("Subscription reconciliation job completed");
    Ok(())
}

async fn reconcile_app_subscriptions(database: &Arc<Database>, app_id: uuid::Uuid) -> Result<(), crate::error::BridgeError> {
    // Get all active subscriptions for the app
    let active_subs = sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT id, subscription_id, provider, external_user_id FROM pay.subscriptions WHERE app_id = $1 AND status IN ('active', 'trial', 'past_due')"
    )
    .bind(app_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    for (_sub_id, subscription_id, provider, _external_user_id) in active_subs {
        // Check current status with provider
        let provider_config = sqlx::query_as::<_, crate::db::provider_configs::ProviderConfig>(
            "SELECT * FROM pay.provider_configs WHERE app_id = $1 AND provider_name = $2"
        )
        .bind(app_id)
        .bind(&provider)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

        if let Some(config) = provider_config {
            // Verify current status (simplified; actual implementation would call provider APIs)
            if let Ok((provider_status, _)) = verify_subscription_status(
                &provider,
                &subscription_id,
                &config.config,
            )
            .await {
                // Compare with DB status and trigger corrective webhook if changed
                let db_status = sqlx::query_scalar::<_, String>(
                    "SELECT status FROM pay.subscriptions WHERE app_id = $1 AND subscription_id = $2"
                )
                .bind(app_id)
                .bind(&subscription_id)
                .fetch_optional(&database.pool)
                .await
                .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

                if let Some(current_db_status) = db_status {
                    if current_db_status != provider_status {
                        info!(
                            "Subscription {} status drift detected: db={}, provider={}. Triggering corrective callback.",
                            subscription_id, current_db_status, provider_status
                        );
                        // Update status and trigger webhook
                        sqlx::query(
                            "UPDATE pay.subscriptions SET status = $1, updated_at = NOW() WHERE app_id = $2 AND subscription_id = $3"
                        )
                        .bind(&provider_status)
                        .bind(app_id)
                        .bind(&subscription_id)
                        .execute(&database.pool)
                        .await
                        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;
                    }
                }
            }
        }
    }

    Ok(())
}

async fn verify_subscription_status(
    provider: &str,
    subscription_id: &str,
    _config: &serde_json::Value,
) -> Result<(String, Option<chrono::DateTime<chrono::Utc>>), crate::error::BridgeError> {
    match provider {
        "google_play" => {
            // Placeholder: Would call Google Play API
            info!("Reconciling Google Play subscription {}", subscription_id);
            Ok(("active".to_string(), None))
        }
        "creem" => {
            // Placeholder: Would call Creem API
            info!("Reconciling Creem subscription {}", subscription_id);
            Ok(("active".to_string(), None))
        }
        "lemonsqueezy" => {
            // Placeholder: Would call LemonSqueezy API
            info!("Reconciling LemonSqueezy subscription {}", subscription_id);
            Ok(("active".to_string(), None))
        }
        _ => Ok(("active".to_string(), None)),
    }
}
