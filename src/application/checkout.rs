use axum::http::StatusCode;
use axum::Json;
use uuid::Uuid;

use crate::ports::BridgeRepository;
use crate::error::BridgeError;
use crate::handlers::checkout::{
    coinbase_amount_from_config, compute_request_fingerprint, extract_checkout_id,
    extract_checkout_url, extract_coinbase_checkout_id, extract_coinbase_checkout_url,
    normalize_provider_name, normalize_required_field, resolve_checkout_redirect_urls,
    CheckoutRequest, CheckoutResponse,
};

pub async fn create_checkout<R: BridgeRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    payload: CheckoutRequest,
) -> Result<(StatusCode, Json<CheckoutResponse>), BridgeError> {
    let external_user_id = normalize_required_field(&payload.external_user_id, "external_user_id")?;
    let email = normalize_required_field(&payload.email, "email")?;
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
            return Ok((StatusCode::OK, Json(cached_response)));
        }
    }

    let app = repo.get_app(app_id).await?;
    let provider_config = repo.get_provider_config(app_id, &provider).await?;

    let checkout_id = Uuid::new_v4().to_string();
    let checkout_urls = resolve_checkout_redirect_urls(app.app_url.as_deref());

    let response = match provider.as_str() {
        "creem" => {
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

            let product_selector = product_type.as_deref().unwrap_or(product_id.as_str());
            let selected_product_id = match product_selector {
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

            let creem_payload = serde_json::json!({
                "product_id": selected_product_id,
                "customer": { "email": email },
                "metadata": {
                    "user_id": external_user_id,
                    "external_user_id": external_user_id,
                    "product_id": product_id,
                },
                "success_url": checkout_urls.success_url,
                "cancel_url": checkout_urls.cancel_url
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

            let redirect_url = extract_checkout_url(&data)
                .ok_or_else(|| BridgeError::ProviderError("Missing Creem checkout_url".to_string()))?;
            let session_id = extract_checkout_id(&data).unwrap_or(checkout_id.as_str());

            CheckoutResponse {
                checkout_id: session_id.to_string(),
                provider: provider.clone(),
                redirect_url: Some(redirect_url.to_string()),
                mobile_checkout_data: None,
            }
        }
        "lemonsqueezy" => {
            let api_key = provider_config
                .config
                .get("api_key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BridgeError::ConfigError("Missing LemonSqueezy api_key".to_string()))?;

            let provider_product_id_raw = provider_config
                .config
                .get("product_id")
                .and_then(|v| v.as_str())
                .unwrap_or(product_id.as_str());
            let provider_product_id = provider_product_id_raw.parse::<i32>().map_err(|e| {
                BridgeError::ConfigError(format!("Invalid LemonSqueezy product_id '{}': {}", provider_product_id_raw, e))
            })?;

            let ls_payload = serde_json::json!({
                "data": {
                    "type": "checkouts",
                    "attributes": {
                        "product_id": provider_product_id,
                        "checkout_data": {
                            "email": email,
                            "custom": {
                                "user_id": external_user_id,
                                "external_user_id": external_user_id,
                                "product_id": product_id,
                                "product_type": product_type,
                                "provider": provider,
                            }
                        },
                        "product_options": {
                            "redirect_url": checkout_urls.success_url
                        }
                    }
                }
            });

            let client = reqwest::Client::new();
            let ls_response = client
                .post("https://api.lemonsqueezy.com/v1/checkouts")
                .bearer_auth(api_key)
                .header("Accept", "application/vnd.api+json")
                .header("Content-Type", "application/vnd.api+json")
                .json(&ls_payload)
                .send()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("LemonSqueezy checkout failed: {}", e)))?;

            if !ls_response.status().is_success() {
                let status = ls_response.status();
                let body = ls_response.text().await.unwrap_or_default();
                return Err(BridgeError::ProviderError(format!(
                    "LemonSqueezy checkout failed: {} - {}",
                    status, body
                )));
            }

            let data: serde_json::Value = ls_response
                .json()
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Invalid LemonSqueezy response: {}", e)))?;

            let redirect_url = data
                .pointer("/data/attributes/url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| BridgeError::ProviderError("Missing LemonSqueezy checkout url".to_string()))?;
            let session_id = data
                .pointer("/data/id")
                .and_then(|v| v.as_str())
                .unwrap_or(&checkout_id);

            CheckoutResponse {
                checkout_id: session_id.to_string(),
                provider: provider.clone(),
                redirect_url: Some(redirect_url.to_string()),
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
        "coinbase" => {
            if let Some(checkout_url) = provider_config
                .config
                .get("checkout_url")
                .or_else(|| provider_config.config.get("hosted_url"))
                .and_then(|value| value.as_str())
            {
                CheckoutResponse {
                    checkout_id,
                    provider: provider.clone(),
                    redirect_url: Some(checkout_url.to_string()),
                    mobile_checkout_data: None,
                }
            } else {
                let api_key = provider_config
                    .config
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BridgeError::ConfigError("Missing Coinbase api_key".to_string()))?;
                let api_url = provider_config
                    .config
                    .get("api_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://api.commerce.coinbase.com");
                let amount = coinbase_amount_from_config(&provider_config.config)?;
                let currency = provider_config
                    .config
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("USDC");

                let coinbase_payload = serde_json::json!({
                    "name": product_id,
                    "description": format!("Checkout for {}", product_id),
                    "pricing_type": "fixed_price",
                    "local_price": {
                        "amount": amount,
                        "currency": currency,
                    },
                    "redirect_url": checkout_urls.success_url,
                    "cancel_url": checkout_urls.cancel_url,
                    "metadata": {
                        "user_id": external_user_id,
                        "external_user_id": external_user_id,
                        "product_id": product_id,
                        "provider": provider,
                    }
                });

                let client = reqwest::Client::new();
                let coinbase_response = client
                    .post(format!("{}/charges", api_url.trim_end_matches('/')))
                    .header("X-CC-Api-Key", api_key)
                    .header("X-CC-Version", "2018-03-22")
                    .header("Content-Type", "application/json")
                    .json(&coinbase_payload)
                    .send()
                    .await
                    .map_err(|e| BridgeError::ProviderError(format!("Coinbase checkout failed: {}", e)))?;

                if !coinbase_response.status().is_success() {
                    let status = coinbase_response.status();
                    let body = coinbase_response.text().await.unwrap_or_default();
                    return Err(BridgeError::ProviderError(format!(
                        "Coinbase checkout failed: {} - {}",
                        status, body
                    )));
                }

                let data: serde_json::Value = coinbase_response
                    .json()
                    .await
                    .map_err(|e| BridgeError::ProviderError(format!("Invalid Coinbase response: {}", e)))?;

                let redirect_url = extract_coinbase_checkout_url(&data)
                    .ok_or_else(|| BridgeError::ProviderError("Missing Coinbase hosted_url".to_string()))?;
                let session_id = extract_coinbase_checkout_id(&data).unwrap_or(checkout_id.as_str());

                CheckoutResponse {
                    checkout_id: session_id.to_string(),
                    provider: provider.clone(),
                    redirect_url: Some(redirect_url.to_string()),
                    mobile_checkout_data: None,
                }
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

    Ok((StatusCode::CREATED, Json(response)))
}
