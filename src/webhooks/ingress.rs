use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use std::sync::Arc;
use tracing::{error, info, info_span, Instrument};
use uuid::Uuid;

use crate::{
    db::Database,
    error::BridgeError,
    ports::{
        ProviderConfigLookupRepository, WebhookForwardRepository, WebhookProviderLookupRepository,
        WebhookSuppressionRepository, WebhookWriteRepository,
    },
    ports::composites::WebhookIngressRepository,
    state::AppState,
    utils::diagnostic_hash,
};
use crate::db::webhooks::WebhookDeliveryEnqueue;
use crate::webhooks::forwarding::{webhook_worker_id, WEBHOOK_DELIVERY_LEASE_SECS};
use crate::webhooks::provider_adapter::{ProviderWebhookAdapter, GOOGLE_PLAY_TEST_NOTIFICATION_EVENT_TYPE};

const CREEM_SIGNATURE_HEADERS: [&str; 3] = ["creem-signature", "Webhook-Signature", "x-signature"];

fn retryable_provider_ack_error(
    provider: &str,
    event_id: &str,
    error: BridgeError,
) -> BridgeError {
    BridgeError::InternalServerError(format!(
        "{} webhook {} processing failed after persistence: {}",
        provider, event_id, error
    ))
}

fn google_voided_purchase_product_type(payload: &serde_json::Value) -> Option<i64> {
    payload
        .pointer("/voidedPurchaseNotification/productType")
        .and_then(|value| value.as_i64().or_else(|| value.as_str().and_then(|s| s.parse::<i64>().ok())))
}

fn google_pubsub_identity_env_override() -> Result<(Option<bool>, Option<String>), BridgeError> {
    let Some(raw) = std::env::var("GOOGLE_VERIFY_PUBSUB_IDENTITY").ok() else {
        return Ok((None, None));
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok((None, None));
    }

    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok((Some(true), None)),
        "0" | "false" | "no" | "off" => Ok((Some(false), None)),
        _ if raw.contains('@') => {
            tracing::warn!(
                "GOOGLE_VERIFY_PUBSUB_IDENTITY contains an email; treating it as GOOGLE_PUB_SUB_SERVICE_ACCOUNT_EMAIL for backward compatibility"
            );
            Ok((Some(true), Some(raw.to_string())))
        }
        _ => Err(BridgeError::ConfigError(format!(
            "Failed to parse GOOGLE_VERIFY_PUBSUB_IDENTITY as bool: {}",
            raw
        ))),
    }
}

fn validate_google_play_package_name(
    app_id: Uuid,
    provider_config: &serde_json::Value,
    payload: &serde_json::Value,
) -> Result<(), BridgeError> {
    let expected_package_name = provider_config
        .get("package_name")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            BridgeError::ConfigError("Missing Google Play package_name in provider config".to_string())
        })?;
    let received_package_name = payload
        .get("packageName")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            BridgeError::WebhookError("Missing Google Play packageName".to_string())
        })?;

    if received_package_name != expected_package_name {
        tracing::warn!(
            app_id = %app_id,
            expected_package_name,
            received_package_name,
            "Rejecting Google Play webhook for unexpected packageName"
        );
        return Err(BridgeError::WebhookError(
            "Google Play packageName mismatch".to_string(),
        ));
    }

    Ok(())
}

