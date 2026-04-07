use crate::db;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::services::google_play::{
    client::GooglePlayClient,
    models::{Money, ProductPurchase, SubscriptionPurchaseV2},
};
use crate::webhooks::processor::CanonicalWebhookPayload;
use axum::{
    extract::{State, Extension},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, sync::Arc};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct VerifyPurchaseRequest {
    pub external_user_id: String,
    pub provider: String,
    pub subscription_id: String,
    pub purchase_token: String,
    pub product_type: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyPurchaseResponse {
    pub status: String,
    pub subscription_id: String,
    pub current_period_end: Option<String>,
    pub auto_renewing: Option<bool>,
    pub amount_cents: Option<i32>,
    pub is_new: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obfuscated_account_id: Option<String>,
}

#[derive(Clone, Copy)]
enum ProductType {
    Subscription,
    OneTimeProduct,
}

impl ProductType {
    fn parse(raw: &str) -> Result<Self, BridgeError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "subscription" | "sub" | "subs" => Ok(Self::Subscription),
            "one_time" | "one-time" | "inapp" => Ok(Self::OneTimeProduct),
            _ => Err(BridgeError::ValidationError(
                "product_type must be 'subscription' or 'one_time'".to_string(),
            )),
        }
    }

    fn is_subscription(self) -> bool {
        matches!(self, Self::Subscription)
    }

    fn callback_event_type(self) -> &'static str {
        match self {
            Self::Subscription => "subscription.activated",
            Self::OneTimeProduct => "purchase.one_time",
        }
    }

    fn payment_status(self, status: &str) -> &'static str {
        if status == "pending" {
            "pending"
        } else {
            "success"
        }
    }

    fn callback_status(self, status: &str) -> String {
        match self {
            Self::Subscription => status.to_string(),
            Self::OneTimeProduct if status == "pending" => "pending".to_string(),
            Self::OneTimeProduct => "completed".to_string(),
        }
    }
}

enum VerificationOutcome {
    Verified(VerifiedPurchase),
    LinkingRequired { obfuscated_account_id: String },
}

struct VerifiedPurchase {
    status: String,
    current_period_end: Option<DateTime<Utc>>,
    auto_renewing: Option<bool>,
    amount_cents: Option<i32>,
    payment_state: Option<i32>,
    acknowledgement: PaymentAcknowledgement,
    obfuscated_account_id: Option<String>,
}

enum PaymentAcknowledgement {
    NotApplicable,
    Pending,
    AlreadyAcknowledged,
}

