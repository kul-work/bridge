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
    pub purchase_token: String,
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
    if payload.purchase_token.is_empty() {
        return Err(BridgeError::ValidationError(
            "purchase_token is required".to_string(),
        ));
    }

    // Get app config
    let _app = db::apps::get_app(&database.pool, auth.app_id).await?;

    // Load provider config
    let provider_config =
        db::provider_configs::get_provider_config(&database.pool, auth.app_id, &payload.provider)
            .await?;

    // Verify purchase token with provider
    let (verified_status, period_end) = verify_purchase_with_provider(
        &payload.provider,
        &payload.subscription_id,
        &payload.purchase_token,
        &provider_config.config,
    )
    .await?;

    // Check if subscription exists
    let existing = db::subscriptions::get_subscription(
        &database.pool,
        auth.app_id,
        &payload.external_user_id,
        &payload.subscription_id,
        &payload.provider,
    )
    .await;

    let is_new = existing.is_err();

    let subscription = if is_new {
        // Create new subscription with verified status
        db::subscriptions::create_subscription(
            &database.pool,
            auth.app_id,
            &payload.external_user_id,
            &payload.subscription_id,
            &payload.provider,
            &verified_status,
        )
        .await?
    } else {
        existing?
    };

    let response = VerifyPurchaseResponse {
        status: verified_status,
        subscription_id: subscription.subscription_id,
        current_period_end: period_end.map(|d| d.to_rfc3339()),
        auto_renewing: subscription.auto_renewing,
        amount_cents: None,
        is_new,
    };

    Ok((StatusCode::OK, Json(response)))
}

async fn verify_purchase_with_provider(
    provider: &str,
    subscription_id: &str,
    purchase_token: &str,
    provider_config: &serde_json::Value,
) -> Result<(String, Option<chrono::DateTime<chrono::Utc>>), BridgeError> {
    match provider {
        "google_play" => {
            verify_google_play(subscription_id, purchase_token, provider_config).await
        }
        "creem" => {
            verify_creem(subscription_id, purchase_token, provider_config).await
        }
        "lemonsqueezy" => {
            verify_lemonsqueezy(subscription_id, purchase_token, provider_config).await
        }
        "coinbase" => {
            verify_coinbase(subscription_id, purchase_token, provider_config).await
        }
        _ => Err(BridgeError::ValidationError(format!(
            "Unknown provider: {}",
            provider
        ))),
    }
}

async fn verify_google_play(
    subscription_id: &str,
    _purchase_token: &str,
    config: &serde_json::Value,
) -> Result<(String, Option<chrono::DateTime<chrono::Utc>>), BridgeError> {
    let _package_name = config.get("package_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Google Play package_name".to_string()))?;

    tracing::warn!(
        "Google Play subscription {} verification requires full service setup with credentials",
        subscription_id
    );
    Ok(("pending".to_string(), None))
}

async fn verify_creem(
    subscription_id: &str,
    purchase_token: &str,
    config: &serde_json::Value,
) -> Result<(String, Option<chrono::DateTime<chrono::Utc>>), BridgeError> {
    let api_key = config.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Creem api_key".to_string()))?;
    
    let api_url = config.get("api_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.creem.com");

    let client = reqwest::Client::new();
    let url = format!(
        "{}/subscriptions/{}/verify",
        api_url.trim_end_matches('/'),
        subscription_id
    );

    let response = client
        .post(&url)
        .header("x-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "purchase_token": purchase_token }))
        .send()
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Creem verify failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let error_msg = response.text().await.unwrap_or_default();
        return Err(BridgeError::ProviderError(format!(
            "Creem verify failed: {} - {}",
            status,
            error_msg
        )));
    }

    let resp_json: serde_json::Value = response.json().await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to parse Creem response: {}", e)))?;

    let sub_status = resp_json["status"].as_str().unwrap_or("active").to_string();
    let period_end = resp_json["current_period_end"].as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    tracing::info!("Creem subscription {} verified with status: {}", subscription_id, sub_status);
    Ok((sub_status, period_end))
}

async fn verify_lemonsqueezy(
    subscription_id: &str,
    _purchase_token: &str,
    config: &serde_json::Value,
) -> Result<(String, Option<chrono::DateTime<chrono::Utc>>), BridgeError> {
    let api_key = config.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing LemonSqueezy api_key".to_string()))?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.lemonsqueezy.com/v1/subscriptions/{}",
        subscription_id
    );

    let response = client
        .get(&url)
        .bearer_auth(api_key)
        .header("Content-Type", "application/vnd.api+json")
        .send()
        .await
        .map_err(|e| BridgeError::ProviderError(format!("LemonSqueezy verify failed: {}", e)))?;

    let status = response.status();
    if !status.is_success() {
        let error_msg = response.text().await.unwrap_or_default();
        return Err(BridgeError::ProviderError(format!(
            "LemonSqueezy verify failed: {} - {}",
            status,
            error_msg
        )));
    }

    let resp_json: serde_json::Value = response.json().await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to parse LemonSqueezy response: {}", e)))?;

    let sub_status = resp_json["data"]["attributes"]["status"].as_str().unwrap_or("active").to_string();
    let period_end = resp_json["data"]["attributes"]["renews_at"].as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    tracing::info!("LemonSqueezy subscription {} verified with status: {}", subscription_id, sub_status);
    Ok((sub_status, period_end))
}

async fn verify_coinbase(
    subscription_id: &str,
    purchase_token: &str,
    config: &serde_json::Value,
) -> Result<(String, Option<chrono::DateTime<chrono::Utc>>), BridgeError> {
    let api_key = config.get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Coinbase api_key".to_string()))?;

    let client = reqwest::Client::new();
    let url = format!(
        "https://api.commerce.coinbase.com/charges/{}",
        purchase_token
    );

    let response = client
        .get(&url)
        .header("X-CC-Api-Key", api_key)
        .header("X-CC-Version", "2018-03-22")
        .send()
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Coinbase verify failed: {}", e)))?;

    let status_code = response.status();
    if !status_code.is_success() {
        let error_msg = response.text().await.unwrap_or_default();
        return Err(BridgeError::ProviderError(format!(
            "Coinbase verify failed: {} - {}",
            status_code,
            error_msg
        )));
    }

    let resp_json: serde_json::Value = response.json().await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to parse Coinbase response: {}", e)))?;

    let charge_status = resp_json["data"]["status"].as_str().unwrap_or("completed").to_string();
    let confirmed = resp_json["data"]["confirmed_at"].as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc));

    let verified_status = if charge_status == "completed" { "active".to_string() } else { "pending".to_string() };
    
    tracing::info!("Coinbase charge {} verified with status: {}", subscription_id, verified_status);
    Ok((verified_status, confirmed))
}