fn spawn_process_and_forward_delivery(
    database: Arc<Database>,
    app_id: Uuid,
    webhook_id: Uuid,
    provider_name: &'static str,
    event_id: String,
    delivery: WebhookDeliveryEnqueue,
) {
    let span = info_span!(
        "webhook_process_delivery",
        app_id = %app_id,
        provider = provider_name,
        webhook_provider_id = %webhook_id,
        webhook_delivery_id = %delivery.id,
        event_id = %event_id,
    );

    tokio::spawn(async move {
        if !delivery.created {
            info!(
                app_id = %app_id,
                webhook_provider_id = %webhook_id,
                webhook_delivery_id = %delivery.id,
                provider = provider_name,
                event_id = %event_id,
                outcome = "duplicate_delivery_queued",
                "Webhook delivery already queued; skipping duplicate immediate processing"
            );
            return;
        }

        let Some(claim_token) = delivery.claim_token else {
            error!("New webhook delivery was not claimed before immediate processing");
            return;
        };

        match crate::webhooks::processor::process_webhook_atomically(
            database.as_ref(),
            app_id,
            webhook_id,
            delivery.id,
            claim_token,
        )
        .await
        {
            Ok(Some(canonical)) => {
                if let Err(e) = crate::webhooks::forwarding::forward_webhook(
                    database.as_ref(),
                    app_id,
                    delivery.id,
                    claim_token,
                    canonical,
                )
                .await
                {
                    error!(error = %e, "Failed to forward processed webhook delivery");
                }
            }
            Ok(None) => {
                if let Err(e) = database
                    .complete_webhook_delivery_attempt(
                        delivery.id,
                        claim_token,
                        None,
                        Some("No app callback for provider webhook".to_string()),
                        true,
                    )
                    .await
                {
                    error!(error = %e, "Failed to mark no-callback webhook delivery complete");
                }
            }
            Err(e) => error!(error = %e, "Failed to process webhook delivery"),
        }
    }.instrument(span));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuplicateWebhookAction {
    Ignore,
    ResumeProcessing,
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
        return if has_delivery { DuplicateWebhookAction::Ignore } else { DuplicateWebhookAction::ResumeProcessing };
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
            tracing::info!(
                app_id = %app_id,
                webhook_provider_id = %webhook_id,
                provider = provider_name,
                event_id = event_id,
                "Duplicate webhook retrying stored unprocessed event"
            );
            let delivery = database
                .as_ref()
                .create_webhook_delivery(
                    app_id,
                    webhook_id,
                    &webhook_worker_id("duplicate-ingress"),
                    WEBHOOK_DELIVERY_LEASE_SECS,
                )
                .await?;
            spawn_process_and_forward_delivery(database, app_id, webhook_id, provider_name, event_id.to_string(), delivery);
        }
        DuplicateWebhookAction::Ignore => {
            tracing::info!(
                app_id = %app_id,
                webhook_provider_id = %webhook_id,
                provider = provider_name,
                event_id = event_id,
                "Duplicate webhook already recovered"
            );
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

    let verify_signature = if crate::config::mock_external_apis_enabled() {
        let db_verify = provider_config
            .config
            .get("verify_webhook_signature")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        headers
            .get("X-Webhook-Verification-Mode")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_lowercase())
            .map(|mode| match mode.as_str() {
                "strict" => true,
                "off" => false,
                _ => db_verify,
            })
            .unwrap_or(db_verify)
    } else {
        let db_verify = provider_config
            .config
            .get("verify_webhook_signature")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if !db_verify {
            tracing::warn!(
                app_id = %app.id,
                provider = "google_play",
                "DB config attempted to disable webhook signature verification in non-mock mode; ignoring and forcing verification"
            );
        }
        true
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
        let pub_sub_audience = std::env::var("GOOGLE_PUB_SUB_AUDIENCE").unwrap_or_else(|_| {
            provider_config
                .config
                .get("pub_sub_audience")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        });
        let (env_verify_pubsub_identity, legacy_pub_sub_service_account_email) = google_pubsub_identity_env_override()?;
        let pub_sub_service_account_email = std::env::var("GOOGLE_PUB_SUB_SERVICE_ACCOUNT_EMAIL")
            .ok()
            .and_then(|email| {
                let email = email.trim().to_string();
                (!email.is_empty()).then_some(email)
            })
            .or(legacy_pub_sub_service_account_email)
            .or_else(|| {
                provider_config
                    .config
                    .get("pub_sub_service_account_email")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|email| !email.is_empty())
                    .map(ToOwned::to_owned)
            });
        let db_verify_pubsub_identity = provider_config
            .config
            .get("verify_pubsub_identity")
            .and_then(|v| v.as_bool());
        let requested_verify_pubsub_identity = env_verify_pubsub_identity.or(db_verify_pubsub_identity);
        let verify_pubsub_identity = if crate::config::mock_external_apis_enabled() {
            requested_verify_pubsub_identity.unwrap_or(false)
        } else {
            if requested_verify_pubsub_identity == Some(false) {
                tracing::warn!(
                    app_id = %app.id,
                    provider = "google_play",
                    "Config attempted to disable Pub/Sub identity verification in non-mock mode; ignoring and forcing verification"
                );
            }
            true
        };
        // Misconfig guard: identity verification is enabled (forced on in
        // non-mock) but no expected service account email is resolvable from
        // any source (env, legacy var, or DB provider config). Without the
        // email every Google Play webhook for this app is rejected as a
        // signature failure — but the per-request error reads like an attack,
        // not a missing config. Surface the real cause once per app so the
        // operator can set GOOGLE_PUB_SUB_SERVICE_ACCOUNT_EMAIL (or the DB
        // provider_config field) instead of chasing a false "bad signature".
        if verify_pubsub_identity && pub_sub_service_account_email.is_none() {
            static WARNED_MISSING_EMAIL: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
                std::sync::OnceLock::new();
            let warned = WARNED_MISSING_EMAIL.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
            if warned.lock().unwrap().insert(app.id.to_string()) {
                tracing::warn!(
                    app_id = %app.id,
                    provider = "google_play",
                    "Google Pub/Sub identity verification is enabled but no expected service account email is configured. \
                     Set GOOGLE_PUB_SUB_SERVICE_ACCOUNT_EMAIL (env) or provider_config.config.pub_sub_service_account_email (DB). \
                     Until then, all Google Play webhooks for this app are rejected as signature failures."
                );
            }
        }
        let skip_rsa_verification = crate::config::parse_bool_env("GOOGLE_SKIP_RSA_VERIFICATION", false)
            .map_err(|e| BridgeError::ConfigError(e.to_string()))?;

        let client = tokio::task::spawn_blocking(move || {
            crate::services::google_play::client::GooglePlayClient::with_config_and_pubsub_identity(
                &service_account_path_owned,
                verify_audience,
                pub_sub_audience,
                verify_pubsub_identity,
                pub_sub_service_account_email,
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
            tracing::error!(
                app_id = %app.id,
                provider = "google_play",
                "Google Play webhook signature verification failed"
            );
            return Err(BridgeError::WebhookError(
                "Invalid Google Play signature".to_string(),
            ));
        }

        tracing::info!(
            app_id = %app.id,
            provider = "google_play",
            "Google Play webhook signature verified"
        );
    }

    let payload: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        error!(error = %e, provider = "google_play", "Failed to parse webhook JSON");
        BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
    })?;

    let Some(normalized_event) = ProviderWebhookAdapter::GooglePlay.decode_and_normalize(payload, &headers)? else {
        return Ok(StatusCode::NO_CONTENT);
    };

    validate_google_play_package_name(app.id, &provider_config.config, &normalized_event.payload)?;

    let provider = normalized_event.provider;
    let event_id = normalized_event.provider_event_id;
    let event_type = normalized_event.raw_event_type;
    let subscription_id = normalized_event.subscription_id;
    let purchase_token = normalized_event.purchase_token;
    let timestamp_ms = normalized_event.occurred_at_ms;
    let google_play_event = normalized_event.payload;

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

    info!(
        app_id = %app.id,
        app_slug = %app.slug,
        provider = "google_play",
        event_id = event_id,
        event_type = %event_type,
        subscription_id = subscription_id.as_deref().unwrap_or("missing"),
        purchase_token_hash = %purchase_token
            .as_deref()
            .map(diagnostic_hash)
            .unwrap_or_else(|| "missing".to_string()),
        event_time_ms = timestamp_ms,
        "Google Play webhook received"
    );

    if subscription_id.is_none()
        && purchase_token.is_none()
        && event_type != GOOGLE_PLAY_TEST_NOTIFICATION_EVENT_TYPE
    {
        tracing::warn!(
            app_id = %app.id,
            event_id = event_id,
            event_type = %event_type,
            "Google Play webhook payload missing both subscription_id and purchase_token — likely malformed or unrecognized schema"
        );
    }

    let (webhook_id, is_new) = database
        .as_ref()
        .create_webhook_provider(
            app.id,
            &provider,
            &event_id,
            &event_type,
            subscription_id,
            purchase_token,
            google_play_event.clone(),
            timestamp_ms,
        )
        .await?;

    if !is_new {
        if let Err(e) = handle_duplicate_webhook(database, app.id, webhook_id, "Google Play", &event_id).await {
            error!(
                app_id = %app.id,
                webhook_provider_id = %webhook_id,
                provider = "Google Play",
                event_id = event_id,
                error = %e,
                "Duplicate webhook recovery failed before provider acknowledgement"
            );
            return Err(retryable_provider_ack_error("Google Play", &event_id, e));
        }
        return Ok(StatusCode::NO_CONTENT);
    }

    if event_type == GOOGLE_PLAY_TEST_NOTIFICATION_EVENT_TYPE {
        if let Err(e) = database.as_ref().suppress_webhook(webhook_id, "google_play_test_notification").await {
            error!(
                app_id = %app.id,
                webhook_provider_id = %webhook_id,
                provider = "Google Play",
                event_id = event_id,
                error = %e,
                "Google Play test notification suppression failed before provider acknowledgement"
            );
            return Err(retryable_provider_ack_error("Google Play", &event_id, e));
        }

        info!(
            app_id = %app.id,
            webhook_provider_id = %webhook_id,
            provider = "Google Play",
            event_id = event_id,
            "Google Play test notification stored as suppressed; no app delivery queued"
        );
        return Ok(StatusCode::NO_CONTENT);
    }

    match database.as_ref().create_webhook_delivery(
        app.id,
        webhook_id,
        &webhook_worker_id("google-ingress"),
        WEBHOOK_DELIVERY_LEASE_SECS,
    ).await {
        Ok(delivery) => {
            spawn_process_and_forward_delivery(database, app.id, webhook_id, "Google Play", event_id.to_string(), delivery);
        }
        Err(e) => {
            error!(
                app_id = %app.id,
                webhook_provider_id = %webhook_id,
                provider = "Google Play",
                event_id = event_id,
                error = %e,
                "Webhook delivery enqueue failed before provider acknowledgement"
            );
            return Err(retryable_provider_ack_error("Google Play", &event_id, e));
        }
    }

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

    let verify_signature = if crate::config::mock_external_apis_enabled() {
        let db_verify = provider_config
            .config
            .get("verify_webhook_signature")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        headers
            .get("X-Webhook-Verification-Mode")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_lowercase())
            .map(|mode| match mode.as_str() {
                "strict" => true,
                "off" => false,
                _ => db_verify,
            })
            .unwrap_or(db_verify)
    } else {
        let db_verify = provider_config
            .config
            .get("verify_webhook_signature")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if !db_verify {
            tracing::warn!(
                app_id = %app.id,
                provider = "creem",
                "DB config attempted to disable webhook signature verification in non-mock mode; ignoring and forcing verification"
            );
        }
        true
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
            tracing::error!(
                app_id = %app.id,
                provider = "creem",
                "Creem webhook signature verification failed"
            );
            return Err(BridgeError::WebhookError("Invalid signature".to_string()));
        }

        tracing::info!(
            app_id = %app.id,
            provider = "creem",
            "Creem webhook signature verified"
        );
    }

    let payload: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        error!(error = %e, provider = "creem", "Failed to parse webhook JSON");
        BridgeError::WebhookError(format!("Invalid JSON payload: {}", e))
    })?;

    let Some(normalized_event) = ProviderWebhookAdapter::Creem.decode_and_normalize(payload, &headers)? else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let provider = normalized_event.provider;
    let event_id = normalized_event.provider_event_id;
    let event_type = normalized_event.raw_event_type;
    let subscription_id = normalized_event.subscription_id;
    let purchase_token = normalized_event.purchase_token;
    let timestamp_ms = normalized_event.occurred_at_ms;
    let payload = normalized_event.payload;

    info!(
        app_id = %app.id,
        app_slug = %app.slug,
        provider = "creem",
        event_id = event_id,
        event_type = event_type,
        subscription_id = subscription_id.as_deref().unwrap_or("missing"),
        correlation_hash = %diagnostic_hash(purchase_token.as_deref().unwrap_or(&event_id)),
        event_time_ms = timestamp_ms,
        "Creem webhook received"
    );

    if subscription_id.is_none() && purchase_token.is_none() {
        tracing::warn!(
            app_id = %app.id,
            event_id = event_id,
            event_type = %event_type,
            "Creem webhook payload missing both subscription_id and purchase_token — likely malformed or unrecognized schema"
        );
    }

    let (webhook_id, is_new) = database
        .as_ref()
        .create_webhook_provider(
            app.id,
            &provider,
            &event_id,
            &event_type,
            subscription_id,
            purchase_token,
            payload.clone(),
            timestamp_ms,
        )
        .await?;

    if !is_new {
        if let Err(e) = handle_duplicate_webhook(database, app.id, webhook_id, "Creem", &event_id).await {
            error!(
                app_id = %app.id,
                webhook_provider_id = %webhook_id,
                provider = "Creem",
                event_id = event_id,
                error = %e,
                "Duplicate webhook recovery failed before provider acknowledgement"
            );
            return Err(retryable_provider_ack_error("Creem", &event_id, e));
        }
        return Ok(StatusCode::NO_CONTENT);
    }

    match database.as_ref().create_webhook_delivery(
        app.id,
        webhook_id,
        &webhook_worker_id("creem-ingress"),
        WEBHOOK_DELIVERY_LEASE_SECS,
    ).await {
        Ok(delivery) => {
            spawn_process_and_forward_delivery(database, app.id, webhook_id, "Creem", event_id.to_string(), delivery);
        }
        Err(e) => {
            error!(
                app_id = %app.id,
                webhook_provider_id = %webhook_id,
                provider = "Creem",
                event_id = event_id,
                error = %e,
                "Webhook delivery enqueue failed before provider acknowledgement"
            );
            return Err(retryable_provider_ack_error("Creem", &event_id, e));
        }
    }

    Ok(StatusCode::NO_CONTENT)
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
    use serde_json::json;

    use super::{
        duplicate_webhook_action, extract_header_value,
        google_pubsub_identity_env_override, google_voided_purchase_product_type,
        validate_google_play_package_name,
        DuplicateWebhookAction, CREEM_SIGNATURE_HEADERS,
    };


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
    fn duplicate_unprocessed_webhook_with_delivery_is_ignored() {
        assert_eq!(
            duplicate_webhook_action(false, false, true),
            DuplicateWebhookAction::Ignore
        );
    }

    #[test]
    fn duplicate_processed_webhook_without_delivery_is_ignored() {
        assert_eq!(
            duplicate_webhook_action(true, false, false),
            DuplicateWebhookAction::Ignore
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

    #[test]
    fn pubsub_identity_env_accepts_legacy_email_value() {
        let key = "GOOGLE_VERIFY_PUBSUB_IDENTITY";
        let old_value = std::env::var(key).ok();
        std::env::set_var(key, "pubsub-push@example.iam.gserviceaccount.com");

        let parsed = google_pubsub_identity_env_override().unwrap();

        assert_eq!(parsed.0, Some(true));
        assert_eq!(parsed.1.as_deref(), Some("pubsub-push@example.iam.gserviceaccount.com"));

        if let Some(value) = old_value {
            std::env::set_var(key, value);
        } else {
            std::env::remove_var(key);
        }
    }

    #[test]
    fn accepts_google_play_webhook_for_configured_package_name() {
        let app_id = uuid::Uuid::new_v4();
        let config = json!({ "package_name": "com.tyde.household" });
        let payload = json!({ "packageName": "com.tyde.household" });

        assert!(validate_google_play_package_name(app_id, &config, &payload).is_ok());
    }

    #[test]
    fn rejects_google_play_webhook_for_wrong_package_name() {
        let app_id = uuid::Uuid::new_v4();
        let config = json!({ "package_name": "com.tyde.hiha" });
        let payload = json!({ "packageName": "com.tyde.household" });

        let result = validate_google_play_package_name(app_id, &config, &payload);

        assert!(matches!(result, Err(crate::error::BridgeError::WebhookError(_))));
    }

    #[test]
    fn rejects_google_play_webhook_when_configured_package_name_is_missing() {
        let app_id = uuid::Uuid::new_v4();
        let config = json!({});
        let payload = json!({ "packageName": "com.tyde.household" });

        let result = validate_google_play_package_name(app_id, &config, &payload);

        assert!(matches!(result, Err(crate::error::BridgeError::ConfigError(_))));
    }
}