struct VerifyPurchaseCallback<'a> {
    request: &'a VerifyPurchaseRequest,
    product_type: ProductType,
    status: &'a str,
    current_period_end: Option<&'a str>,
    auto_renewing: Option<bool>,
    amount_cents: Option<i32>,
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
    if payload.product_type.trim().is_empty() {
        return Err(BridgeError::ValidationError(
            "product_type is required".to_string(),
        ));
    }

    let product_type = ProductType::parse(&payload.product_type)?;

    // Get app config
    let app = db::apps::get_app(&database.pool, auth.app_id).await?;

    // Load provider config
    let provider_config =
        db::provider_configs::get_provider_config(&database.pool, auth.app_id, &payload.provider)
            .await?;

    let verification = verify_purchase_with_provider(
        &payload.provider,
        &payload.subscription_id,
        &payload.purchase_token,
        product_type,
        &payload.external_user_id,
        &provider_config.config,
    )
    .await?;

    let verified = match verification {
        VerificationOutcome::LinkingRequired {
            obfuscated_account_id,
        } => {
            return Ok((
                StatusCode::OK,
                Json(VerifyPurchaseResponse {
                    status: "linking_required".to_string(),
                    subscription_id: payload.subscription_id.clone(),
                    current_period_end: None,
                    auto_renewing: None,
                    amount_cents: None,
                    is_new: false,
                    message: Some(
                        "This purchase belongs to a different Google Play account and must be linked first"
                            .to_string(),
                    ),
                    obfuscated_account_id: Some(obfuscated_account_id),
                }),
            ));
        }
        VerificationOutcome::Verified(verified) => verified,
    };

    let existing_subscription = if product_type.is_subscription() {
        match db::subscriptions::get_subscription(
            &database.pool,
            auth.app_id,
            &payload.external_user_id,
            &payload.subscription_id,
            &payload.provider,
        )
        .await
        {
            Ok(subscription) => Some(subscription),
            Err(BridgeError::SubscriptionNotFound(_)) => None,
            Err(e) => return Err(e),
        }
    } else {
        None
    };

    let is_new = existing_subscription.is_none();

    if product_type.is_subscription() {
        if let Some(token_subscription) = db::subscriptions::get_subscription_by_purchase_token(
            &database.pool,
            auth.app_id,
            &payload.purchase_token,
        )
        .await?
        {
            if token_subscription.external_user_id != payload.external_user_id {
                if payload.provider == "google_play" {
                    if let Some(obfuscated_account_id) = verified.obfuscated_account_id.clone() {
                        return Ok((
                            StatusCode::OK,
                            Json(VerifyPurchaseResponse {
                                status: "linking_required".to_string(),
                                subscription_id: payload.subscription_id.clone(),
                                current_period_end: None,
                                auto_renewing: None,
                                amount_cents: verified.amount_cents,
                                is_new: false,
                                message: Some(
                                    "This purchase token is already owned by a different user and must be linked first"
                                        .to_string(),
                                ),
                                obfuscated_account_id: Some(obfuscated_account_id),
                            }),
                        ));
                    }
                }

                return Err(BridgeError::FraudDetected(
                    "Purchase token already bound to different user".to_string(),
                ));
            }

            if token_subscription.subscription_id != payload.subscription_id
                || token_subscription.provider != payload.provider
            {
                return Err(BridgeError::ValidationError(
                    "Purchase token already linked to a different subscription".to_string(),
                ));
            }
        }
    }

    let mut tx = database
        .pool
        .begin()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let payment_record_result = db::payments::record_payment_tx(
        &mut tx,
        auth.app_id,
        &payload.external_user_id,
        &payload.provider,
        &payload.purchase_token,
        Some(&payload.subscription_id),
        verified.amount_cents.unwrap_or(0),
        product_type.payment_status(&verified.status),
    )
    .await;

    if let Err(err) = payment_record_result {
        tx.rollback()
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;

        if payload.provider == "google_play" {
            if let BridgeError::FraudDetected(_) = err {
                if let Some(obfuscated_account_id) = verified.obfuscated_account_id.clone() {
                    return Ok((
                        StatusCode::OK,
                        Json(VerifyPurchaseResponse {
                            status: "linking_required".to_string(),
                            subscription_id: payload.subscription_id.clone(),
                            current_period_end: None,
                            auto_renewing: None,
                            amount_cents: verified.amount_cents,
                            is_new: false,
                            message: Some(
                                "This purchase belongs to a different Google Play account and must be linked first"
                                    .to_string(),
                            ),
                            obfuscated_account_id: Some(obfuscated_account_id),
                        }),
                    ));
                }
            }
        }

        return Err(err);
    }

    let current_period_end = verified.current_period_end.or_else(|| {
        existing_subscription
            .as_ref()
            .and_then(|subscription| subscription.current_period_end.as_ref().cloned())
    });
    let response_current_period_end = current_period_end.map(|d| d.to_rfc3339());

    let mut response_auto_renewing = verified.auto_renewing;

    if product_type.is_subscription() {
        let subscription = db::subscriptions::upsert_subscription_tx(
            &mut tx,
            auth.app_id,
            &payload.external_user_id,
            &payload.subscription_id,
            &payload.provider,
            &verified.status,
            current_period_end,
            Some(&payload.purchase_token),
            verified.auto_renewing.or_else(|| {
                existing_subscription
                    .as_ref()
                    .and_then(|subscription| subscription.auto_renewing)
            }),
            verified.payment_state.or_else(|| {
                existing_subscription
                    .as_ref()
                    .and_then(|subscription| subscription.payment_state)
            }),
            existing_subscription
                .as_ref()
                .and_then(|subscription| subscription.provider_customer_id.as_deref()),
            Utc::now().timestamp_millis(),
        )
        .await?
        .subscription;

        response_auto_renewing = subscription.auto_renewing;
    }

    let payment_acknowledged = db::payments::payment_acknowledged_at_tx(
        &mut tx,
        auth.app_id,
        &payload.provider,
        &payload.purchase_token,
    )
    .await?
    .is_some();

    match verified.acknowledgement {
        PaymentAcknowledgement::AlreadyAcknowledged => {
            db::payments::mark_payment_acknowledged_tx(
                &mut tx,
                auth.app_id,
                &payload.provider,
                &payload.purchase_token,
            )
            .await?;
        }
        PaymentAcknowledgement::Pending
            if payload.provider == "google_play" && !payment_acknowledged =>
        {
            if let Err(err) = acknowledge_google_play(
                &payload.subscription_id,
                &payload.purchase_token,
                product_type,
                &provider_config.config,
            )
            .await
            {
                tracing::warn!(
                    "verify_purchase acknowledgement failed for app {} token {}: {}",
                    app.id,
                    payload.purchase_token,
                    err
                );
            } else {
                db::payments::mark_payment_acknowledged_tx(
                    &mut tx,
                    auth.app_id,
                    &payload.provider,
                    &payload.purchase_token,
                )
                .await?;
            }
        }
        PaymentAcknowledgement::NotApplicable | PaymentAcknowledgement::Pending => {}
    }

    tx.commit()
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let response = VerifyPurchaseResponse {
        status: verified.status.clone(),
        subscription_id: payload.subscription_id.clone(),
        current_period_end: response_current_period_end,
        auto_renewing: response_auto_renewing,
        amount_cents: verified.amount_cents,
        is_new,
        message: None,
        obfuscated_account_id: None,
    };

    if response.status != "pending" {
        let callback_status = product_type.callback_status(&response.status);

        if let Err(e) = forward_verify_purchase_callback(
            &database.pool,
            app.id,
            &app.slug,
            VerifyPurchaseCallback {
                request: &payload,
                product_type,
                status: &callback_status,
                current_period_end: response.current_period_end.as_deref(),
                auto_renewing: response.auto_renewing,
                amount_cents: response.amount_cents,
            },
        )
        .await
        {
            tracing::warn!(
                "verify_purchase callback forwarding failed for app {} sub {}: {}",
                app.id,
                payload.subscription_id,
                e
            );
        }
    }

    Ok((StatusCode::OK, Json(response)))
}

