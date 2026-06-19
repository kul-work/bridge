use crate::utils::redact_with_prefix;
use std::time::Duration;
use crate::db::Database;
use crate::ports::{
    AppLookupRepository, ProviderConfigLookupRepository, SchedulerRepository,
    WebhookForwardRepository, WebhookProcessingRepository, WebhookWriteRepository,
};
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

            if let Err(e) = retry_webhooks(database.as_ref()).await {
                error!("Webhook retry worker failed: {}", e);
            }

            if let Err(e) = retry_google_play_subscription_acknowledgements(database.as_ref()).await {
                error!("Google Play acknowledgement retry failed: {}", e);
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

pub async fn retry_webhooks(
    repo: &(
        impl SchedulerRepository
        + WebhookForwardRepository
        + WebhookProcessingRepository
    ),
) -> Result<(), crate::error::BridgeError> {
    let apps_result = SchedulerRepository::list_enabled_app_ids(repo).await?;

    // 2. Iterate apps and retry
    for app_id in apps_result {
        let deliveries = match SchedulerRepository::list_pending_webhook_deliveries(repo, app_id, 50).await {
            Ok(deliveries) => deliveries,
            Err(e) => {
                error!(
                    "Failed to load pending webhook deliveries for app {}: {}",
                    app_id,
                    e
                );
                continue;
            }
        };

        for delivery in deliveries {
            match crate::webhooks::processor::build_canonical_payload(
                repo,
                delivery.webhook_provider_id,
                app_id,
            )
            .await
            {
                Ok(Some(canonical)) => {
                    let _ = crate::webhooks::forwarding::forward_webhook(
                        repo,
                        app_id,
                        delivery.id,
                        canonical,
                    ).await;
                }
                Ok(None) => {
                    WebhookForwardRepository::update_webhook_delivery_attempt(
                        repo,
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

async fn retry_google_play_subscription_acknowledgements(
    database: &Database,
) -> Result<(), crate::error::BridgeError> {
    let apps = SchedulerRepository::list_enabled_app_ids(database).await?;

    for app_id in apps {
        let provider_config = match database.get_provider_config(app_id, "google_play").await {
            Ok(config) => config,
            Err(_) => continue,
        };

        let candidates = crate::db::payments::list_google_play_subscription_ack_candidates(
            database.pool(),
            app_id,
            50,
        )
        .await?;

        for candidate in candidates {
            if let Err(err) = crate::services::provider_api::acknowledge_subscription(
                "google_play",
                &candidate.subscription_id,
                &candidate.purchase_token,
                &provider_config.config,
            )
            .await
            {
                warn!(
                    "Retrying Google Play subscription acknowledgement failed for app {} subscription {} token {}: {}",
                    app_id,
                    candidate.subscription_id,
                    redact_with_prefix(&candidate.purchase_token),
                    err
                );
                continue;
            }

            crate::db::payments::mark_payment_acknowledged(
                database.pool(),
                app_id,
                "google_play",
                &candidate.purchase_token,
            )
            .await?;
        }
    }

    Ok(())
}

pub async fn reconcile_subscriptions(database: &Arc<Database>) -> Result<(), crate::error::BridgeError> {
    info!("Starting subscription reconciliation job");
    
    let apps_result = SchedulerRepository::list_enabled_app_ids(database.as_ref()).await?;

    for app_id in apps_result {
        if let Err(e) = reconcile_app_subscriptions(database.as_ref(), app_id).await {
            error!("Reconciliation failed for app {}: {}", app_id, e);
            // Continue with next app, don't fail entire job
        }
    }

    info!("Subscription reconciliation job completed");
    Ok(())
}

async fn reconcile_app_subscriptions(
    repo: &(impl SchedulerRepository + WebhookForwardRepository + AppLookupRepository + ProviderConfigLookupRepository + WebhookWriteRepository),
    app_id: uuid::Uuid,
) -> Result<(), crate::error::BridgeError> {
    let active_subs = SchedulerRepository::list_reconciliation_subscriptions(repo, app_id).await?;

    for sub in active_subs {
        let provider_config = match repo.get_provider_config(app_id, &sub.provider).await {
            Ok(config) => config,
            Err(e) => {
                warn!(
                    "Skipping reconciliation for subscription {} because provider config is missing: {}",
                    sub.subscription_id,
                    e
                );
                continue;
            }
        };

        let provider_result = crate::services::provider_api::fetch_subscription_status(
            &sub.provider,
            &sub.subscription_id,
            sub.purchase_token.as_deref(),
            &provider_config.config,
        )
        .await;

        match provider_result {
            Ok((provider_status, provider_period_end)) => {
                let current_db_status = sub.status.clone();

                if current_db_status != provider_status {
                    info!(
                        "Subscription {} status drift detected: db={}, provider={}. Triggering corrective callback.",
                        sub.subscription_id, current_db_status, provider_status
                    );

                    let event_time_ms = chrono::Utc::now().timestamp_millis();
                    let updated = SchedulerRepository::update_subscription_status(
                            repo,
                            app_id,
                            &sub.subscription_id,
                            &provider_status,
                            provider_period_end,
                            event_time_ms,
                        )
                        .await?;

                    if !updated {
                        info!(
                            "Skipped stale reconciliation update for subscription {} (provider={})",
                            sub.subscription_id,
                            sub.provider,
                        );
                        continue;
                    }

                    let alert = format!(
                        "Admin alert: reconciliation drift app_id={} sub={} provider={} db_status={} provider_status={}",
                        app_id, sub.subscription_id, sub.provider, current_db_status, provider_status
                    );
                    error!("{}", alert);

                    if let Err(e) = send_reconciliation_admin_alert_email(
                        repo,
                        app_id,
                        &sub.provider,
                        &sub.subscription_id,
                        &current_db_status,
                        &provider_status,
                    )
                    .await
                    {
                        warn!(
                            "Failed to send reconciliation admin alert for subscription {}: {}",
                            sub.subscription_id,
                            e
                        );
                    }

                    if let Err(e) = emit_scheduler_callback(
                        repo,
                        app_id,
                        &sub.provider,
                        &sub.subscription_id,
                        Some(sub.external_user_id.clone()),
                        sub.purchase_token.clone(),
                        "reconciliation.drift_detected",
                        Some(provider_status.clone()),
                        Some(current_db_status),
                        Some(provider_status),
                        Some(sub.provider.clone()),
                        None,
                        sub.google_price_step_up_consent_deadline.map(|date| date.timestamp_millis()),
                        sub.google_pause_scheduled_at.map(|date| date.timestamp_millis()),
                        sub.google_deferred_until.map(|date| date.timestamp_millis()),
                    )
                    .await
                    {
                        error!("Failed to forward reconciliation callback for {}: {}", sub.subscription_id, e);
                    }
                }
            }
            Err(e) => {
                error!(
                    "Failed to fetch status for subscription {} from {}: {}",
                    sub.subscription_id, sub.provider, e
                );
            }
        }
    }

    Ok(())
}

async fn send_reconciliation_admin_alert_email(
    repo: &impl AppLookupRepository,
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

    let app = repo.get_app(app_id).await?;
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
    let expired = SchedulerRepository::list_price_step_up_expired_subscriptions(database.as_ref(), 100).await?;

    for sub in expired {
        let id = sub.id;
        let app_id = sub.app_id;
        let external_user_id = sub.external_user_id.clone();
        let subscription_id = sub.subscription_id.clone();
        let provider = sub.provider.clone();
        let purchase_token = sub.purchase_token.clone();
        let google_price_step_up_consent_deadline = sub
            .google_price_step_up_consent_deadline
            .map(|date| date.timestamp_millis());
        info!("Price step-up expired for subscription {}, auto-cancelling", subscription_id);

        if let Ok(config) = database.as_ref().get_provider_config(app_id, &provider).await {
            if let Err(e) = crate::services::provider_api::cancel_subscription(
                &provider,
                &subscription_id,
                purchase_token.as_deref(),
                None,
                None,
                &config.config,
            )
            .await
            {
                error!("Failed to cancel price step-up expired sub {}: {}", subscription_id, e);
            }
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        if !SchedulerRepository::mark_subscription_price_step_up_expired(database.as_ref(), id, now_ms).await? {
            info!(
                "Skipped price step-up expiry transition for subscription {} because it was already updated",
                subscription_id
            );
            continue;
        }

        if let Err(e) = emit_scheduler_callback(
            database.as_ref(),
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
            google_price_step_up_consent_deadline,
            sub.google_pause_scheduled_at.map(|date| date.timestamp_millis()),
            sub.google_deferred_until.map(|date| date.timestamp_millis()),
        )
        .await
        {
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
    let pending_pause = SchedulerRepository::list_pending_pause_subscriptions(database.as_ref(), 100).await?;

    for sub in pending_pause {
        let id = sub.id;
        let app_id = sub.app_id;
        let external_user_id = sub.external_user_id.clone();
        let subscription_id = sub.subscription_id.clone();
        let provider = sub.provider.clone();
        let purchase_token = sub.purchase_token.clone();
        let google_pause_scheduled_at = sub
            .google_pause_scheduled_at
            .map(|date| date.timestamp_millis());
        info!("Transitioning subscription {} to paused (scheduled pause)", subscription_id);

        let now_ms = chrono::Utc::now().timestamp_millis();
        if !SchedulerRepository::mark_subscription_paused(database.as_ref(), id, now_ms).await? {
            info!(
                "Skipped pause transition callback for subscription {} because state was already updated",
                subscription_id
            );
            continue;
        }

        if let Err(e) = emit_scheduler_callback(
            database.as_ref(),
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
            sub.google_price_step_up_consent_deadline.map(|date| date.timestamp_millis()),
            google_pause_scheduled_at,
            sub.google_deferred_until.map(|date| date.timestamp_millis()),
        )
        .await
        {
            error!("Failed to forward pause transition callback for {}: {}", subscription_id, e);
        }
    }

    let deleted = SchedulerRepository::delete_orphaned_pending_subscriptions(database.as_ref()).await?;
    if deleted > 0 {
        info!("Cleaned up {} orphaned pending subscriptions", deleted);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn emit_scheduler_callback(
    repo: &(impl WebhookForwardRepository + AppLookupRepository + WebhookWriteRepository),
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
    google_price_step_up_consent_deadline: Option<i64>,
    google_pause_scheduled_at: Option<i64>,
    google_deferred_until: Option<i64>,
) -> Result<(), crate::error::BridgeError> {
    let app = repo.get_app(app_id).await?;
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
        "google_price_step_up_consent_deadline": google_price_step_up_consent_deadline,
        "google_pause_scheduled_at": google_pause_scheduled_at,
        "google_deferred_until": google_deferred_until,
    });

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
        purchase_token: purchase_token.clone(),
        current_period_end: None,
        status,
        provider: provider.to_string(),
        provider_event_id: provider_event_id.clone(),
        previous_status,
        corrected_status,
        reconciliation_source,
        revocation_reason,
        cancellation_mode: None,
        google_price_step_up_consent_deadline,
        google_pause_scheduled_at,
        google_deferred_until,
        google_pending_price_change_new_price_cents: None,
        google_pending_price_change_currency: None,
        google_pending_price_change_mode: None,
        google_pending_price_change_state: None,
        google_pending_price_change_expected_at: None,
    };

    crate::webhooks::forwarding::create_and_forward_webhook(
        repo,
        app_id,
        provider,
        &provider_event_id,
        event_type,
        Some(subscription_id.to_string()),
        purchase_token.clone(),
        payload,
        Some(timestamp_epoch_ms),
        canonical,
    )
    .await
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

    SchedulerRepository::cleanup_old_webhook_provider(database.as_ref()).await?;
    SchedulerRepository::cleanup_purged_fraud_prevention(database.as_ref()).await?;

    info!("Data retention cleanup completed");
    Ok(())
}
