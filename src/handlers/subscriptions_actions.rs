use crate::db;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
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
    // Get subscription to verify ownership
    let sub = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &subscription_id,
        "", // provider will be fetched from db
    )
    .await?;

    // Update subscription status to cancelled
    sqlx::query(
        "UPDATE pay.subscriptions SET status = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind("cancelled")
    .bind(sub.id)
    .execute(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(SubscriptionActionResponse {
            success: true,
            message: "Subscription cancelled".to_string(),
        }),
    ))
}

pub async fn resume_subscription(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(subscription_id): Path<String>,
    Json(request): Json<CancelSubscriptionRequest>,
) -> Result<(StatusCode, Json<SubscriptionActionResponse>), BridgeError> {
    let sub = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &subscription_id,
        "",
    )
    .await?;

    // Update subscription status to active
    sqlx::query(
        "UPDATE pay.subscriptions SET status = $1, updated_at = NOW() WHERE id = $2",
    )
    .bind("active")
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
    let sub = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &subscription_id,
        "",
    )
    .await?;

    // Mark as acknowledged (clear any pending action flags)
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
    let sub = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &subscription_id,
        "",
    )
    .await?;

    // Generate portal URL based on provider
    let portal_url = match sub.provider.as_str() {
        "creem" => format!("https://creem.app/portal/{}", sub.subscription_id),
        "lemonsqueezy" => format!("https://lemon.app/portal/{}", sub.subscription_id),
        _ => format!("https://portal.pay.tydecode.com/{}", sub.subscription_id),
    };

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
    let sub = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &subscription_id,
        "",
    )
    .await?;

    // Clear price step-up flag
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
    let sub = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &subscription_id,
        "",
    )
    .await?;

    // Set status to pending cancellation due to declined price step-up
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
