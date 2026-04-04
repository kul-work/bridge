use crate::db;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::services::provider_api;
use axum::{
    extract::{State, Extension, Path},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CancelSubscriptionRequest {
    pub external_user_id: String,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionActionResponse {
    pub success: bool,
    pub message: String,
}

pub async fn cancel_subscription(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<CancelSubscriptionRequest>,
) -> Result<(StatusCode, Json<SubscriptionActionResponse>), BridgeError> {
    let sub = db::subscriptions::get_subscription_by_sub_id(
        &database.pool,
        auth.app_id,
        &subscription_id,
    )
    .await?
    .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    let provider_config = db::provider_configs::get_provider_config(
        &database.pool,
        auth.app_id,
        &sub.provider,
    ).await?;

    provider_api::cancel_subscription(
        &sub.provider,
        &sub.subscription_id,
        sub.purchase_token.as_deref(),
        request.mode.as_deref(),
        &provider_config.config,
    ).await?;

    let mode = request.mode.as_deref().unwrap_or("scheduled");
    let (updated_sub, event_type, message) = match mode {
        "scheduled" => {
            let updated = sqlx::query_as::<_, db::subscriptions::Subscription>(
                "UPDATE pay.subscriptions
                 SET auto_renewing = false, cancellation_initiated_at = NOW(), updated_at = NOW()
                 WHERE id = $1
                 RETURNING *",
            )
            .bind(sub.id)
            .fetch_one(&database.pool)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;
            (
                updated,
                "subscription.cancellation_scheduled",
                "Subscription cancellation requested".to_string(),
            )
        }
        "immediate" => {
            let updated = sqlx::query_as::<_, db::subscriptions::Subscription>(
                "UPDATE pay.subscriptions
                 SET status = 'cancelled', auto_renewing = false, cancellation_initiated_at = NOW(), current_period_end = NOW(), updated_at = NOW()
                 WHERE id = $1
                 RETURNING *",
            )
            .bind(sub.id)
            .fetch_one(&database.pool)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;
            (
                updated,
                "subscription.cancelled",
                "Subscription cancelled immediately".to_string(),
            )
        }
        _ => {
            return Err(BridgeError::ValidationError(
                "mode must be either 'scheduled' or 'immediate'".to_string(),
            ))
        }
    };

    if let Err(e) = dispatch_subscription_callback(
        &database.pool,
        auth.app_id,
        &updated_sub,
        event_type,
    )
    .await
    {
        warn!("Failed to forward cancel callback for {}: {}", updated_sub.subscription_id, e);
    }

    Ok((
        StatusCode::OK,
        Json(SubscriptionActionResponse {
            success: true,
            message,
        }),
    ))
}

pub async fn resume_subscription(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<CancelSubscriptionRequest>,
) -> Result<(StatusCode, Json<SubscriptionActionResponse>), BridgeError> {
    let sub = db::subscriptions::get_subscription_by_sub_id(
        &database.pool,
        auth.app_id,
        &subscription_id,
    )
    .await?
    .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    let provider_config = db::provider_configs::get_provider_config(
        &database.pool,
        auth.app_id,
        &sub.provider,
    ).await?;

    provider_api::resume_subscription(
        &sub.provider,
        &sub.subscription_id,
        &provider_config.config,
    ).await?;

    let updated_sub = sqlx::query_as::<_, db::subscriptions::Subscription>(
        "UPDATE pay.subscriptions
         SET status = 'active', auto_renewing = true, cancellation_initiated_at = NULL, updated_at = NOW()
         WHERE id = $1
         RETURNING *",
    )
    .bind(sub.id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    if let Err(e) = dispatch_subscription_callback(
        &database.pool,
        auth.app_id,
        &updated_sub,
        "subscription.resumed",
    )
    .await
    {
        warn!("Failed to forward resume callback for {}: {}", updated_sub.subscription_id, e);
    }

    Ok((
        StatusCode::OK,
        Json(SubscriptionActionResponse {
            success: true,
            message: "Subscription resumed".to_string(),
        }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct AcknowledgeRequest {
    pub external_user_id: String,
}

pub async fn acknowledge_subscription(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<AcknowledgeRequest>,
) -> Result<(StatusCode, Json<SubscriptionActionResponse>), BridgeError> {
    let sub = db::subscriptions::get_subscription_by_sub_id(
        &database.pool,
        auth.app_id,
        &subscription_id,
    )
    .await?
    .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    if let Some(purchase_token) = sub.purchase_token.as_deref() {
        sqlx::query(
            "UPDATE pay.payments
             SET acknowledged_at = COALESCE(acknowledged_at, NOW())
             WHERE app_id = $1 AND external_user_id = $2 AND provider = $3 AND provider_transaction_id = $4",
        )
        .bind(auth.app_id)
        .bind(&sub.external_user_id)
        .bind(&sub.provider)
        .bind(purchase_token)
        .execute(&database.pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;
    } else {
        sqlx::query(
            "UPDATE pay.payments
             SET acknowledged_at = COALESCE(acknowledged_at, NOW())
             WHERE app_id = $1 AND external_user_id = $2 AND provider = $3 AND subscription_id = $4",
        )
        .bind(auth.app_id)
        .bind(&sub.external_user_id)
        .bind(&sub.provider)
        .bind(&sub.subscription_id)
        .execute(&database.pool)
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;
    }

    Ok((
        StatusCode::OK,
        Json(SubscriptionActionResponse {
            success: true,
            message: "Subscription acknowledged".to_string(),
        }),
    ))
}

#[derive(Debug, Serialize)]
pub struct BillingPortalResponse {
    pub portal_url: String,
}

pub async fn create_billing_portal(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<AcknowledgeRequest>,
) -> Result<(StatusCode, Json<BillingPortalResponse>), BridgeError> {
    let sub = db::subscriptions::get_subscription_by_sub_id(
        &database.pool,
        auth.app_id,
        &subscription_id,
    )
    .await?
    .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    let customer_id = sub.provider_customer_id.as_deref()
        .ok_or_else(|| BridgeError::ValidationError("Provider customer ID not available for this subscription".to_string()))?;

    let provider_config = db::provider_configs::get_provider_config(
        &database.pool,
        auth.app_id,
        &sub.provider,
    ).await?;

    let portal_url = provider_api::create_billing_portal(
        &sub.provider,
        customer_id,
        &provider_config.config,
    ).await?;

    Ok((
        StatusCode::OK,
        Json(BillingPortalResponse { portal_url }),
    ))
}

#[derive(Debug, Deserialize)]
pub struct PriceStepUpRequest {
    pub external_user_id: String,
}

pub async fn accept_price_step_up(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<PriceStepUpRequest>,
) -> Result<(StatusCode, Json<SubscriptionActionResponse>), BridgeError> {
    let sub = db::subscriptions::get_subscription_by_sub_id(
        &database.pool,
        auth.app_id,
        &subscription_id,
    )
    .await?
    .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    if sub.provider != "google_play" {
        return Err(BridgeError::ValidationError(
            "Price step-up actions are only supported for Google Play subscriptions".to_string(),
        ));
    }

    sqlx::query(
        "UPDATE pay.subscriptions
         SET google_requires_price_step_up_consent = false,
             google_price_step_up_consent_status = 'accepted',
             google_price_step_up_consent_deadline = NULL,
             updated_at = NOW()
         WHERE id = $1",
    )
    .bind(sub.id)
    .execute(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(SubscriptionActionResponse {
            success: true,
            message: "Price step-up accepted".to_string(),
        }),
    ))
}

pub async fn decline_price_step_up(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<PriceStepUpRequest>,
) -> Result<(StatusCode, Json<SubscriptionActionResponse>), BridgeError> {
    let sub = db::subscriptions::get_subscription_by_sub_id(
        &database.pool,
        auth.app_id,
        &subscription_id,
    )
    .await?
    .ok_or_else(|| BridgeError::SubscriptionNotFound("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    if sub.provider != "google_play" {
        return Err(BridgeError::ValidationError(
            "Price step-up actions are only supported for Google Play subscriptions".to_string(),
        ));
    }

    sqlx::query(
        "UPDATE pay.subscriptions
         SET google_requires_price_step_up_consent = false,
             google_price_step_up_consent_status = 'declined',
             google_price_step_up_consent_deadline = NULL,
             google_pending_cancellation = true,
             google_pending_cancellation_at = NOW(),
             auto_renewing = false,
             cancellation_initiated_at = NOW(),
             updated_at = NOW()
         WHERE id = $1",
    )
    .bind(sub.id)
    .execute(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(SubscriptionActionResponse {
            success: true,
            message: "Price step-up declined, subscription scheduled for cancellation".to_string(),
        }),
    ))
}

async fn dispatch_subscription_callback(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    sub: &db::subscriptions::Subscription,
    event_type: &str,
) -> Result<(), BridgeError> {
    let app = db::apps::get_app(pool, app_id).await?;
    let provider_event_id = format!("manual-{}", Uuid::new_v4());
    let timestamp_epoch_ms = chrono::Utc::now().timestamp_millis();
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_epoch_ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339();

    let payload = serde_json::json!({
        "source": "api",
        "event_type": event_type,
        "subscription_id": sub.subscription_id,
        "external_user_id": sub.external_user_id,
        "provider": sub.provider,
    });

    let (webhook_provider_id, _) = db::webhooks::create_webhook_provider(
        pool,
        app_id,
        &sub.provider,
        &provider_event_id,
        event_type,
        Some(sub.subscription_id.clone()),
        sub.purchase_token.clone(),
        payload,
        Some(timestamp_epoch_ms),
    )
    .await?;

    let delivery_id = db::webhooks::create_webhook_delivery(pool, app_id, webhook_provider_id).await?;

    let canonical = crate::webhooks::processor::CanonicalWebhookPayload {
        event_id: format!("{}-{}", sub.provider, provider_event_id),
        event_type: event_type.to_string(),
        timestamp,
        timestamp_epoch_ms,
        app_slug: app.slug,
        product_id: None,
        subscription_id: Some(sub.subscription_id.clone()),
        external_user_id: Some(sub.external_user_id.clone()),
        amount_cents: None,
        auto_renewing: sub.auto_renewing,
        purchase_token: sub.purchase_token.clone(),
        current_period_end: sub.current_period_end.map(|d| d.to_rfc3339()),
        status: Some(sub.status.clone()),
        provider: sub.provider.clone(),
        provider_event_id,
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
    };

    crate::webhooks::forwarding::forward_webhook(pool, app_id, delivery_id, canonical).await
}
