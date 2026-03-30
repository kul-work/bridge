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

#[derive(Debug, Deserialize)]
pub struct CancelSubscriptionRequest {
    pub external_user_id: String,
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
    // Load subscription (provider-agnostic lookup)
    let sub = db::subscriptions::get_subscription_by_sub_id(
        &database.pool,
        auth.app_id,
        &subscription_id,
    )
    .await?
    .ok_or_else(|| BridgeError::ValidationError("Subscription not found".to_string()))?;

    // Verify ownership
    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    // Load provider config and call provider API
    let provider_config = db::provider_configs::get_provider_config(
        &database.pool,
        auth.app_id,
        &sub.provider,
    ).await?;

    provider_api::cancel_subscription(
        &sub.provider,
        &sub.subscription_id,
        sub.purchase_token.as_deref(),
        &provider_config.config,
    ).await?;

    // Provider call succeeded — update local DB
    sqlx::query(
        "UPDATE pay.subscriptions SET auto_renewing = false, cancellation_initiated_at = NOW(), updated_at = NOW() WHERE id = $1",
    )
    .bind(sub.id)
    .execute(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(SubscriptionActionResponse {
            success: true,
            message: "Subscription cancellation requested".to_string(),
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
    .ok_or_else(|| BridgeError::ValidationError("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    // Load provider config and call provider API
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

    // Provider call succeeded — update local DB
    sqlx::query(
        "UPDATE pay.subscriptions SET status = 'active', auto_renewing = true, cancellation_initiated_at = NULL, updated_at = NOW() WHERE id = $1",
    )
    .bind(sub.id)
    .execute(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

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
    .ok_or_else(|| BridgeError::ValidationError("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    sqlx::query(
        "UPDATE pay.subscriptions SET acknowledged_at = NOW(), updated_at = NOW() WHERE id = $1",
    )
    .bind(sub.id)
    .execute(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

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
    .ok_or_else(|| BridgeError::ValidationError("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    let customer_id = sub.provider_customer_id.as_deref()
        .ok_or_else(|| BridgeError::ValidationError("Provider customer ID not available for this subscription".to_string()))?;

    // Load provider config and call provider API for real portal URL
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
    .ok_or_else(|| BridgeError::ValidationError("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    sqlx::query(
        "UPDATE pay.subscriptions SET price_step_up_pending = false, updated_at = NOW() WHERE id = $1",
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
    .ok_or_else(|| BridgeError::ValidationError("Subscription not found".to_string()))?;

    if sub.external_user_id != request.external_user_id {
        return Err(BridgeError::ValidationError("Subscription does not belong to this user".to_string()));
    }

    sqlx::query(
        "UPDATE pay.subscriptions SET status = $1, price_step_up_pending = false, updated_at = NOW() WHERE id = $2",
    )
    .bind("pending_cancellation")
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