async fn verify_purchase_with_provider(
    provider: &str,
    subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    external_user_id: &str,
    provider_config: &serde_json::Value,
) -> Result<VerificationOutcome, BridgeError> {
    match provider {
        "google_play" => verify_google_play(
            subscription_id,
            purchase_token,
            product_type,
            external_user_id,
            provider_config,
        )
        .await,
        "creem" => verify_creem(subscription_id, purchase_token, provider_config)
            .await
            .map(VerificationOutcome::Verified),
        "lemonsqueezy" => verify_lemonsqueezy(subscription_id, purchase_token, provider_config)
            .await
            .map(VerificationOutcome::Verified),
        "coinbase" => verify_coinbase(subscription_id, purchase_token, provider_config)
            .await
            .map(VerificationOutcome::Verified),
        _ => Err(BridgeError::ValidationError(format!(
            "Unknown provider: {}",
            provider
        ))),
    }
}

async fn verify_google_play(
    subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    external_user_id: &str,
    config: &serde_json::Value,
) -> Result<VerificationOutcome, BridgeError> {
    if mock_external_apis_enabled() {
        return mock_verify_google_play(subscription_id, purchase_token, product_type, external_user_id);
    }

    let client = build_google_play_client(config).await?;

    match product_type {
        ProductType::Subscription => {
            let purchase = client
                .get_subscription(google_package_name(config)?, subscription_id, purchase_token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play verify failed: {}", e)))?;

            map_google_subscription_verification(purchase, external_user_id)
        }
        ProductType::OneTimeProduct => {
            let purchase = client
                .get_product(google_package_name(config)?, subscription_id, purchase_token)
                .await
                .map_err(|e| BridgeError::ProviderError(format!("Google Play verify failed: {}", e)))?;

            let amount_cents = match purchase.order_id.as_deref() {
                Some(order_id) => match client
                    .get_order_amount_cents(google_package_name(config)?, order_id)
                    .await
                {
                    Ok(amount_cents) => amount_cents,
                    Err(err) => {
                        tracing::warn!(
                            "Google Play order amount lookup failed for order {}: {}",
                            order_id,
                            err
                        );
                        None
                    }
                },
                None => None,
            };

            Ok(VerificationOutcome::Verified(map_google_product_verification(
                purchase,
                amount_cents,
            )))
        }
    }
}

async fn verify_creem(
    subscription_id: &str,
    purchase_token: &str,
    config: &serde_json::Value,
) -> Result<VerifiedPurchase, BridgeError> {
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
    let amount_cents = first_minor_unit_amount(&resp_json, &["/price", "/total", "/amount"]);

    tracing::info!("Creem subscription {} verified with status: {}", subscription_id, sub_status);
    Ok(VerifiedPurchase {
        status: sub_status,
        current_period_end: period_end,
        auto_renewing: None,
        amount_cents,
        payment_state: None,
        acknowledgement: PaymentAcknowledgement::NotApplicable,
        obfuscated_account_id: None,
    })
}

async fn verify_lemonsqueezy(
    subscription_id: &str,
    _purchase_token: &str,
    config: &serde_json::Value,
) -> Result<VerifiedPurchase, BridgeError> {
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
    let amount_cents = first_minor_unit_amount(
        &resp_json,
        &["/data/attributes/total_cents", "/data/attributes/total"],
    );

    tracing::info!("LemonSqueezy subscription {} verified with status: {}", subscription_id, sub_status);
    Ok(VerifiedPurchase {
        status: sub_status,
        current_period_end: period_end,
        auto_renewing: None,
        amount_cents,
        payment_state: None,
        acknowledgement: PaymentAcknowledgement::NotApplicable,
        obfuscated_account_id: None,
    })
}

async fn verify_coinbase(
    subscription_id: &str,
    purchase_token: &str,
    config: &serde_json::Value,
) -> Result<VerifiedPurchase, BridgeError> {
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
    let amount_cents = resp_json["data"]["payments"]
        .as_array()
        .and_then(|payments| {
            let total = payments
                .iter()
                .filter(|payment| payment["status"].as_str() == Some("CONFIRMED"))
                .filter_map(|payment| {
                    payment["value"]["local"]["amount"]
                        .as_str()
                        .and_then(|amount| amount.parse::<f64>().ok())
                })
                .map(|amount| (amount * 100.0).round() as i64)
                .sum::<i64>();

            (total > 0)
                .then_some(total)
                .and_then(|total| i32::try_from(total).ok())
        })
        .or_else(|| parse_major_unit_amount_to_cents(&resp_json["data"]["pricing"]["local"]["amount"]));

    let verified_status = if charge_status == "completed" { "active".to_string() } else { "pending".to_string() };
    
    tracing::info!("Coinbase charge {} verified with status: {}", subscription_id, verified_status);
    Ok(VerifiedPurchase {
        status: verified_status,
        current_period_end: confirmed,
        auto_renewing: None,
        amount_cents,
        payment_state: None,
        acknowledgement: PaymentAcknowledgement::NotApplicable,
        obfuscated_account_id: None,
    })
}

async fn forward_verify_purchase_callback(
    pool: &sqlx::PgPool,
    app_id: uuid::Uuid,
    app_slug: &str,
    callback: VerifyPurchaseCallback<'_>,
) -> Result<(), BridgeError> {
    let now = chrono::Utc::now();
    let event_id = format!("verify-purchase-{}", Uuid::new_v4());

    let (webhook_id, _) = crate::db::webhooks::create_webhook_provider(
        pool,
        app_id,
        &callback.request.provider,
        &event_id,
        "verify_purchase.succeeded",
        Some(callback.request.subscription_id.clone()),
        Some(callback.request.purchase_token.clone()),
        serde_json::json!({
            "source": "verify_purchase",
            "external_user_id": callback.request.external_user_id,
            "subscription_id": callback.request.subscription_id,
            "provider": callback.request.provider,
            "status": callback.status,
        }),
        Some(now.timestamp_millis()),
    )
    .await?;

    let delivery_id = crate::db::webhooks::create_webhook_delivery(pool, app_id, webhook_id).await?;

    let callback_payload = CanonicalWebhookPayload {
        event_id: event_id.clone(),
        event_type: callback.product_type.callback_event_type().to_string(),
        timestamp: now.to_rfc3339(),
        timestamp_epoch_ms: now.timestamp_millis(),
        app_slug: app_slug.to_string(),
        product_id: Some(callback.request.subscription_id.clone()),
        subscription_id: callback.product_type
            .is_subscription()
            .then(|| callback.request.subscription_id.clone()),
        external_user_id: Some(callback.request.external_user_id.clone()),
        amount_cents: callback.amount_cents,
        new_price_cents: None,
        auto_renewing: callback.auto_renewing,
        purchase_token: Some(callback.request.purchase_token.clone()),
        current_period_end: callback.current_period_end.map(|s| s.to_string()),
        status: Some(callback.status.to_string()),
        provider: callback.request.provider.clone(),
        provider_event_id: event_id,
        previous_status: None,
        corrected_status: None,
        reconciliation_source: None,
        revocation_reason: None,
        cancellation_mode: None,
    };

    crate::webhooks::forwarding::forward_webhook(pool, app_id, delivery_id, callback_payload).await
}

fn mock_external_apis_enabled() -> bool {
    matches!(
        std::env::var("MOCK_EXTERNAL_APIS")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn google_package_name(config: &serde_json::Value) -> Result<&str, BridgeError> {
    config
        .get("package_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| BridgeError::ConfigError("Missing Google Play package_name".to_string()))
}

async fn build_google_play_client(config: &serde_json::Value) -> Result<GooglePlayClient, BridgeError> {
    let sa_path = config
        .get("service_account_json")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            BridgeError::ConfigError("Missing Google Play service_account_json path".to_string())
        })?;

    let sa_path_owned = sa_path.to_string();
    tokio::task::spawn_blocking(move || GooglePlayClient::new(&sa_path_owned))
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to spawn blocking task: {}", e)))?
        .map_err(|e| BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e)))
}

