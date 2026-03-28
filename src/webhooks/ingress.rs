use axum::{
    extract::{Path, State},
    http::{StatusCode, HeaderMap},
};
use std::sync::Arc;
use tracing::{info, error};
use uuid::Uuid;
use base64::Engine;

use crate::{
    db::Database,
    error::BridgeError,
};

/// Handle Google Play webhook
pub async fn handle_google_play(
    State(db): State<Arc<Database>>,
    Path(token): Path<String>,
    _headers: HeaderMap,
    body: String,
) -> Result<StatusCode, BridgeError> {
    info!("Received Google Play webhook with token: {}", token);

    // 1. Extract app from token
    let token_uuid = Uuid::parse_str(&token)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook token format".to_string()))?;
    
    let app = crate::db::apps::get_app_by_webhook_token(&db.pool, token_uuid)
        .await
        .map_err(|_| {
            error!("App not found for token: {}", token);
            BridgeError::ValidationError("App not found".to_string())
        })?;

    // 2. Verify provider signature and parse webhook
    // For Google Play, we need to extract the JWT from the body and verify it
    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| {
            error!("Failed to parse Google Play webhook JSON: {}", e);
            BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
        })?;

    // Extract the message (Google Play sends in Pub/Sub format)
    let message = payload["message"]["data"]
        .as_str()
        .ok_or_else(|| {
            error!("Missing message.data in Google Play webhook");
            BridgeError::WebhookError("Missing message.data field".to_string())
        })?;

    // Decode the base64-encoded JWT
    let jwt_bytes = base64::engine::general_purpose::STANDARD.decode(message)
        .map_err(|e| {
            error!("Failed to decode base64 message: {}", e);
            BridgeError::WebhookError(format!("Invalid base64 encoding: {}", e))
        })?;

    let jwt_str = String::from_utf8(jwt_bytes)
        .map_err(|e| {
            error!("Failed to convert bytes to UTF-8: {}", e);
            BridgeError::WebhookError(format!("Invalid UTF-8 in JWT: {}", e))
        })?;

    // Verify JWT signature using Google's public key
    // Extract the signature from the JWT
    let parts: Vec<&str> = jwt_str.split('.').collect();
    if parts.len() != 3 {
        return Err(BridgeError::WebhookError("Invalid JWT format".to_string()));
    }

    // In production, verify JWT signature with Google's public certificates
    // For now, we parse the payload and log that verification should occur
    tracing::warn!("Google Play JWT signature verification should be implemented with Google's public keys");

    let payload_json = base64::engine::general_purpose::STANDARD.decode(parts[1])
        .map_err(|e| {
            error!("Failed to decode JWT payload: {}", e);
            BridgeError::WebhookError(format!("Invalid JWT: {}", e))
        })?;

    let google_play_event: serde_json::Value = serde_json::from_slice(&payload_json)
        .map_err(|e| {
            error!("Failed to parse JWT payload: {}", e);
            BridgeError::WebhookError(format!("Invalid JWT payload: {}", e))
        })?;

    // 3. Dedup via webhook_provider table (check idempotency)
    let event_id = google_play_event["eventId"]
        .as_str()
        .unwrap_or("unknown");

    let event_type = google_play_event["subscriptionNotification"]["notificationType"]
        .as_str()
        .unwrap_or("unknown");

    let subscription_id = google_play_event["subscriptionNotification"]["subscriptionId"]
        .as_str()
        .map(|s| s.to_string());

    let timestamp_ms = google_play_event["eventTimeMillis"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok());

    let (webhook_id, is_new) = crate::db::webhooks::create_webhook_provider(
        &db.pool,
        app.id,
        "google_play",
        event_id,
        event_type,
        subscription_id,
        None,
        google_play_event.clone(),
        timestamp_ms,
    ).await?;

    if !is_new {
        info!("Duplicate Google Play webhook received (already processed): {}", event_id);
        return Ok(StatusCode::NO_CONTENT);
    }

    // 4. Call webhook processor to normalize the event
    if let Ok(Some(_canonical)) = crate::webhooks::processor::process_webhook(&db.pool, webhook_id, app.id).await {
        // 5. Create webhook_delivery record
        let _delivery_id = crate::db::webhooks::create_webhook_delivery(&db.pool, app.id, webhook_id).await?;
        info!("Google Play webhook processed successfully: {} -> {}", event_id, webhook_id);
    }

    // 6. Return 204 No Content per contract
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

    // 1. Extract app from token
    let token_uuid = Uuid::parse_str(&token)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook token format".to_string()))?;
    
    let app = crate::db::apps::get_app_by_webhook_token(&db.pool, token_uuid)
        .await
        .map_err(|_| {
            error!("App not found for token: {}", token);
            BridgeError::ValidationError("App not found".to_string())
        })?;

    // 2. Verify provider signature
    // Creem uses HMAC-SHA256 signature verification via x-signature header
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(app.webhook_callback_secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;
    
    mac.update(body.as_bytes());
    let computed_sig = hex::encode(mac.finalize().into_bytes());

    // Extract signature from x-signature header
    let provided_sig = headers.get("x-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_compare(provided_sig.as_bytes(), computed_sig.as_bytes()) {
        error!("Creem webhook signature verification failed");
        return Err(BridgeError::WebhookError("Invalid signature".to_string()));
    }

    tracing::info!("Creem webhook signature verified");
    
    // Parse payload
    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| {
            error!("Failed to parse Creem webhook JSON: {}", e);
            BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
        })?;

    // 3. Dedup via webhook_provider table
    let event_id = payload["id"]
        .as_str()
        .unwrap_or("unknown");

    let event_type = payload["eventType"]
        .as_str()
        .unwrap_or("unknown");

    let subscription_id = payload["object"]["subscription"]["id"]
        .as_str()
        .or_else(|| payload["object"]["subscription_id"].as_str())
        .map(|s| s.to_string());

    let timestamp_ms = payload["createdAt"]
        .as_str()
        .and_then(|s| {
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
    ).await?;

    if !is_new {
        info!("Duplicate Creem webhook received (already processed): {}", event_id);
        return Ok(StatusCode::NO_CONTENT);
    }

    // 4. Call webhook processor
    if let Ok(Some(_canonical)) = crate::webhooks::processor::process_webhook(&db.pool, webhook_id, app.id).await {
        // 5. Create webhook_delivery record
        let _delivery_id = crate::db::webhooks::create_webhook_delivery(&db.pool, app.id, webhook_id).await?;
        info!("Creem webhook processed successfully: {} -> {}", event_id, webhook_id);
    }

    // 6. Return 204 No Content per contract
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

    // 1. Extract app from token
    let token_uuid = Uuid::parse_str(&token)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook token format".to_string()))?;
    
    let app = crate::db::apps::get_app_by_webhook_token(&db.pool, token_uuid)
        .await
        .map_err(|_| {
            error!("App not found for token: {}", token);
            BridgeError::ValidationError("App not found".to_string())
        })?;

    // 2. Verify provider signature (LemonSqueezy uses HMAC-SHA256 with "sha256=" prefix)
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(app.webhook_callback_secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;
    
    mac.update(body.as_bytes());
    let computed_sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    // Extract signature from x-signature header
    let provided_sig = headers.get("x-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_compare(provided_sig.as_bytes(), computed_sig.as_bytes()) {
        error!("LemonSqueezy webhook signature verification failed");
        return Err(BridgeError::WebhookError("Invalid signature".to_string()));
    }

    tracing::info!("LemonSqueezy webhook signature verified");
    
    // Parse payload
    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| {
            error!("Failed to parse LemonSqueezy webhook JSON: {}", e);
            BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
        })?;

    // 3. Dedup via webhook_provider table
    let event_id = payload["meta"]["webhook_id"]
        .as_str()
        .unwrap_or("unknown");

    let event_type = payload["meta"]["event_name"]
        .as_str()
        .unwrap_or("unknown");

    let subscription_id = payload["data"]["id"]
        .as_str()
        .or_else(|| payload["data"]["attributes"]["subscription_id"].as_str())
        .map(|s| s.to_string());

    let timestamp_ms = None; // LemonSqueezy timestamps are in RFC3339 format, extract if needed

    let (webhook_id, is_new) = crate::db::webhooks::create_webhook_provider(
        &db.pool,
        app.id,
        "lemonsqueezy",
        event_id,
        event_type,
        subscription_id,
        None,
        payload.clone(),
        timestamp_ms,
    ).await?;

    if !is_new {
        info!("Duplicate LemonSqueezy webhook received (already processed): {}", event_id);
        return Ok(StatusCode::NO_CONTENT);
    }

    // 4. Call webhook processor
    if let Ok(Some(_canonical)) = crate::webhooks::processor::process_webhook(&db.pool, webhook_id, app.id).await {
        // 5. Create webhook_delivery record
        let _delivery_id = crate::db::webhooks::create_webhook_delivery(&db.pool, app.id, webhook_id).await?;
        info!("LemonSqueezy webhook processed successfully: {} -> {}", event_id, webhook_id);
    }

    // 6. Return 204 No Content per contract
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

    // 1. Extract app from token
    let token_uuid = Uuid::parse_str(&token)
        .map_err(|_| BridgeError::ValidationError("Invalid webhook token format".to_string()))?;
    
    let app = crate::db::apps::get_app_by_webhook_token(&db.pool, token_uuid)
        .await
        .map_err(|_| {
            error!("App not found for token: {}", token);
            BridgeError::ValidationError("App not found".to_string())
        })?;

    // 2. Verify provider signature (Coinbase uses HMAC-SHA256)
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(app.webhook_callback_secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;
    
    mac.update(body.as_bytes());
    let computed_sig = hex::encode(mac.finalize().into_bytes());

    // Extract signature from x-cc-webhook-signature header
    let provided_sig = headers.get("x-cc-webhook-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !constant_time_compare(provided_sig.as_bytes(), computed_sig.as_bytes()) {
        error!("Coinbase webhook signature verification failed");
        return Err(BridgeError::WebhookError("Invalid signature".to_string()));
    }

    tracing::info!("Coinbase webhook signature verified");
    
    // Parse payload
    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| {
            error!("Failed to parse Coinbase webhook JSON: {}", e);
            BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
        })?;

    // 3. Dedup via webhook_provider table
    let event_id = payload["event"]["id"]
        .as_str()
        .unwrap_or("unknown");

    let event_type = payload["event"]["type"]
        .as_str()
        .unwrap_or("unknown");

    let charge_id = payload["event"]["data"]["id"]
        .as_str()
        .map(|s| s.to_string());

    let timestamp_ms = payload["event"]["created_at"]
        .as_str()
        .and_then(|s| {
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
    ).await?;

    if !is_new {
        info!("Duplicate Coinbase webhook received (already processed): {}", event_id);
        return Ok(StatusCode::NO_CONTENT);
    }

    // 4. Call webhook processor
    if let Ok(Some(_canonical)) = crate::webhooks::processor::process_webhook(&db.pool, webhook_id, app.id).await {
        // 5. Create webhook_delivery record
        let _delivery_id = crate::db::webhooks::create_webhook_delivery(&db.pool, app.id, webhook_id).await?;
        info!("Coinbase webhook processed successfully: {} -> {}", event_id, webhook_id);
    }

    // 6. Return 204 No Content per contract
    Ok(StatusCode::NO_CONTENT)
    }

    /// Constant-time comparison of byte slices to prevent timing attacks
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

