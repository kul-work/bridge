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