fn parse_minor_unit_amount(value: &serde_json::Value) -> Option<i32> {
    value
        .as_i64()
        .and_then(|amount| i32::try_from(amount).ok())
        .or_else(|| {
            value
                .as_str()
                .and_then(|amount| amount.parse::<i64>().ok())
                .and_then(|amount| i32::try_from(amount).ok())
        })
}

fn parse_major_unit_amount_to_cents(value: &serde_json::Value) -> Option<i32> {
    let amount = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|amount| amount.parse::<f64>().ok()))?;

    let cents = (amount * 100.0).round();
    if !cents.is_finite() || cents < 0.0 || cents > i32::MAX as f64 {
        return None;
    }

    Some(cents as i32)
}

fn first_minor_unit_amount(resp_json: &serde_json::Value, paths: &[&str]) -> Option<i32> {
    paths
        .iter()
        .find_map(|path| resp_json.pointer(path).and_then(parse_minor_unit_amount))
}

fn google_money_to_cents(money: &Money) -> Option<i32> {
    let units = money.units.as_deref()?.parse::<i64>().ok()?;
    if units < 0 {
        return None;
    }

    let nanos = i64::from(money.nanos.unwrap_or(0)).clamp(0, 999_999_999);
    let total_cents = units.checked_mul(100)?.checked_add(nanos / 10_000_000)?;
    i32::try_from(total_cents).ok()
}

