use crate::db;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use axum::{
    extract::{State, Extension},
    http::StatusCode,
    Json,
};
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CheckoutRequest {
    pub external_user_id: String,
    pub email: Option<String>,
    pub provider: String,
    pub product_id: String,
    pub product_type: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
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
    if payload.idempotency_key.as_deref() == Some("") {
        return Err(BridgeError::ValidationError(
            "idempotency_key cannot be empty".to_string(),
        ));
    }

    let request_fingerprint = compute_request_fingerprint(&payload)?;

    if let Some(key) = payload.idempotency_key.as_deref() {
        if let Some(cached) =
            db::checkout_idempotency::get_cached_checkout(&database.pool, auth.app_id, key).await?
        {
            if cached.request_fingerprint != request_fingerprint {
                return Err(BridgeError::ValidationError(
                    "idempotency_key reused with different checkout payload".to_string(),
                ));
            }

            let cached_response: CheckoutResponse = serde_json::from_value(cached.response_payload)
                .map_err(|e| BridgeError::InternalServerError(format!("Invalid cached checkout payload: {}", e)))?;
            return Ok((StatusCode::OK, Json(cached_response)));
        }
    }

    // Get app config
    let app = db::apps::get_app(&database.pool, auth.app_id).await?;

    // Load provider config
    let provider_config =
        db::provider_configs::get_provider_config(&database.pool, auth.app_id, &payload.provider)
            .await?;

    // Generate checkout ID
    let checkout_id = Uuid::new_v4().to_string();

    let response = if payload.provider == "creem" {
        let api_key = provider_config
            .config
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BridgeError::ConfigError("Missing Creem api_key".to_string()))?;
        let api_url = provider_config
            .config
            .get("api_url")
            .and_then(|v| v.as_str())
            .unwrap_or("https://api.creem.com");

        let selected_product_id = match payload.product_id.as_str() {
            "offer" => provider_config
                .config
                .get("offer_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BridgeError::ConfigError("Missing Creem offer_id".to_string()))?,
            "otp" => provider_config
                .config
                .get("otp_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BridgeError::ConfigError("Missing Creem otp_id".to_string()))?,
            _ => provider_config
                .config
                .get("product_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BridgeError::ConfigError("Missing Creem product_id".to_string()))?,
        };

        let email = payload
            .email
            .clone()
            .unwrap_or_else(|| format!("{}@local.tyde", payload.external_user_id));
        let success_url = format!(
            "{}/billing",
            app.app_url.clone().unwrap_or_else(|| "http://localhost:3000".to_string())
        );

        let creem_payload = serde_json::json!({
            "product_id": selected_product_id,
            "customer": { "email": email },
            "metadata": { "user_id": payload.external_user_id },
            "success_url": success_url
        });

        let client = reqwest::Client::new();
        let creem_response = client
            .post(format!("{}/checkouts", api_url.trim_end_matches('/')))
            .header("x-api-key", api_key)
            .header("Content-Type", "application/json")
            .json(&creem_payload)
            .send()
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Creem checkout failed: {}", e)))?;

        if !creem_response.status().is_success() {
            let status = creem_response.status();
            let body = creem_response.text().await.unwrap_or_default();
            return Err(BridgeError::ProviderError(format!(
                "Creem checkout failed: {} - {}",
                status, body
            )));
        }

        let data: serde_json::Value = creem_response
            .json()
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Invalid Creem response: {}", e)))?;

        let redirect_url = data
            .get("checkout_url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BridgeError::ProviderError("Missing Creem checkout_url".to_string()))?;
        let session_id = data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(&checkout_id);

        CheckoutResponse {
            checkout_id: session_id.to_string(),
            redirect_url: Some(redirect_url.to_string()),
            mobile_checkout_data: None,
        }
    } else {
        // Temporary fallback for non-Creem providers until all providers are wired in this endpoint.
        CheckoutResponse {
            checkout_id,
            redirect_url: Some(format!(
                "{}/checkout/{}",
                app.app_url.unwrap_or_default(),
                payload.product_id
            )),
            mobile_checkout_data: None,
        }
    };

    if let Some(key) = payload.idempotency_key.as_deref() {
        let response_json = serde_json::to_value(&response)
            .map_err(|e| BridgeError::InternalServerError(format!("Failed to serialize checkout response: {}", e)))?;
        db::checkout_idempotency::cache_checkout_response(
            &database.pool,
            auth.app_id,
            key,
            &request_fingerprint,
            &response_json,
        )
        .await?;
    }

    Ok((StatusCode::CREATED, Json(response)))
}

fn compute_request_fingerprint(payload: &CheckoutRequest) -> Result<String, BridgeError> {
    let normalized_payload = serde_json::json!({
        "external_user_id": payload.external_user_id,
        "email": payload.email,
        "provider": payload.provider,
        "product_id": payload.product_id,
        "product_type": payload.product_type,
    });

    let body = serde_json::to_vec(&normalized_payload)
        .map_err(|e| BridgeError::InternalServerError(format!("Failed to serialize checkout payload: {}", e)))?;
    let mut hasher = Sha256::new();
    hasher.update(body);
    Ok(hex::encode(hasher.finalize()))
}
