use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use base64::Engine;
use std::sync::Arc;
use tracing::{error, info};
use uuid::Uuid;

use crate::{db::Database, error::BridgeError};

/// Handle Google Play webhook
pub async fn handle_google_play(
    State(db): State<Arc<Database>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    info!("Received Google Play webhook with token: {}", token);

    let token_uuid = match Uuid::parse_str(&token) {
        Ok(token_uuid) => token_uuid,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let app = match crate::db::apps::get_app_by_webhook_token(&db.pool, token_uuid).await {
        Ok(app) => app,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let provider_config =
        crate::db::provider_configs::get_provider_config(&db.pool, app.id, "google_play").await?;

    let verify_signature = provider_config
        .config
        .get("verify_webhook_signature")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if verify_signature {
        let authorization_header = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                BridgeError::WebhookError("Missing Authorization header".to_string())
            })?;

        let service_account_path = provider_config
            .config
            .get("service_account_json")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                BridgeError::ConfigError("Missing Google Play service_account_json path".to_string())
            })?;

        let service_account_path_owned = service_account_path.to_string();
        let client = tokio::task::spawn_blocking(move || {
            crate::services::google_play::client::GooglePlayClient::new(&service_account_path_owned)
        })
        .await
        .map_err(|e| BridgeError::ProviderError(format!("Failed to spawn blocking task: {}", e)))?
        .map_err(|e| {
            BridgeError::ConfigError(format!("Failed to init Google Play client: {}", e))
        })?;

        let verified = client
            .verify_pubsub_signature(authorization_header)
            .await
            .map_err(|e| BridgeError::WebhookError(format!("Google signature verify failed: {}", e)))?;

        if !verified {
            return Err(BridgeError::WebhookError(
                "Invalid Google Play signature".to_string(),
            ));
        }

        info!("Google Play webhook signature verified");
    }

    let payload: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        error!("Failed to parse Google Play webhook JSON: {}", e);
        BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
    })?;

    let message_data = payload["message"]["data"].as_str().ok_or_else(|| {
        error!("Missing message.data in Google Play webhook");
        BridgeError::WebhookError("Missing message.data field".to_string())
    })?;

    let decoded_message = decode_base64_flexible(message_data)
        .map_err(|e| BridgeError::WebhookError(format!("Invalid message.data: {}", e)))?;

    let google_play_event: serde_json::Value = serde_json::from_slice(&decoded_message).map_err(|e| {
        error!("Failed to parse Google Play message.data payload: {}", e);
        BridgeError::WebhookError(format!("Invalid Google Play message.data payload: {}", e))
    })?;

    let event_id = payload["message"]["messageId"]
        .as_str()
        .or_else(|| google_play_event["eventId"].as_str())
        .unwrap_or("unknown");

    let event_type = extract_google_event_type(&google_play_event);

    let subscription_id = google_play_event["subscriptionNotification"]["subscriptionId"]
        .as_str()
        .or_else(|| google_play_event["oneTimeProductNotification"]["productId"].as_str())
        .map(|s| s.to_string());

    let purchase_token = google_play_event["subscriptionNotification"]["purchaseToken"]
        .as_str()
        .or_else(|| google_play_event["oneTimeProductNotification"]["purchaseToken"].as_str())
        .or_else(|| google_play_event["voidedPurchaseNotification"]["purchaseToken"].as_str())
        .map(|s| s.to_string());

    let timestamp_ms = google_play_event["eventTimeMillis"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| google_play_event["eventTimeMillis"].as_i64());

    let (webhook_id, is_new) = crate::db::webhooks::create_webhook_provider(
        &db.pool,
        app.id,
        "google_play",
        event_id,
        &event_type,
        subscription_id,
        purchase_token,
        google_play_event.clone(),
        timestamp_ms,
    )
    .await?;

    if !is_new {
        info!(
            "Duplicate Google Play webhook received (already processed): {}",
            event_id
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    let pool = db.pool.clone();
    let app_id = app.id;
    let event_id_owned = event_id.to_string();
    tokio::spawn(async move {
        match crate::webhooks::processor::process_webhook(&pool, webhook_id, app_id).await {
            Ok(Some(canonical)) => {
                match crate::db::webhooks::create_webhook_delivery(&pool, app_id, webhook_id).await {
                    Ok(delivery_id) => {
                        let _ = crate::webhooks::forwarding::forward_webhook(
                            &pool, app_id, delivery_id, canonical,
                        )
                        .await;
                    }
                    Err(e) => error!("Failed to create delivery for {}: {}", event_id_owned, e),
                }
                info!("Google Play webhook processed: {}", event_id_owned);
            }
            Ok(None) => info!("Google Play webhook suppressed: {}", event_id_owned),
            Err(e) => error!("Google Play webhook processing failed {}: {}", event_id_owned, e),
        }
    });

    Ok(StatusCode::NO_CONTENT)
}

/// Handle Creem webhook
pub async fn handle_creem(
    State(db): State<Arc<Database>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    info!("Received Creem webhook with token: {}", token);

    let token_uuid = match Uuid::parse_str(&token) {
        Ok(token_uuid) => token_uuid,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let app = match crate::db::apps::get_app_by_webhook_token(&db.pool, token_uuid).await {
        Ok(app) => app,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let webhook_secret = get_provider_webhook_secret(&db.pool, app.id, "creem").await?;
    let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;

    mac.update(body.as_bytes());
    let computed_sig = hex::encode(mac.finalize().into_bytes());

    let provided_sig = headers
        .get("x-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_compare(provided_sig.as_bytes(), computed_sig.as_bytes()) {
        error!("Creem webhook signature verification failed");
        return Err(BridgeError::WebhookError("Invalid signature".to_string()));
    }

    info!("Creem webhook signature verified");

    let payload: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        error!("Failed to parse Creem webhook JSON: {}", e);
        BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
    })?;

    let event_id = payload["id"].as_str().unwrap_or("unknown");
    let event_type = payload["eventType"].as_str().unwrap_or("unknown");

    let subscription_id = payload["object"]["subscription"]["id"]
        .as_str()
        .or_else(|| payload["object"]["subscription_id"].as_str())
        .map(|s| s.to_string());

    let timestamp_ms = payload["createdAt"].as_str().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp_millis())
    });

    let (webhook_id, is_new) = crate::db::webhooks::create_webhook_provider(
        &db.pool,
        app.id,
        "creem",
        event_id,
        event_type,
        subscription_id,
        None,
        payload.clone(),
        timestamp_ms,
    )
    .await?;

    if !is_new {
        info!("Duplicate Creem webhook received (already processed): {}", event_id);
        return Ok(StatusCode::NO_CONTENT);
    }

    let pool = db.pool.clone();
    let app_id = app.id;
    let event_id_owned = event_id.to_string();
    tokio::spawn(async move {
        match crate::webhooks::processor::process_webhook(&pool, webhook_id, app_id).await {
            Ok(Some(canonical)) => {
                match crate::db::webhooks::create_webhook_delivery(&pool, app_id, webhook_id).await {
                    Ok(delivery_id) => {
                        let _ = crate::webhooks::forwarding::forward_webhook(
                            &pool, app_id, delivery_id, canonical,
                        )
                        .await;
                    }
                    Err(e) => error!("Failed to create delivery for {}: {}", event_id_owned, e),
                }
                info!("Creem webhook processed: {}", event_id_owned);
            }
            Ok(None) => info!("Creem webhook suppressed: {}", event_id_owned),
            Err(e) => error!("Creem webhook processing failed {}: {}", event_id_owned, e),
        }
    });

    Ok(StatusCode::NO_CONTENT)
}

