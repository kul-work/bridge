use std::time::Duration;
use crate::db::Database;
use std::sync::Arc;
use tracing::{info, error, warn};
use uuid::Uuid;

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
            "SELECT * FROM pay.webhook_delivery WHERE app_id = $1 AND forwarded = false AND dead_lettered = false AND forward_attempts < 3 ORDER BY created_at ASC LIMIT 50"
        )
        .bind(app.id)
        .fetch_all(&database.pool)
        .await
        .unwrap_or_default();

        for delivery in deliveries {
            match crate::webhooks::processor::build_canonical_payload(
                database.as_ref(),
                delivery.webhook_provider_id,
                app.id,
            )
            .await
            {
                Ok(Some(canonical)) => {
                    let _ = crate::webhooks::forwarding::forward_webhook(
                        &database.pool,
                        app.id,
                        delivery.id,
                        canonical,
                    ).await;
                }
                Ok(None) => {
                    crate::db::webhooks::update_webhook_delivery_attempt(
                        &database.pool,
                        delivery.id,
                        None,
                        Some("Suppressed before retry".to_string()),
                        true,
                    )
                    .await?;
                }
                Err(e) => {
                    error!("Failed to rebuild canonical webhook payload for delivery {}: {}", delivery.id, e);
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
    let active_subs = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
        "SELECT id::text, subscription_id, provider, external_user_id, purchase_token FROM pay.subscriptions WHERE app_id = $1 AND status IN ('active', 'trial', 'past_due')"
    )
    .bind(app_id)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    for (_sub_id, subscription_id, provider, external_user_id, purchase_token) in active_subs {
        // Check current status with provider
        let provider_config = sqlx::query_as::<_, crate::db::provider_configs::ProviderConfig>(
            "SELECT * FROM pay.provider_configs WHERE app_id = $1 AND provider = $2"
        )
        .bind(app_id)
        .bind(&provider)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

        if let Some(config) = provider_config {
            // Call provider API to get current subscription status
            let provider_result = crate::services::provider_api::fetch_subscription_status(
                &provider,
                &subscription_id,
                purchase_token.as_deref(),
                &config.config,
            ).await;

            match provider_result {
                Ok((provider_status, _)) => {
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
                            let event_time_ms = chrono::Utc::now().timestamp_millis();
                            let updated = crate::db::subscriptions::update_subscription_status(
                                &database.pool,
                                app_id,
                                &subscription_id,
                                &provider_status,
                                event_time_ms,
                            )
                            .await?;

                            if !updated {
                                info!(
                                    "Skipped stale reconciliation update for subscription {} (provider={})",
                                    subscription_id,
                                    provider,
                                );
                                continue;
                            }

                            let alert = format!(
                                "Admin alert: reconciliation drift app_id={} sub={} provider={} db_status={} provider_status={}",
                                app_id, subscription_id, provider, current_db_status, provider_status
                            );
                            error!("{}", alert);

                            if let Err(e) = send_reconciliation_admin_alert_email(
                                &database.pool,
                                app_id,
                                &provider,
                                &subscription_id,
                                &current_db_status,
                                &provider_status,
                            ).await {
                                warn!(
                                    "Failed to send reconciliation admin alert for subscription {}: {}",
                                    subscription_id,
                                    e
                                );
                            }

                            if let Err(e) = emit_scheduler_callback(
                                &database.pool,
                                app_id,
                                &provider,
                                &subscription_id,
                                Some(external_user_id.clone()),
                                purchase_token.clone(),
                                "reconciliation.drift_detected",
                                Some(provider_status.clone()),
                                Some(current_db_status),
                                Some(provider_status),
                                Some(provider.clone()),
                                None,
                            ).await {
                                error!("Failed to forward reconciliation callback for {}: {}", subscription_id, e);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to fetch status for subscription {} from {}: {}", subscription_id, provider, e);
                    // Skip this subscription, don't fail the whole job
                }
            }
        }
    }

    Ok(())
}

async fn send_reconciliation_admin_alert_email(
    pool: &sqlx::PgPool,
    app_id: uuid::Uuid,
    provider: &str,
    subscription_id: &str,
    current_db_status: &str,
    provider_status: &str,
) -> Result<(), crate::error::BridgeError> {
    let admin_email = match std::env::var("ADMIN_ALERT_EMAIL")
        .or_else(|_| std::env::var("TYDE_SUPPORT_EMAIL"))
    {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            warn!(
                "Skipping reconciliation admin email for subscription {}: ADMIN_ALERT_EMAIL not configured",
                subscription_id
            );
            return Ok(());
        }
    };

    let app = crate::db::apps::get_app(pool, app_id).await?;
    let subject = format!(
        "Bridge reconciliation drift: {} ({})",
        app.display_name,
        subscription_id
    );
    let body = format!(
        "A subscription reconciliation drift was detected.\n\n\
         App: {}\n\
         App slug: {}\n\
         App ID: {}\n\
         Provider: {}\n\
         Subscription ID: {}\n\
         Database status: {}\n\
         Provider status: {}\n",
        app.display_name,
        app.slug,
        app.id,
        provider,
        subscription_id,
        current_db_status,
        provider_status,
    );

    crate::services::email::send_email(&admin_email, &subject, &body)
        .await
        .map_err(|e| crate::error::BridgeError::InternalServerError(format!(
            "Failed to send reconciliation admin alert email: {}",
            e
        )))?;

    info!(
        "Reconciliation admin email sent for app_id={} subscription_id={}",
        app_id,
        subscription_id
    );

    Ok(())
}

pub fn spawn_price_step_up_expiry_worker(database: Arc<Database>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300)); // 5 minutes
        info!("Price step-up expiry worker started");

        loop {
            interval.tick().await;

            if let Err(e) = process_price_step_up_expiry(&database).await {
                error!("Price step-up expiry worker failed: {}", e);
            }
        }
    });
}