fn map_google_subscription_verification(
    purchase: SubscriptionPurchaseV2,
    external_user_id: &str,
) -> Result<VerificationOutcome, BridgeError> {
    let obfuscated_account_id = purchase
        .external_account_identifiers
        .as_ref()
        .and_then(|ids| ids.obfuscated_account_id.clone());

    if let Some(expired_identifiers) = purchase
        .out_of_app_purchase_context
        .as_ref()
        .and_then(|ctx| ctx.expired_external_account_identifiers.as_ref())
    {
        if let Some(expired_id) = expired_identifiers.obfuscated_account_id.clone() {
            if compute_obfuscated_id_hash(external_user_id) != expired_id {
                return Ok(VerificationOutcome::LinkingRequired {
                    obfuscated_account_id: expired_id,
                });
            }
        }
    }

    if let Some(owner_hash) = obfuscated_account_id.clone() {
        if compute_obfuscated_id_hash(external_user_id) != owner_hash {
            return Ok(VerificationOutcome::LinkingRequired {
                obfuscated_account_id: owner_hash,
            });
        }
    }

    let status = match purchase.subscription_state.as_deref() {
        Some("SUBSCRIPTION_STATE_ACTIVE") | Some("SUBSCRIPTION_STATE_IN_GRACE_PERIOD") => "active",
        Some("SUBSCRIPTION_STATE_CANCELED") => "cancelled",
        Some("SUBSCRIPTION_STATE_ON_HOLD") => "on_hold",
        Some("SUBSCRIPTION_STATE_PAUSED") => "paused",
        Some("SUBSCRIPTION_STATE_EXPIRED") => "expired",
        Some("SUBSCRIPTION_STATE_PENDING") => "pending",
        _ => "expired",
    }
    .to_string();

    let current_period_end = purchase
        .line_items
        .first()
        .and_then(|line_item| line_item.expiry_time.as_deref())
        .or(purchase.expiry_time.as_deref())
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let auto_renewing = purchase.auto_renewing.or_else(|| {
        purchase
            .line_items
            .first()
            .and_then(|line_item| line_item.auto_renewing_plan.as_ref())
            .and_then(|plan| plan.auto_renew_enabled)
    });

    let amount_cents = purchase
        .line_items
        .iter()
        .find_map(|line_item| line_item.auto_renewing_plan.as_ref())
        .and_then(|plan| plan.recurring_price.as_ref())
        .and_then(google_money_to_cents);

    let acknowledgement = match purchase.acknowledgement_state.as_deref() {
        Some("ACKNOWLEDGEMENT_STATE_ACKNOWLEDGED") => PaymentAcknowledgement::AlreadyAcknowledged,
        _ => PaymentAcknowledgement::Pending,
    };

    Ok(VerificationOutcome::Verified(VerifiedPurchase {
        status,
        current_period_end,
        auto_renewing,
        amount_cents,
        payment_state: None,
        acknowledgement,
        obfuscated_account_id,
    }))
}

