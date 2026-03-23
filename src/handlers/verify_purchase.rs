use crate::db;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use axum::{
    extract::{State, Extension},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct VerifyPurchaseRequest {
    pub external_user_id: String,
    pub provider: String,
    pub subscription_id: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyPurchaseResponse {
    pub status: String,
    pub subscription_id: String,
    pub current_period_end: Option<String>,
    pub auto_renewing: Option<bool>,
    pub amount_cents: Option<i32>,
    pub is_new: bool,
}

pub async fn verify_purchase(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Json(payload): Json<VerifyPurchaseRequest>,
) -> Result<(StatusCode, Json<VerifyPurchaseResponse>), BridgeError> {
    // Validate inputs
    if payload.external_user_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "external_user_id is required".to_string(),
        ));
    }
    if payload.provider.is_empty() {
        return Err(BridgeError::ValidationError(
            "provider is required".to_string(),
        ));
    }
    if payload.subscription_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "subscription_id is required".to_string(),
        ));
    }

    // Get app config
    let _app = db::apps::get_app(&database.pool, auth.app_id).await?;

    // Load provider config
    let _provider_config =
        db::provider_configs::get_provider_config(&database.pool, auth.app_id, &payload.provider)
            .await?;

    // Check if subscription exists
    let existing = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &payload.subscription_id,
    )
    .await;

    let is_new = existing.is_err();

    let subscription = if is_new {
        // Create new subscription
        db::subscriptions::create_subscription(
            &database.pool,
            auth.app_id,
            &payload.external_user_id,
            &payload.subscription_id,
            &payload.provider,
            "pending",
        )
        .await?
    } else {
        existing?
    };

    let response = VerifyPurchaseResponse {
        status: subscription.status,
        subscription_id: subscription.subscription_id,
        current_period_end: subscription.current_period_end.map(|d| d.to_rfc3339()),
        auto_renewing: subscription.auto_renewing,
        amount_cents: None, // TODO: Load from provider config
        is_new,
    };

    Ok((StatusCode::OK, Json(response)))
}