/// Handle LemonSqueezy webhook
pub async fn handle_lemonsqueezy(
    State(db): State<Arc<Database>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    info!("Received LemonSqueezy webhook with token: {}", token);

    let token_uuid = match Uuid::parse_str(&token) {
        Ok(token_uuid) => token_uuid,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let app = match crate::db::apps::get_app_by_webhook_token(&db.pool, token_uuid).await {
        Ok(app) => app,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let webhook_secret = get_provider_webhook_secret(&db.pool, app.id, "lemonsqueezy").await?;
    let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;

    mac.update(body.as_bytes());
    let computed_sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    let provided_sig = headers
        .get("x-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_compare(provided_sig.as_bytes(), computed_sig.as_bytes()) {
        error!("LemonSqueezy webhook signature verification failed");
        return Err(BridgeError::WebhookError("Invalid signature".to_string()));
    }

    info!("LemonSqueezy webhook signature verified");

    let payload: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        error!("Failed to parse LemonSqueezy webhook JSON: {}", e);
        BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
    })?;

    let event_id = payload["meta"]["webhook_id"].as_str().unwrap_or("unknown");
    let event_type = payload["meta"]["event_name"].as_str().unwrap_or("unknown");

    let subscription_id = payload["data"]["id"]
        .as_str()
        .or_else(|| payload["data"]["attributes"]["subscription_id"].as_str())
        .map(|s| s.to_string());

    let (webhook_id, is_new) = crate::db::webhooks::create_webhook_provider(
        &db.pool,
        app.id,
        "lemonsqueezy",
        event_id,
        event_type,
        subscription_id,
        None,
        payload.clone(),
        None,
    )
    .await?;

    if !is_new {
        info!(
            "Duplicate LemonSqueezy webhook received (already processed): {}",
            event_id
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    let pool = db.pool.clone();
    let app_id = app.id;
    let event_id_owned = event_id.to_string();
    tokio::spawn(async move {
        match crate::webhooks::processor::process_webhook(&pool, webhook_id, app_id).await {
            Ok(Some(canonical)) => {
                match crate::db::webhooks::create_webhook_delivery(&pool, app_id, webhook_id).await {
                    Ok(delivery_id) => {
                        let _ = crate::webhooks::forwarding::forward_webhook(
                            &pool, app_id, delivery_id, canonical,
                        )
                        .await;
                    }
                    Err(e) => error!("Failed to create delivery for {}: {}", event_id_owned, e),
                }
                info!("LemonSqueezy webhook processed: {}", event_id_owned);
            }
            Ok(None) => info!("LemonSqueezy webhook suppressed: {}", event_id_owned),
            Err(e) => error!("LemonSqueezy webhook processing failed {}: {}", event_id_owned, e),
        }
    });

    Ok(StatusCode::NO_CONTENT)
}

/// Handle Coinbase webhook
pub async fn handle_coinbase(
    State(db): State<Arc<Database>>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    info!("Received Coinbase webhook with token: {}", token);

    let token_uuid = match Uuid::parse_str(&token) {
        Ok(token_uuid) => token_uuid,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let app = match crate::db::apps::get_app_by_webhook_token(&db.pool, token_uuid).await {
        Ok(app) => app,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let webhook_secret = get_provider_webhook_secret(&db.pool, app.id, "coinbase").await?;
    let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;

    mac.update(body.as_bytes());
    let computed_sig = hex::encode(mac.finalize().into_bytes());

    let provided_sig = headers
        .get("x-cc-webhook-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_compare(provided_sig.as_bytes(), computed_sig.as_bytes()) {
        error!("Coinbase webhook signature verification failed");
        return Err(BridgeError::WebhookError("Invalid signature".to_string()));
    }

    info!("Coinbase webhook signature verified");

    let payload: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        error!("Failed to parse Coinbase webhook JSON: {}", e);
        BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
    })?;

    let event_id = payload["event"]["id"].as_str().unwrap_or("unknown");
    let event_type = payload["event"]["type"].as_str().unwrap_or("unknown");
    let charge_id = payload["event"]["data"]["id"].as_str().map(|s| s.to_string());

    let timestamp_ms = payload["event"]["created_at"].as_str().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp_millis())
    });

    let (webhook_id, is_new) = crate::db::webhooks::create_webhook_provider(
        &db.pool,
        app.id,
        "coinbase",
        event_id,
        event_type,
        charge_id,
        None,
        payload.clone(),
        timestamp_ms,
    )
    .await?;

    if !is_new {
        info!(
            "Duplicate Coinbase webhook received (already processed): {}",
            event_id
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    let pool = db.pool.clone();
    let app_id = app.id;
    let event_id_owned = event_id.to_string();
    tokio::spawn(async move {
        match crate::webhooks::processor::process_webhook(&pool, webhook_id, app_id).await {
            Ok(Some(canonical)) => {
                match crate::db::webhooks::create_webhook_delivery(&pool, app_id, webhook_id).await {
                    Ok(delivery_id) => {
                        let _ = crate::webhooks::forwarding::forward_webhook(
                            &pool, app_id, delivery_id, canonical,
                        )
                        .await;
                    }
                    Err(e) => error!("Failed to create delivery for {}: {}", event_id_owned, e),
                }
                info!("Coinbase webhook processed: {}", event_id_owned);
            }
            Ok(None) => info!("Coinbase webhook suppressed: {}", event_id_owned),
            Err(e) => error!("Coinbase webhook processing failed {}: {}", event_id_owned, e),
        }
    });

    Ok(StatusCode::NO_CONTENT)
}

fn decode_base64_flexible(input: &str) -> Result<Vec<u8>, String> {
    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(input) {
        return Ok(decoded);
    }
    if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE.decode(input) {
        return Ok(decoded);
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(input)
        .map_err(|e| e.to_string())
}

fn extract_google_event_type(payload: &serde_json::Value) -> String {
    if let Some(notification_type) = payload["subscriptionNotification"]["notificationType"]
        .as_i64()
        .or_else(|| {
            payload["subscriptionNotification"]["notificationType"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
        })
    {
        return match notification_type {
            1 => "SUBSCRIPTION_RESTORED",
            2 => "SUBSCRIPTION_RENEWED",
            3 => "SUBSCRIPTION_CANCELED",
            4 => "SUBSCRIPTION_PURCHASED",
            5 => "SUBSCRIPTION_ON_HOLD",
            6 => "SUBSCRIPTION_IN_GRACE_PERIOD",
            7 => "SUBSCRIPTION_RESTARTED",
            8 => "SUBSCRIPTION_PRICE_CHANGE_CONFIRMED",
            9 => "SUBSCRIPTION_DEFERRED",
            10 => "SUBSCRIPTION_PAUSED",
            11 => "SUBSCRIPTION_PAUSE_SCHEDULE_CHANGED",
            12 => "SUBSCRIPTION_REVOKED",
            13 => "SUBSCRIPTION_EXPIRED",
            19 => "SUBSCRIPTION_PRICE_CHANGE_UPDATED",
            20 => "SUBSCRIPTION_PENDING_PURCHASE_CANCELED",
            21 => "SUBSCRIPTION_RENEWAL_PENDING",
            22 => "SUBSCRIPTION_PRICE_STEP_UP_CONSENT_UPDATED",
            _ => "SUBSCRIPTION_UNKNOWN",
        }
        .to_string();
    }

    if let Some(notification_type) = payload["oneTimeProductNotification"]["notificationType"]
        .as_i64()
        .or_else(|| {
            payload["oneTimeProductNotification"]["notificationType"]
                .as_str()
                .and_then(|s| s.parse::<i64>().ok())
        })
    {
        return match notification_type {
            1 => "ONE_TIME_PRODUCT_PURCHASED",
            2 => "ONE_TIME_PRODUCT_REFUNDED",
            14 => "ONE_TIME_PRODUCT_CANCELED",
            _ => "ONE_TIME_PRODUCT_UNKNOWN",
        }
        .to_string();
    }

    if payload.get("voidedPurchaseNotification").is_some() {
        return "VOIDED_PURCHASE".to_string();
    }

    "unknown".to_string()
}

async fn get_provider_webhook_secret(
    pool: &sqlx::PgPool,
    app_id: Uuid,
    provider: &str,
) -> Result<String, BridgeError> {
    let provider_config =
        crate::db::provider_configs::get_provider_config(pool, app_id, provider).await?;

    provider_config
        .config
        .get("webhook_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            BridgeError::ConfigError(format!(
                "Missing {} webhook_secret in provider config",
                provider
            ))
        })
}

fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut result = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}
