use axum::{
    extract::{Path, State},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use chrono::Utc;

use crate::db::Database;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;

#[derive(Deserialize)]
pub struct AnonymizeRequest {
    pub reason: Option<String>,
}

pub async fn anonymize(
    State(database): State<Arc<Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(external_user_id): Path<String>,
    Json(request): Json<AnonymizeRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let (subscriptions_cancelled, payments_anonymized, new_anonymous_id) = 
        crate::db::users::anonymize_user(
            &database.pool,
            auth.app_id,
            &external_user_id,
            request.reason.as_deref(),
        )
        .await?;

    if subscriptions_cancelled == 0 && payments_anonymized == 0 {
        return Err(BridgeError::ValidationError("User not found".to_string()));
    }

    // Send anonymized external webhook asynchronously
    let pool = database.pool.clone();
    let app_id = auth.app_id;
    let old_user_id = external_user_id.clone();
    let new_user_id = new_anonymous_id.clone();
    tokio::spawn(async move {
        let _ = send_anonymize_webhook(&pool, app_id, &old_user_id, &new_user_id).await;
    });

    Ok(Json(json!({
        "anonymized": true,
        "subscriptions_cancelled": subscriptions_cancelled,
        "payments_anonymized": payments_anonymized,
        "new_anonymous_id": new_anonymous_id
    })))
}

pub async fn data_export(
    State(database): State<Arc<Database>>,
    Extension(auth): Extension<AppAuth>,
    Path(external_user_id): Path<String>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let subscriptions = crate::db::subscriptions::get_user_subscriptions(
        &database.pool,
        auth.app_id,
        &external_user_id,
        100,
        0,
    )
    .await?;
    
    let payments = crate::db::payments::get_user_payments(
        &database.pool,
        auth.app_id,
        &external_user_id,
        100,
        0,
    )
    .await.unwrap_or_default();

    let agent_credits = sqlx::query!(
        "SELECT balance_cents, lifetime_spent_cents, updated_at FROM pay.agent_credits WHERE app_id = $1 AND external_user_id = $2 LIMIT 1",
        auth.app_id, external_user_id
    )
    .fetch_optional(&database.pool)
    .await
    .unwrap_or_default()
    .map(|c| json!({"balance_cents": c.balance_cents, "lifetime_spent_cents": c.lifetime_spent_cents, "updated_at": c.updated_at}));

    let agent_transactions = sqlx::query!(
        "SELECT request_type, amount_cents, charge_id, status, created_at FROM pay.agent_transactions WHERE app_id = $1 AND external_user_id = $2 ORDER BY created_at DESC",
        auth.app_id, external_user_id
    )
    .fetch_all(&database.pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|t| json!({"request_type": t.request_type, "amount_cents": t.amount_cents, "charge_id": t.charge_id, "status": t.status, "created_at": t.created_at}))
    .collect::<Vec<_>>();

    let sub_ids: Vec<String> = subscriptions.iter().map(|s| s.subscription_id.clone()).collect();
    let tokens: Vec<String> = payments.iter().map(|p| p.provider_transaction_id.clone()).collect();

    let webhook_records = sqlx::query!(
        "SELECT provider, event_type, payload, created_at FROM pay.webhook_provider WHERE app_id = $1 AND (subscription_id = ANY($2) OR purchase_token = ANY($3)) ORDER BY created_at DESC",
        auth.app_id, &sub_ids, &tokens
    )
    .fetch_all(&database.pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|w| json!({"provider": w.provider, "event_type": w.event_type, "payload": w.payload, "created_at": w.created_at}))
    .collect::<Vec<_>>();

    Ok(Json(json!({
        "external_user_id": external_user_id,
        "export_date": Utc::now(),
        "subscriptions": subscriptions,
        "payments": payments,
        "agent_credits": agent_credits,
        "agent_transactions": agent_transactions,
        "webhook_records": webhook_records
    })))
}

async fn send_anonymize_webhook(
    pool: &sqlx::PgPool,
    app_id: uuid::Uuid,
    external_user_id: &str,
    new_anonymous_id: &str,
) -> Result<(), BridgeError> {
    let app = crate::db::apps::get_app(pool, app_id).await?;
    
    let payload = json!({
        "event_id": format!("anon-{}", uuid::Uuid::new_v4()),
        "event_type": "user.anonymized",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "timestamp_epoch_ms": chrono::Utc::now().timestamp_millis(),
        "app_slug": app.slug,
        "external_user_id": external_user_id,
        "new_anonymous_id": new_anonymous_id,
    });
    
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| BridgeError::WebhookError(format!("Failed to serialize payload: {}", e)))?;

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(app.webhook_callback_secret.as_bytes())
        .map_err(|_| BridgeError::WebhookError("Invalid webhook secret".to_string()))?;
    mac.update(payload_json.as_bytes());
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    let timestamp = chrono::Utc::now().timestamp().to_string();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| BridgeError::WebhookError(format!("Failed to build webhook client: {}", e)))?;
        
    let _ = client
        .post(&app.webhook_callback_url)
        .header("X-Pay-Signature", &signature)
        .header("X-Pay-Timestamp", &timestamp)
        .header("X-Pay-Event-Id", payload["event_id"].as_str().unwrap())
        .header("Content-Type", "application/json")
        .body(payload_json)
        .send()
        .await;

    Ok(())
}
