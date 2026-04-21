use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use base64::Engine;
use std::sync::Arc;
use tracing::{error, info};
use uuid::Uuid;

use crate::{
    db::Database,
    error::BridgeError,
    ports::{ProviderConfigLookupRepository, WebhookWriteRepository},
    ports::composites::WebhookIngressRepository,
    state::AppState,
};

const CREEM_SIGNATURE_HEADERS: [&str; 3] = ["creem-signature", "Webhook-Signature", "x-signature"];

fn spawn_process_and_forward_webhook(
    database: Arc<Database>,
    app_id: Uuid,
    webhook_id: Uuid,
    provider_name: &'static str,
    event_id: String,
) {
    tokio::spawn(async move {
        match crate::webhooks::processor::process_webhook(database.as_ref(), webhook_id, app_id).await {
            Ok(Some(canonical)) => {
                if let Err(e) = crate::webhooks::forwarding::queue_and_forward_webhook(
                    database.as_ref(),
                    app_id,
                    webhook_id,
                    canonical,
                )
                .await
                {
                    error!("Failed to forward webhook for {}: {}", event_id, e);
                }
                info!("{} webhook processed: {}", provider_name, event_id);
            }
            Ok(None) => info!("{} webhook suppressed: {}", provider_name, event_id),
            Err(e) => error!("{} webhook processing failed {}: {}", provider_name, event_id, e),
        }
    });
}

