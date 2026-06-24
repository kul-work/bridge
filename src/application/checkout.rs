use uuid::Uuid;

use crate::error::BridgeError;
use crate::application::checkout_helpers::{
    compute_request_fingerprint, normalize_provider_name,
    normalize_required_field, resolve_checkout_redirect_urls,
    validate_email_format,
};
use crate::application::checkout_types::{CheckoutRequest, CheckoutResponse};
use crate::ports::CheckoutHandlerRepository;
use crate::services::creem::config::CreemConfig;
use crate::services::creem::client::CreemClient;
use crate::utils::diagnostic_hash;

pub async fn create_checkout<R: CheckoutHandlerRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    payload: CheckoutRequest,
) -> Result<CheckoutResponse, BridgeError>
{
    let app = repo.get_app(app_id).await?;

    let external_user_id = normalize_required_field(&payload.external_user_id, "external_user_id")?;
    let email = normalize_required_field(&payload.email, "email")?;
    validate_email_format(&email)?;
    let provider = normalize_provider_name(&normalize_required_field(&payload.provider, "provider")?);
    let product_id = normalize_required_field(&payload.product_id, "product_id")?;
    let product_type = payload
        .product_type
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let mobile_product_type = product_type
        .clone()
        .unwrap_or_else(|| "subscription".to_string());

    if payload.idempotency_key.as_deref().is_some_and(|value| value.trim().is_empty()) {
        return Err(BridgeError::ValidationError(
            "idempotency_key cannot be empty".to_string(),
        ));
    }

    let request_fingerprint = compute_request_fingerprint(
        &external_user_id,
        &email,
        &provider,
        &product_id,
        product_type.as_deref(),
    )?;

    if let Some(key) = payload.idempotency_key.as_deref() {
        if let Some(cached) = repo.get_cached_checkout(app_id, key.trim()).await? {
            if cached.request_fingerprint != request_fingerprint {
                return Err(BridgeError::ValidationError(
                    "idempotency_key reused with different checkout payload".to_string(),
                ));
            }

            let cached_response: CheckoutResponse = serde_json::from_value(cached.response_payload)
                .map_err(|e| BridgeError::InternalServerError(format!("Invalid cached checkout payload: {}", e)))?;
            tracing::info!(
                route = "/api/v1/payment/checkout",
                operation = "create_checkout",
                app_id = %app_id,
                external_user_id_hash = %diagnostic_hash(&external_user_id),
                provider = %provider,
                product_id = %product_id,
                product_type = product_type.as_deref(),
                outcome = "idempotency_cache_hit",
                "Checkout response returned from idempotency cache"
            );
            return Ok(cached_response);
        }
    }

    if provider == "creem"
        && !matches!(product_type.as_deref(), Some("otp" | "inapp"))
        && repo
            .has_live_subscription_for_product(app_id, &external_user_id, &provider, &product_id)
            .await?
    {
        return Err(BridgeError::Conflict(
            "user already has an active subscription for this product".to_string(),
        ));
    }

    let provider_config = repo.get_provider_config(app_id, &provider).await?;

    let checkout_id = Uuid::new_v4().to_string();
    let checkout_urls = resolve_checkout_redirect_urls(app.app_url.as_deref());

    let response = match provider.as_str() {
        "creem" => {
            let creem_config = CreemConfig::from_json(&provider_config.config)?;
            let creem_client = CreemClient::new(creem_config.clone())?;

            let product_selector = product_type.as_deref().unwrap_or(product_id.as_str());
            let selected_product_id = match product_selector {
                "offer" => creem_config
                    .offer_id
                    .as_deref()
                    .ok_or_else(|| BridgeError::ConfigError("Missing Creem offer_id".to_string()))?,
                "otp" | "inapp" => creem_config
                    .otp_id
                    .as_deref()
                    .ok_or_else(|| BridgeError::ConfigError("Missing Creem otp_id".to_string()))?,
                "subscription" | "subs" => creem_config
                    .product_id
                    .as_deref()
                    .unwrap_or(product_id.as_str()),
                _ => product_id.as_str(),
            };

            let metadata = serde_json::json!({
                "user_id": external_user_id,
                "external_user_id": external_user_id,
                "product_id": product_id,
            });

            let (session_id, redirect_url) = creem_client
                .create_checkout(
                    selected_product_id,
                    &email,
                    metadata,
                    &checkout_urls.success_url,
                )
                .await?;

            CheckoutResponse {
                checkout_id: if session_id.is_empty() { checkout_id } else { session_id },
                provider: provider.clone(),
                redirect_url: Some(redirect_url),
                mobile_checkout_data: None,
            }
        }
        "google_play" => {
            let package_name = app
                .google_package_name
                .as_deref()
                .or_else(|| {
                    provider_config
                        .config
                        .get("package_name")
                        .and_then(|v| v.as_str())
                })
                .ok_or_else(|| BridgeError::ConfigError("Missing Google Play package_name".to_string()))?;

            let mobile_checkout_data = serde_json::json!({
                "provider": "google_play",
                "platform": "android",
                "package_name": package_name,
                "external_user_id": external_user_id,
                "email": email,
                "product_id": product_id,
                "sku": product_id,
                "product_type": mobile_product_type,
            });

            CheckoutResponse {
                checkout_id,
                provider: provider.clone(),
                redirect_url: None,
                mobile_checkout_data: Some(mobile_checkout_data),
            }
        }
        "apple" => {
            let bundle_id = app
                .apple_bundle_id
                .as_deref()
                .or_else(|| {
                    provider_config
                        .config
                        .get("bundle_id")
                        .and_then(|v| v.as_str())
                })
                .ok_or_else(|| BridgeError::ConfigError("Missing Apple bundle_id".to_string()))?;

            let mobile_checkout_data = serde_json::json!({
                "provider": "apple",
                "platform": "ios",
                "bundle_id": bundle_id,
                "external_user_id": external_user_id,
                "email": email,
                "product_id": product_id,
                "sku": product_id,
                "product_type": mobile_product_type,
            });

            CheckoutResponse {
                checkout_id,
                provider: provider.clone(),
                redirect_url: None,
                mobile_checkout_data: Some(mobile_checkout_data),
            }
        }
        _ => {
            return Err(BridgeError::ValidationError(format!(
                "Unknown provider: {}",
                provider
            )));
        }
    };

    if let Some(key) = payload.idempotency_key.as_deref() {
        let response_json = serde_json::to_value(&response)
            .map_err(|e| BridgeError::InternalServerError(format!("Failed to serialize checkout response: {}", e)))?;
        repo.cache_checkout_response(app_id, key.trim(), &request_fingerprint, &response_json)
        .await?;
    }

    tracing::info!(
        route = "/api/v1/payment/checkout",
        operation = "create_checkout",
        app_id = %app_id,
        external_user_id_hash = %diagnostic_hash(&external_user_id),
        provider = %provider,
        product_id = %product_id,
        product_type = product_type.as_deref(),
        checkout_id = %response.checkout_id,
        outcome = "created",
        "Checkout response created"
    );

    Ok(response)
}