async fn process_price_step_up_expiry(database: &Arc<Database>) -> Result<(), crate::error::BridgeError> {
    let expired = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, String, String, Option<String>)>(
        "SELECT id, app_id, external_user_id, subscription_id, provider, purchase_token 
         FROM pay.subscriptions 
         WHERE google_requires_price_step_up_consent = true 
           AND google_price_step_up_consent_deadline IS NOT NULL 
           AND google_price_step_up_consent_deadline < NOW()
         LIMIT 100"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    for (id, app_id, external_user_id, subscription_id, provider, purchase_token) in expired {
        info!("Price step-up expired for subscription {}, auto-cancelling", subscription_id);

        // Try to cancel with provider
        let provider_config = sqlx::query_as::<_, crate::db::provider_configs::ProviderConfig>(
            "SELECT * FROM pay.provider_configs WHERE app_id = $1 AND provider = $2"
        )
        .bind(app_id)
        .bind(&provider)
        .fetch_optional(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

        if let Some(config) = provider_config {
            if let Err(e) = crate::services::provider_api::cancel_subscription(
                &provider,
                &subscription_id,
                purchase_token.as_deref(),
                None,
                &config.config,
            ).await {
                error!("Failed to cancel price step-up expired sub {}: {}", subscription_id, e);
            }
        }

        // Update local DB regardless (clear consent flags, set auto_renewing=false)
        let now_ms = chrono::Utc::now().timestamp_millis();
        let result = sqlx::query(
            "UPDATE pay.subscriptions 
             SET google_requires_price_step_up_consent = false,
                 google_price_step_up_consent_deadline = NULL,
                 status = 'cancelled',
                 revocation_reason = 'price_step_up_expiry',
                 auto_renewing = false,
                 version = version + 1,
                 last_event_time = CASE WHEN last_event_time < $1 THEN $1 ELSE last_event_time END,
                 updated_at = NOW()
             WHERE id = $2"
        )
        .bind(now_ms)
        .bind(id)
        .execute(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

        if result.rows_affected() == 0 {
            info!("Skipped price step-up expiry transition for subscription {} because it was already updated", subscription_id);
            continue;
        }

        if let Err(e) = emit_scheduler_callback(
            &database.pool,
            app_id,
            &provider,
            &subscription_id,
            Some(external_user_id),
            purchase_token.clone(),
            "subscription.cancelled",
            Some("cancelled".to_string()),
            None,
            None,
            None,
            Some("price_step_up_expiry".to_string()),
        ).await {
            error!("Failed to forward price step-up expiry callback for {}: {}", subscription_id, e);
        }
    }

    Ok(())
}

pub fn spawn_pause_scheduler_worker(database: Arc<Database>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1500)); // 25 minutes
        info!("Pause scheduler worker started");

        loop {
            interval.tick().await;

            if let Err(e) = process_pause_transitions(&database).await {
                error!("Pause scheduler worker failed: {}", e);
            }
        }
    });
}