/// Handle Google Play webhook
pub async fn handle_google_play(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    let database = state.database();
    info!("Received Google Play webhook with token: {}", token);

    let token_uuid = match Uuid::parse_str(&token) {
        Ok(token_uuid) => token_uuid,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let app = match database.as_ref().get_app_by_webhook_token(token_uuid).await {
        Ok(app) => app,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let provider_config = database
        .as_ref()
        .get_provider_config(app.id, "google_play")
        .await?;

    let verify_signature = provider_config
        .config
        .get("verify_webhook_signature")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Allow header override for testing (X-Webhook-Verification-Mode: off)
    let verify_signature = if let Some(mode) = headers.get("X-Webhook-Verification-Mode").and_then(|h| h.to_str().ok()) {
        match mode.to_lowercase().as_str() {
            "off" => false,
            "strict" => true,
            _ => verify_signature,
        }
    } else {
        verify_signature
    };

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

    let mut google_play_event: serde_json::Value = serde_json::from_slice(&decoded_message).map_err(|e| {
        error!("Failed to parse Google Play message.data payload: {}", e);
        BridgeError::WebhookError(format!("Invalid Google Play message.data payload: {}", e))
    })?;

    // Inject test price override into payload for mock-mode enrichment
    if let Some(price_str) = headers.get("X-Test-Price-Cents").and_then(|h| h.to_str().ok()) {
        if let Ok(cents) = price_str.parse::<i64>() {
            google_play_event["_test_price_cents"] = serde_json::Value::Number(cents.into());
        }
    }

    let event_id = payload["message"]["messageId"]
        .as_str()
        .or_else(|| payload["message"]["message_id"].as_str())
        .or_else(|| google_play_event["eventId"].as_str())
        .ok_or_else(|| BridgeError::WebhookError("Missing provider event ID".to_string()))?;

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

    // For voided purchase notifications, lookup subscription_id from purchase_token if not present
    let subscription_id = if subscription_id.is_none() && google_play_event["voidedPurchaseNotification"].is_object() {
        if let Some(purchase_token) = purchase_token.as_deref() {
            if let Ok(Some(sub_id)) = crate::db::subscriptions::lookup_subscription_id_by_purchase_token(database.pool(), app.id, purchase_token).await {
                Some(sub_id)
            } else {
                subscription_id
            }
        } else {
            subscription_id
        }
    } else {
        subscription_id
    };

    let timestamp_ms = google_play_event["eventTimeMillis"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| google_play_event["eventTimeMillis"].as_i64());

    let (webhook_id, is_new) = database
        .as_ref()
        .create_webhook_provider(
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

    spawn_process_and_forward_webhook(database, app.id, webhook_id, "Google Play", event_id.to_string());

    Ok(StatusCode::NO_CONTENT)
}

/// Handle Creem webhook
pub async fn handle_creem(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    let database = state.database();
    info!("Received Creem webhook with token: {}", token);

    let token_uuid = match Uuid::parse_str(&token) {
        Ok(token_uuid) => token_uuid,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let app = match database.as_ref().get_app_by_webhook_token(token_uuid).await {
        Ok(app) => app,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let webhook_secret = get_provider_webhook_secret(database.as_ref(), app.id, "creem").await?;
    let mut mac = HmacSha256::new_from_slice(webhook_secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;

    mac.update(body.as_bytes());
    let computed_sig = hex::encode(mac.finalize().into_bytes());

    let provided_sig = extract_header_value(&headers, &CREEM_SIGNATURE_HEADERS)
        .ok_or_else(|| BridgeError::WebhookError("Missing Webhook-Signature header".to_string()))?;

    if !constant_time_compare(provided_sig.as_bytes(), computed_sig.as_bytes()) {
        error!("Creem webhook signature verification failed");
        return Err(BridgeError::WebhookError("Invalid signature".to_string()));
    }

    info!("Creem webhook signature verified");

    let payload: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        error!("Failed to parse Creem webhook JSON: {}", e);
        BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
    })?;

    let event_id = payload["id"]
        .as_str()
        .ok_or_else(|| BridgeError::WebhookError("Missing provider event ID".to_string()))?;
    let event_type = payload["eventType"].as_str().unwrap_or("unknown");

    let subscription_id = payload["object"]["subscription"]["id"]
        .as_str()
        .or_else(|| payload["object"]["subscription_id"].as_str())
        .or_else(|| payload["object"]["id"].as_str())
        .map(|s| s.to_string());

    let purchase_token = if event_type.starts_with("subscription.") {
        payload["object"]["checkout_id"]
            .as_str()
            .or_else(|| payload["object"]["order_id"].as_str())
            .map(|s| s.to_string())
    } else {
        payload["object"]["checkout_id"]
            .as_str()
            .or_else(|| payload["object"]["order_id"].as_str())
            .or_else(|| payload["object"]["id"].as_str())
            .map(|s| s.to_string())
    };

    let timestamp_ms = payload["createdAt"].as_str().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp_millis())
    });

    let (webhook_id, is_new) = database
        .as_ref()
        .create_webhook_provider(
            app.id,
            "creem",
            event_id,
            event_type,
            subscription_id,
            purchase_token,
            payload.clone(),
            timestamp_ms,
        )
        .await?;

    if !is_new {
        info!("Duplicate Creem webhook received (already processed): {}", event_id);
        return Ok(StatusCode::NO_CONTENT);
    }

    spawn_process_and_forward_webhook(database, app.id, webhook_id, "Creem", event_id.to_string());

    Ok(StatusCode::NO_CONTENT)
}

/// Handle Coinbase webhook
pub async fn handle_coinbase(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    let database = state.database();
    info!("Received Coinbase webhook with token: {}", token);

    let token_uuid = match Uuid::parse_str(&token) {
        Ok(token_uuid) => token_uuid,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    let app = match database.as_ref().get_app_by_webhook_token(token_uuid).await {
        Ok(app) => app,
        Err(_) => return Ok(StatusCode::NOT_FOUND),
    };

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let webhook_secret = get_provider_webhook_secret(database.as_ref(), app.id, "coinbase").await?;
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

    let event_id = payload["event"]["id"]
        .as_str()
        .ok_or_else(|| BridgeError::WebhookError("Missing provider event ID".to_string()))?;
    let event_type = payload["event"]["type"].as_str().unwrap_or("unknown");
    let charge_id = payload["event"]["data"]["id"].as_str().map(|s| s.to_string());

    let timestamp_ms = payload["event"]["created_at"].as_str().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.timestamp_millis())
    });

    let (webhook_id, is_new) = database
        .as_ref()
        .create_webhook_provider(
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

    spawn_process_and_forward_webhook(database, app.id, webhook_id, "Coinbase", event_id.to_string());

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

async fn get_provider_webhook_secret<R: WebhookIngressRepository + ?Sized>(
    repo: &R,
    app_id: Uuid,
    provider: &str,
) -> Result<String, BridgeError> {
    let provider_config = repo.get_provider_config(app_id, provider).await?;

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

fn extract_header_value<'a>(headers: &'a HeaderMap, names: &[&str]) -> Option<&'a str> {
    for name in names {
        if let Some(value) = headers.get(*name).and_then(|v| v.to_str().ok()) {
            return Some(value);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::{extract_header_value, CREEM_SIGNATURE_HEADERS};

    #[test]
    fn prefers_creem_signature_over_all_others() {
        let mut headers = HeaderMap::new();
        headers.insert("creem-signature", HeaderValue::from_static("creem-sig"));
        headers.insert("Webhook-Signature", HeaderValue::from_static("primary-signature"));
        headers.insert("x-signature", HeaderValue::from_static("legacy-signature"));

        assert_eq!(
            extract_header_value(&headers, &CREEM_SIGNATURE_HEADERS),
            Some("creem-sig")
        );
    }

    #[test]
    fn prefers_webhook_signature_over_legacy_signature_header() {
        let mut headers = HeaderMap::new();
        headers.insert("Webhook-Signature", HeaderValue::from_static("primary-signature"));
        headers.insert("x-signature", HeaderValue::from_static("legacy-signature"));

        assert_eq!(
            extract_header_value(&headers, &CREEM_SIGNATURE_HEADERS),
            Some("primary-signature")
        );
    }

    #[test]
    fn falls_back_to_legacy_signature_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-signature", HeaderValue::from_static("legacy-signature"));

        assert_eq!(
            extract_header_value(&headers, &CREEM_SIGNATURE_HEADERS),
            Some("legacy-signature")
        );
    }

    #[test]
    fn missing_signature_header_returns_none() {
        let headers = HeaderMap::new();

        assert_eq!(extract_header_value(&headers, &CREEM_SIGNATURE_HEADERS), None);
    }
}
