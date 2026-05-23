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
    ports::{ProviderConfigLookupRepository, WebhookForwardRepository, WebhookProviderLookupRepository, WebhookWriteRepository},
    ports::composites::WebhookIngressRepository,
    state::AppState,
    utils::{diagnostic_hash, redact_with_prefix},
};

const CREEM_SIGNATURE_HEADERS: [&str; 3] = ["creem-signature", "Webhook-Signature", "x-signature"];

fn google_voided_purchase_product_type(payload: &serde_json::Value) -> Option<i64> {
    payload
        .pointer("/voidedPurchaseNotification/productType")
        .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok())))
}

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
                info!("END processing {} webhook: {}", provider_name, event_id);
            }
            Ok(None) => info!("{} webhook suppressed: {}", provider_name, event_id),
            Err(e) => error!("{} webhook processing failed {}: {}", provider_name, event_id, e),
        }
    });
}

fn spawn_forward_existing_webhook(
    database: Arc<Database>,
    app_id: Uuid,
    webhook_id: Uuid,
    provider_name: &'static str,
    event_id: String,
) {
    tokio::spawn(async move {
        match crate::webhooks::processor::build_canonical_payload(database.as_ref(), webhook_id, app_id).await {
            Ok(Some(canonical)) => {
                if let Err(e) = crate::webhooks::forwarding::queue_and_forward_webhook(
                    database.as_ref(),
                    app_id,
                    webhook_id,
                    canonical,
                )
                .await
                {
                    error!("Failed to resume forwarding for {}: {}", event_id, e);
                } else {
                    info!("{} webhook forwarding resumed: {}", provider_name, event_id);
                }
            }
            Ok(None) => info!("{} webhook suppressed while resuming: {}", provider_name, event_id),
            Err(e) => error!("{} webhook payload rebuild failed {}: {}", provider_name, event_id, e),
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuplicateWebhookAction {
    Ignore,
    ResumeProcessing,
    ResumeForwarding,
}

fn duplicate_webhook_action(
    processed: bool,
    suppressed: bool,
    has_delivery: bool,
) -> DuplicateWebhookAction {
    if suppressed {
        return DuplicateWebhookAction::Ignore;
    }

    if !processed {
        return DuplicateWebhookAction::ResumeProcessing;
    }

    if !has_delivery {
        return DuplicateWebhookAction::ResumeForwarding;
    }

    DuplicateWebhookAction::Ignore
}

async fn handle_duplicate_webhook(
    database: Arc<Database>,
    app_id: Uuid,
    webhook_id: Uuid,
    provider_name: &'static str,
    event_id: &str,
) -> Result<(), BridgeError> {
    let webhook = database.as_ref().get_webhook_provider(webhook_id).await?;
    let has_delivery = database.as_ref().webhook_delivery_exists(webhook_id).await?;

    match duplicate_webhook_action(webhook.processed, webhook.suppressed, has_delivery) {
        DuplicateWebhookAction::ResumeProcessing => {
            info!("Duplicate {} webhook retrying stored unprocessed event: {}", provider_name, event_id);
            spawn_process_and_forward_webhook(database, app_id, webhook_id, provider_name, event_id.to_string());
        }
        DuplicateWebhookAction::ResumeForwarding => {
            info!("Duplicate {} webhook retrying stored undelivered event: {}", provider_name, event_id);
            spawn_forward_existing_webhook(database, app_id, webhook_id, provider_name, event_id.to_string());
        }
        DuplicateWebhookAction::Ignore => {
            info!("Duplicate {} webhook already recovered: {}", provider_name, event_id);
        }
    }

    Ok(())
}

/// Handle Google Play webhook
pub async fn handle_google_play(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    let database = state.database();

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

    // Only allow header override in test/mock mode (MOCK_EXTERNAL_APIS=true)
    let verify_signature = if crate::config::mock_external_apis_enabled() {
        // Priority 1: Request header override (X-Webhook-Verification-Mode: strict/off) - test mode only
        headers
            .get("X-Webhook-Verification-Mode")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_lowercase())
            .map(|mode| match mode.as_str() {
                "strict" => true,
                "off" => false,
                _ => verify_signature,
            })
            // Priority 2: DB config value
            .unwrap_or(verify_signature)
    } else {
        // Production: always use DB config, ignore headers
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
        let verify_audience = crate::config::parse_bool_env("GOOGLE_VERIFY_AUDIENCE", false)
            .map_err(|e| BridgeError::ConfigError(e.to_string()))?;
        let pub_sub_audience = std::env::var("GOOGLE_PUB_SUB_AUDIENCE").unwrap_or_default();
        let skip_rsa_verification = crate::config::parse_bool_env("GOOGLE_SKIP_RSA_VERIFICATION", false)
            .map_err(|e| BridgeError::ConfigError(e.to_string()))?;

        let client = tokio::task::spawn_blocking(move || {
            crate::services::google_play::client::GooglePlayClient::with_config(
                &service_account_path_owned,
                verify_audience,
                pub_sub_audience,
                skip_rsa_verification,
            )
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

    let (mut google_play_event, pubsub_message_id) = decode_google_play_payload(&payload, &headers)?;
    tracing::debug!(
        target: "BPT-RAW",
        "Webhook Incoming Payload [google_play]: {}",
        sanitize_google_play_payload_for_log(&google_play_event)
    );

    if google_play_event.get("testNotification").is_some() {
        info!(
            message_id = pubsub_message_id.as_deref().unwrap_or("unknown"),
            "Google Play test notification received; no-op"
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    // Inject test price override into payload for mock-mode enrichment
    if let Some(price_str) = headers.get("X-Test-Price-Cents").and_then(|h| h.to_str().ok()) {
        if let Ok(cents) = price_str.parse::<i64>() {
            google_play_event["_test_price_cents"] = serde_json::Value::Number(cents.into());
        }
    }

    let event_id = pubsub_message_id
        .as_deref()
        .or_else(|| google_play_event["eventId"].as_str())
        .ok_or_else(|| BridgeError::WebhookError("Missing provider event ID".to_string()))?;

    let event_type = extract_google_event_type(&google_play_event);

    let subscription_id = google_play_event["subscriptionNotification"]["subscriptionId"]
        .as_str()
        .map(|s| s.to_string());

    let purchase_token = google_play_event["subscriptionNotification"]["purchaseToken"]
        .as_str()
        .or_else(|| google_play_event["oneTimeProductNotification"]["purchaseToken"].as_str())
        .or_else(|| google_play_event["voidedPurchaseNotification"]["purchaseToken"].as_str())
        .map(|s| s.to_string());

    // For voided purchase notifications, lookup subscription_id from purchase_token if not present
    let subscription_id = if subscription_id.is_none()
        && google_play_event["voidedPurchaseNotification"].is_object()
        && google_voided_purchase_product_type(&google_play_event) != Some(2)
    {
        if let Some(purchase_token) = purchase_token.as_deref() {
            if let Ok(Some(sub_id)) = crate::db::subscriptions::lookup_subscription_id_by_purchase_token(database.pool(), app.id, purchase_token).await {
                Some(sub_id)
            } else if let Ok(Some(product_id)) = crate::db::payments::lookup_product_id_by_purchase_token_payment(database.pool(), app.id, "google_play", purchase_token).await {
                Some(product_id)
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

    info!(
        "Google Play webhook received: app_id={}, app_slug={}, event_id={}, event={}, sub_id={}, token_hash={}, event_time_ms={}",
        app.id,
        app.slug,
        event_id,
        event_type,
        subscription_id.as_deref().unwrap_or("missing"),
        purchase_token
            .as_deref()
            .map(diagnostic_hash)
            .unwrap_or_else(|| "missing".to_string()),
        timestamp_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "missing".to_string())
    );

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
        handle_duplicate_webhook(database, app.id, webhook_id, "Google Play", event_id).await?;
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
        .get_provider_config(app.id, "creem")
        .await?;

    let verify_signature = provider_config
        .config
        .get("verify_webhook_signature")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Only allow header override in test/mock mode (MOCK_EXTERNAL_APIS=true)
    let verify_signature = if crate::config::mock_external_apis_enabled() {
        // Priority 1: Request header override (X-Webhook-Verification-Mode: strict/off) - test mode only
        headers
            .get("X-Webhook-Verification-Mode")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_lowercase())
            .map(|mode| match mode.as_str() {
                "strict" => true,
                "off" => false,
                _ => verify_signature,
            })
            // Priority 2: DB config value
            .unwrap_or(verify_signature)
    } else {
        // Production: always use DB config, ignore headers
        verify_signature
    };

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    if verify_signature {
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
    }

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

    info!(
        "Creem webhook received: app_id={}, app_slug={}, event_id={}, event={}, sub_id={}, token_hash={}, event_time_ms={}",
        app.id,
        app.slug,
        event_id,
        event_type,
        subscription_id.as_deref().unwrap_or("missing"),
        diagnostic_hash(purchase_token.as_deref().unwrap_or(event_id)),
        timestamp_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "missing".to_string())
    );

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
        handle_duplicate_webhook(database, app.id, webhook_id, "Creem", event_id).await?;
        return Ok(StatusCode::NO_CONTENT);
    }

    spawn_process_and_forward_webhook(database, app.id, webhook_id, "Creem", event_id.to_string());

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

fn decode_google_play_payload(
    payload: &serde_json::Value,
    headers: &HeaderMap,
) -> Result<(serde_json::Value, Option<String>), BridgeError> {
    if payload.get("message").is_some() {
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

        let message_id = payload["message"]["messageId"]
            .as_str()
            .or_else(|| payload["message"]["message_id"].as_str())
            .map(|s| s.to_string());

        return Ok((google_play_event, message_id));
    }

    let message_id = headers
        .get("x-goog-pubsub-message-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    Ok((payload.clone(), message_id))
}

fn sanitize_google_play_payload_for_log(payload: &serde_json::Value) -> String {
    let mut sanitized = payload.clone();

    for pointer in [
        "/subscriptionNotification/purchaseToken",
        "/oneTimeProductNotification/purchaseToken",
        "/voidedPurchaseNotification/purchaseToken",
    ] {
        if let Some(value) = sanitized.pointer_mut(pointer) {
            if let Some(token) = value.as_str() {
                *value = serde_json::Value::String(redact_with_prefix(token));
            }
        }
    }

    serde_json::to_string(&sanitized).unwrap_or_else(|_| "{}".to_string())
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
            17 => "SUBSCRIPTION_ITEMS_CHANGED",
            18 => "SUBSCRIPTION_CANCELLATION_SCHEDULED",
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
    let mut result = a.len() ^ b.len();
    let max_len = a.len().max(b.len());

    for i in 0..max_len {
        let x = a.get(i).copied().unwrap_or(0) as usize;
        let y = b.get(i).copied().unwrap_or(0) as usize;
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
    use base64::Engine as _;
    use serde_json::json;

    use super::{
        decode_google_play_payload, duplicate_webhook_action, extract_header_value,
        google_voided_purchase_product_type,
        DuplicateWebhookAction, CREEM_SIGNATURE_HEADERS,
    };

    #[test]
    fn decodes_wrapped_google_play_pubsub_payload() {
        let headers = HeaderMap::new();
        let google_event = json!({
            "version": "1.0",
            "packageName": "com.hiha.fe",
            "eventTimeMillis": "1778936707956",
            "testNotification": { "version": "1.0" }
        });
        let payload = json!({
            "message": {
                "messageId": "wrapped-message-id",
                "data": base64::engine::general_purpose::STANDARD.encode(google_event.to_string())
            },
            "subscription": "projects/play/subscriptions/play-sub-dev"
        });

        let (decoded, message_id) = decode_google_play_payload(&payload, &headers).unwrap();

        assert_eq!(message_id.as_deref(), Some("wrapped-message-id"));
        assert_eq!(decoded["packageName"].as_str(), Some("com.hiha.fe"));
        assert!(decoded.get("testNotification").is_some());
    }

    #[test]
    fn accepts_unwrapped_google_play_pubsub_payload() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-goog-pubsub-message-id",
            HeaderValue::from_static("unwrapped-message-id"),
        );
        let payload = json!({
            "version": "1.0",
            "packageName": "com.hiha.fe",
            "eventTimeMillis": "1778936707956",
            "testNotification": { "version": "1.0" }
        });

        let (decoded, message_id) = decode_google_play_payload(&payload, &headers).unwrap();

        assert_eq!(message_id.as_deref(), Some("unwrapped-message-id"));
        assert_eq!(decoded["packageName"].as_str(), Some("com.hiha.fe"));
        assert!(decoded.get("testNotification").is_some());
    }

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

    #[test]
    fn duplicate_unprocessed_webhook_resumes_processing() {
        assert_eq!(
            duplicate_webhook_action(false, false, false),
            DuplicateWebhookAction::ResumeProcessing
        );
    }

    #[test]
    fn duplicate_processed_webhook_without_delivery_resumes_forwarding() {
        assert_eq!(
            duplicate_webhook_action(true, false, false),
            DuplicateWebhookAction::ResumeForwarding
        );
    }

    #[test]
    fn duplicate_suppressed_webhook_is_ignored() {
        assert_eq!(
            duplicate_webhook_action(false, true, false),
            DuplicateWebhookAction::Ignore
        );
    }

    #[test]
    fn duplicate_processed_webhook_with_delivery_is_ignored() {
        assert_eq!(
            duplicate_webhook_action(true, false, true),
            DuplicateWebhookAction::Ignore
        );
    }

    #[test]
    fn reads_voided_purchase_product_type_from_number_or_string() {
        let numeric_payload = json!({
            "voidedPurchaseNotification": {
                "productType": 2
            }
        });
        let string_payload = json!({
            "voidedPurchaseNotification": {
                "productType": "2"
            }
        });

        assert_eq!(google_voided_purchase_product_type(&numeric_payload), Some(2));
        assert_eq!(google_voided_purchase_product_type(&string_payload), Some(2));
    }
}