async fn process_pause_transitions(database: &Arc<Database>) -> Result<(), crate::error::BridgeError> {
    // 1. Pause transition: subscriptions scheduled to pause
    let pending_pause = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, String, String, String, Option<String>)>(
        "SELECT id, app_id, external_user_id, subscription_id, provider, purchase_token FROM pay.subscriptions 
         WHERE google_pause_scheduled_at IS NOT NULL 
           AND google_pause_scheduled_at <= NOW() 
           AND status != 'paused'
         LIMIT 100"
    )
    .fetch_all(&database.pool)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    for (id, app_id, external_user_id, subscription_id, provider, purchase_token) in pending_pause {
        info!("Transitioning subscription {} to paused (scheduled pause)", subscription_id);

        let now_ms = chrono::Utc::now().timestamp_millis();
        let result = sqlx::query(
            "UPDATE pay.subscriptions 
             SET status = 'paused',
                 auto_renewing = false,
                 google_paused_at = NOW(),
                 version = version + 1,
                 last_event_time = CASE WHEN last_event_time < $1 THEN $1 ELSE last_event_time END,
                 updated_at = NOW()
             WHERE id = $2 AND status != 'paused'"
        )
        .bind(now_ms)
        .bind(id)
        .execute(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

        if result.rows_affected() == 0 {
            info!("Skipped pause transition callback for subscription {} because state was already updated", subscription_id);
            continue;
        }

        if let Err(e) = emit_scheduler_callback(
            &database.pool,
            app_id,
            &provider,
            &subscription_id,
            Some(external_user_id),
            purchase_token,
            "subscription.paused",
            Some("paused".to_string()),
            None,
            None,
            None,
            None,
        ).await {
            error!("Failed to forward pause transition callback for {}: {}", subscription_id, e);
        }
    }

    // 2. Orphaned pending cleanup: remove stale register_purchase placeholders
    let deleted = sqlx::query(
        "DELETE FROM pay.subscriptions 
         WHERE status = 'pending' 
           AND purchase_token IS NULL 
           AND created_at < NOW() - INTERVAL '30 minutes'"
    )
    .execute(&database.pool)
    .await
    .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    if deleted.rows_affected() > 0 {
        info!("Cleaned up {} orphaned pending subscriptions", deleted.rows_affected());
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn emit_scheduler_callback(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
    subscription_id: &str,
    external_user_id: Option<String>,
    purchase_token: Option<String>,
    event_type: &str,
    status: Option<String>,
    previous_status: Option<String>,
    corrected_status: Option<String>,
    reconciliation_source: Option<String>,
    revocation_reason: Option<String>,
) -> Result<(), crate::error::BridgeError> {
    let app = crate::db::apps::get_app(pool, app_id).await?;
    let provider_event_id = format!("scheduler-{}", Uuid::new_v4());
    let timestamp_epoch_ms = chrono::Utc::now().timestamp_millis();
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    let payload = serde_json::json!({
        "source": "scheduler",
        "event_type": event_type,
        "subscription_id": subscription_id,
        "external_user_id": external_user_id,
        "provider": provider,
        "status": status,
        "previous_status": previous_status,
        "corrected_status": corrected_status,
        "reconciliation_source": reconciliation_source,
        "revocation_reason": revocation_reason,
    });

    let (webhook_provider_id, _) = crate::db::webhooks::create_webhook_provider(
        pool,
        app_id,
        provider,
        &provider_event_id,
        event_type,
        Some(subscription_id.to_string()),
        purchase_token.clone(),
        payload,
        Some(timestamp_epoch_ms),
    )
    .await?;

    let delivery_id = crate::db::webhooks::create_webhook_delivery(pool, app_id, webhook_provider_id).await?;

    let canonical = crate::webhooks::processor::CanonicalWebhookPayload {
        event_id: format!("{}-{}", provider, provider_event_id),
        event_type: event_type.to_string(),
        timestamp,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: None,
        subscription_id: Some(subscription_id.to_string()),
        external_user_id,
        amount_cents: None,
        new_price_cents: None,
        auto_renewing: None,
        purchase_token,
        current_period_end: None,
        status,
        provider: provider.to_string(),
        provider_event_id,
        previous_status,
        corrected_status,
        reconciliation_source,
        revocation_reason,
        cancellation_mode: None,
    };

    crate::webhooks::forwarding::forward_webhook(pool, app_id, delivery_id, canonical).await
}

pub fn spawn_webhook_cleanup_worker(database: Arc<Database>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(86400)); // daily
        info!("Webhook log cleanup worker started");

        loop {
            interval.tick().await;

            if let Err(e) = cleanup_old_data(&database).await {
                error!("Webhook log cleanup worker failed: {}", e);
            }
        }
    });
}

async fn cleanup_old_data(database: &Arc<Database>) -> Result<(), crate::error::BridgeError> {
    info!("Starting data retention cleanup");

    // §49: Webhook log cleanup (90-day retention)
    sqlx::query("SELECT pay.cleanup_old_webhook_provider()")
        .execute(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    // Also clean up expired agent tokens and fraud prevention records
    sqlx::query("SELECT pay.cleanup_expired_agent_tokens()")
        .execute(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    sqlx::query("SELECT pay.cleanup_purged_fraud_prevention()")
        .execute(&database.pool)
        .await
        .map_err(|e| crate::error::BridgeError::DbError(e.to_string()))?;

    info!("Data retention cleanup completed");
    Ok(())
}
