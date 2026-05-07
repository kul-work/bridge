use hmac::{Hmac, Mac};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use sha2::Sha256;
use tracing::warn;

use crate::{
    application::app_context::AppSnapshot,
    error::BridgeError,
};

#[derive(Debug, Deserialize)]
struct EmailLookupResponse {
    email: String,
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

pub async fn lookup_user_email(
    app: &AppSnapshot,
    external_user_id: &str,
) -> Result<Option<String>, BridgeError> {
    let url = email_lookup_url(&app.webhook_callback_url)?;
    let body = serde_json::json!({ "clerk_id": external_user_id }).to_string();
    let signature = sign_body(&app.webhook_callback_secret, &body)?;

    let response = Client::new()
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-Pay-Signature", signature)
        .body(body)
        .send()
        .await
        .map_err(|e| BridgeError::InternalServerError(format!("Email lookup request failed: {}", e)))?;

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
                "Skipping lifecycle email: app user email unavailable (status={})",
                response.status()
            );
            Ok(None)
        }
        StatusCode::UNAUTHORIZED => {
            warn!("Skipping lifecycle email: email lookup signature rejected");
            Ok(None)
        }
        status if status.is_server_error() => {
            Err(BridgeError::InternalServerError(format!("Email lookup transient failure: {}", status)))
        }
        status => {
            warn!("Skipping lifecycle email: email lookup returned {}", status);
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