fn map_google_product_verification(
    purchase: ProductPurchase,
    amount_cents: Option<i32>,
) -> VerifiedPurchase {
    let status = match purchase.purchase_state {
        0 => "active",
        1 => "cancelled",
        2 => "pending",
        _ => "expired",
    }
    .to_string();

    let acknowledgement = match purchase.acknowledgement_state {
        Some(1) => PaymentAcknowledgement::AlreadyAcknowledged,
        _ => PaymentAcknowledgement::Pending,
    };

    VerifiedPurchase {
        status,
        current_period_end: None,
        auto_renewing: None,
        amount_cents,
        payment_state: Some(purchase.purchase_state),
        acknowledgement,
        obfuscated_account_id: purchase.obfuscated_account_id,
    }
}

async fn acknowledge_google_play(
    subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    config: &serde_json::Value,
) -> Result<(), BridgeError> {
    if mock_external_apis_enabled() {
        return Ok(());
    }

    let client = build_google_play_client(config).await?;

    match product_type {
        ProductType::Subscription => client
            .acknowledge_subscription(google_package_name(config)?, subscription_id, purchase_token)
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Google Play acknowledgement failed: {}", e))),
        ProductType::OneTimeProduct => client
            .acknowledge(google_package_name(config)?, subscription_id, purchase_token)
            .await
            .map_err(|e| BridgeError::ProviderError(format!("Google Play acknowledgement failed: {}", e))),
    }
}

