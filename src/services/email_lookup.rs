use std::time::Duration;

use hmac::{Hmac, Mac};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use sha2::Sha256;
use tracing::{error, warn};

use crate::{
    application::app_context::AppSnapshot,
    error::BridgeError,
    utils::diagnostic_hash,
};

#[derive(Debug, Deserialize)]
struct EmailLookupResponse {
    email: String,
}

const EMAIL_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug)]
pub struct EmailLookupContext<'a> {
    pub origin: &'a str,
    pub event_type: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub provider_webhook_id: Option<&'a str>,
    pub subscription_id: Option<&'a str>,
}

fn email_lookup_url(callback_url: &str) -> Result<Url, BridgeError> {
    let mut url = Url::parse(callback_url)
        .map_err(|e| BridgeError::ConfigError(format!("Invalid webhook callback URL: {}", e)))?;
    url.set_path("/internal/bridge/email-lookup");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn sign_body(secret: &str, body: &str) -> Result<String, BridgeError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| BridgeError::InternalServerError("HMAC init failed".to_string()))?;
    mac.update(body.as_bytes());
    Ok(format!("sha256={}", hex::encode(mac.finalize().into_bytes())))
}

pub async fn lookup_user_email_with_context(
    app: &AppSnapshot,
    external_user_id: &str,
    context: EmailLookupContext<'_>,
) -> Result<Option<String>, BridgeError> {
    let external_user_id_hash = diagnostic_hash(external_user_id);
    let url = email_lookup_url(&app.webhook_callback_url)?;
    let body = serde_json::json!({ "clerk_id": external_user_id }).to_string();
    let signature = sign_body(&app.webhook_callback_secret, &body)?;

    let request = Client::new()
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-Pay-Signature", signature)
        .body(body);

    let response = match tokio::time::timeout(EMAIL_LOOKUP_TIMEOUT, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => {
            error!(
                app_id = %app.id,
                origin = context.origin,
                event_type = context.event_type,
                provider = context.provider,
                provider_webhook_id = context.provider_webhook_id,
                external_user_id_hash = %external_user_id_hash,
                subscription_id = context.subscription_id,
                error_kind = "request_failed",
                "Email lookup request failed"
            );
            return Err(BridgeError::InternalServerError(
                "Email lookup request failed".to_string(),
            ));
        }
        Err(_) => {
            error!(
                app_id = %app.id,
                origin = context.origin,
                event_type = context.event_type,
                provider = context.provider,
                provider_webhook_id = context.provider_webhook_id,
                external_user_id_hash = %external_user_id_hash,
                subscription_id = context.subscription_id,
                error_kind = "timeout",
                "Email lookup timed out"
            );
            return Err(BridgeError::InternalServerError(
                "Email lookup timed out".to_string(),
            ));
        }
    };

    match response.status() {
        StatusCode::OK => {
            let payload: EmailLookupResponse = response
                .json()
                .await
                .map_err(|e| BridgeError::InternalServerError(format!("Email lookup response parse failed: {}", e)))?;
            Ok(Some(payload.email))
        }
        StatusCode::NOT_FOUND | StatusCode::CONFLICT => {
            warn!(
                app_id = %app.id,
                origin = context.origin,
                event_type = context.event_type,
                provider = context.provider,
                provider_webhook_id = context.provider_webhook_id,
                external_user_id_hash = %external_user_id_hash,
                subscription_id = context.subscription_id,
                response_status = response.status().as_u16(),
                "Skipping lifecycle email: app user email unavailable"
            );
            Ok(None)
        }
        StatusCode::UNAUTHORIZED => {
            warn!(
                app_id = %app.id,
                origin = context.origin,
                event_type = context.event_type,
                provider = context.provider,
                provider_webhook_id = context.provider_webhook_id,
                external_user_id_hash = %external_user_id_hash,
                subscription_id = context.subscription_id,
                response_status = response.status().as_u16(),
                "Skipping lifecycle email: email lookup signature rejected"
            );
            Ok(None)
        }
        status if status.is_server_error() => {
            error!(
                app_id = %app.id,
                origin = context.origin,
                event_type = context.event_type,
                provider = context.provider,
                provider_webhook_id = context.provider_webhook_id,
                external_user_id_hash = %external_user_id_hash,
                subscription_id = context.subscription_id,
                response_status = status.as_u16(),
                error_kind = "transient_failure",
                "Email lookup transient failure"
            );
            Err(BridgeError::InternalServerError(format!(
                "Email lookup transient failure: {}",
                status
            )))
        }
        status => {
            warn!(
                app_id = %app.id,
                origin = context.origin,
                event_type = context.event_type,
                provider = context.provider,
                provider_webhook_id = context.provider_webhook_id,
                external_user_id_hash = %external_user_id_hash,
                subscription_id = context.subscription_id,
                response_status = status.as_u16(),
                "Skipping lifecycle email: email lookup returned non-success status"
            );
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::email_lookup_url;

    #[test]
    fn email_lookup_url_reuses_callback_origin() {
        let url = email_lookup_url("https://hiha.app/webhooks/callback?x=1").unwrap();

        assert_eq!(url.as_str(), "https://hiha.app/internal/bridge/email-lookup");
    }
}
