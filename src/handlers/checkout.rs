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
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    pub external_user_id: String,
    pub provider: String,
    pub product_id: String,
}

#[derive(Debug, Serialize)]
pub struct CheckoutResponse {
    pub checkout_id: String,
    pub redirect_url: Option<String>,
    pub mobile_checkout_data: Option<serde_json::Value>,
}

pub async fn create_checkout(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Json(payload): Json<CheckoutRequest>,
) -> Result<(StatusCode, Json<CheckoutResponse>), BridgeError> {
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
    if payload.product_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "product_id is required".to_string(),
        ));
    }

    // Get app config
    let app = db::apps::get_app(&database.pool, auth.app_id).await?;

    // Load provider config
    let _provider_config =
        db::provider_configs::get_provider_config(&database.pool, auth.app_id, &payload.provider)
            .await?;

    // Generate checkout ID
    let checkout_id = Uuid::new_v4().to_string();

    // TODO: Delegate to provider-specific checkout logic
    let response = CheckoutResponse {
        checkout_id,
        redirect_url: Some(format!(
            "{}/checkout/{}",
            app.app_url.unwrap_or_default(),
            payload.product_id
        )),
        mobile_checkout_data: None,
    };

    Ok((StatusCode::CREATED, Json(response)))
}