fn mock_verify_google_play(
    _subscription_id: &str,
    purchase_token: &str,
    product_type: ProductType,
    external_user_id: &str,
) -> Result<VerificationOutcome, BridgeError> {
    match product_type {
        ProductType::Subscription => {
            if let Some(purchase) = load_mock_fixture::<SubscriptionPurchaseV2>(
                "MOCK_GOOGLE_PURCHASE_RESPONSE",
            ) {
                return map_google_subscription_verification(purchase, external_user_id);
            }

            if purchase_token == "resubscribe-linking-required" {
                return Ok(VerificationOutcome::LinkingRequired {
                    obfuscated_account_id: "sub-19b-owner-hash".to_string(),
                });
            }

            let status = if purchase_token.contains("pending") {
                "pending"
            } else if purchase_token.contains("on_hold") {
                "on_hold"
            } else if purchase_token.contains("paused") {
                "paused"
            } else if purchase_token.contains("cancelled") || purchase_token.contains("canceled") {
                "cancelled"
            } else {
                "active"
            }
            .to_string();

            Ok(VerificationOutcome::Verified(VerifiedPurchase {
                status,
                current_period_end: Some(Utc::now() + Duration::days(30)),
                auto_renewing: Some(
                    !purchase_token.contains("cancelled") && !purchase_token.contains("canceled"),
                ),
                amount_cents: None,
                payment_state: None,
                acknowledgement: PaymentAcknowledgement::Pending,
                obfuscated_account_id: Some(compute_obfuscated_id_hash(external_user_id)),
            }))
        }
        ProductType::OneTimeProduct => {
            if let Some(purchase) = load_mock_fixture::<ProductPurchase>("MOCK_GOOGLE_PURCHASE_RESPONSE")
            {
                return Ok(VerificationOutcome::Verified(map_google_product_verification(
                    purchase,
                    None,
                )));
            }

            if purchase_token.contains("declined") || purchase_token.contains("fail") {
                return Err(BridgeError::ProviderError(
                    "Payment was declined by the payment method".to_string(),
                ));
            }

            let status = if purchase_token.contains("slow") || purchase_token.contains("pending") {
                "pending"
            } else {
                "active"
            }
            .to_string();

            Ok(VerificationOutcome::Verified(VerifiedPurchase {
                status,
                current_period_end: None,
                auto_renewing: None,
                amount_cents: None,
                payment_state: Some(if purchase_token.contains("slow") || purchase_token.contains("pending") {
                    2
                } else {
                    0
                }),
                acknowledgement: PaymentAcknowledgement::Pending,
                obfuscated_account_id: None,
            }))
        }
    }
}

fn load_mock_fixture<T: DeserializeOwned>(env_key: &str) -> Option<T> {
    let path = std::env::var(env_key).ok()?;
    let content = fs::read_to_string(path).ok()?;

    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(payload) = value.get("payload") {
            if let Ok(parsed) = serde_json::from_value::<T>(payload.clone()) {
                return Some(parsed);
            }
        }
    }

    serde_json::from_str::<T>(&content).ok()
}

fn compute_obfuscated_id_hash(external_user_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(external_user_id.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::google_play::models::{AutoRenewingPlan, SubscriptionLineItem};

    #[test]
    fn google_subscription_verification_returns_recurring_price_cents() {
        let purchase = SubscriptionPurchaseV2 {
            auto_renewing: Some(true),
            subscription_state: Some("SUBSCRIPTION_STATE_ACTIVE".to_string()),
            acknowledgement_state: Some("ACKNOWLEDGEMENT_STATE_PENDING".to_string()),
            line_items: vec![SubscriptionLineItem {
                product_id: "premium_monthly".to_string(),
                expiry_time: Some("2026-04-30T00:00:00Z".to_string()),
                auto_renewing_plan: Some(AutoRenewingPlan {
                    auto_renew_enabled: Some(true),
                    recurring_price: Some(Money {
                        currency_code: Some("USD".to_string()),
                        units: Some("12".to_string()),
                        nanos: Some(990_000_000),
                    }),
                }),
                offer_details: None,
            }],
            ..Default::default()
        };

        let verification = map_google_subscription_verification(purchase, "user-123")
            .expect("google subscription verification should succeed");

        match verification {
            VerificationOutcome::Verified(verified) => {
                assert_eq!(verified.amount_cents, Some(1299));
                assert_eq!(verified.status, "active");
            }
            VerificationOutcome::LinkingRequired { .. } => {
                panic!("expected a verified purchase response")
            }
        }
    }

    #[test]
    fn google_product_verification_uses_enriched_order_amount() {
        let purchase = ProductPurchase {
            kind: "androidpublisher".to_string(),
            purchase_state: 0,
            purchase_time_millis: "1712448000000".to_string(),
            order_id: Some("GPA.1234-5678-9012-34567".to_string()),
            obfuscated_account_id: None,
            acknowledgement_state: Some(1),
        };

        let verified = map_google_product_verification(purchase, Some(499));

        assert_eq!(verified.amount_cents, Some(499));
        assert_eq!(verified.status, "active");
        assert_eq!(verified.payment_state, Some(0));
    }
}
