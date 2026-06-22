use crate::error::BridgeError;
use crate::ports::traits::{ApiKeyRepository, AppLookupRepository};
use crate::state::AppState;
use axum::{
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
    Extension, Json,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct AppAuth {
    pub app_id: Uuid,
    pub api_key_id: Uuid,
}

#[derive(Deserialize)]
pub struct VerifyExpectedAppRequest {
    pub expected_slug: String,
    pub webhook_secret_nonce: String,
    pub webhook_secret_proof: String,
}

pub async fn verify_expected_app(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(payload): Json<VerifyExpectedAppRequest>,
) -> Result<StatusCode, BridgeError> {
    let expected_slug = payload.expected_slug.trim();
    if expected_slug.is_empty() {
        return Err(BridgeError::BadRequest("expected_slug is required".to_string()));
    }
    let webhook_secret_nonce = payload.webhook_secret_nonce.trim();
    if webhook_secret_nonce.is_empty() {
        return Err(BridgeError::BadRequest("webhook_secret_nonce is required".to_string()));
    }
    let webhook_secret_proof = payload.webhook_secret_proof.trim();
    if webhook_secret_proof.is_empty() {
        return Err(BridgeError::BadRequest("webhook_secret_proof is required".to_string()));
    }

    let database = state.database();
    let app = database.as_ref().get_app(auth.app_id).await?;

    if app.slug != expected_slug {
        tracing::error!(
            app_id = %auth.app_id,
            api_key_id = %auth.api_key_id,
            expected_slug = %expected_slug,
            actual_slug = %app.slug,
            "Bridge API key app mismatch"
        );

        return Err(BridgeError::Forbidden(
            "Bridge API key is not valid for expected app".to_string(),
        ));
    }

    let signed_message = format!("{}:{}", webhook_secret_nonce, expected_slug);
    let mut mac = HmacSha256::new_from_slice(app.webhook_callback_secret.as_bytes())
        .map_err(|_| BridgeError::InternalServerError("HMAC init failed".to_string()))?;
    mac.update(signed_message.as_bytes());

    let provided_proof = webhook_secret_proof
        .strip_prefix("sha256=")
        .ok_or_else(|| BridgeError::BadRequest("webhook_secret_proof must use sha256= prefix".to_string()))?;
    let provided_proof = hex::decode(provided_proof)
        .map_err(|_| BridgeError::BadRequest("webhook_secret_proof must be valid hex".to_string()))?;

    if mac.verify_slice(&provided_proof).is_err() {
        tracing::error!(
            app_id = %auth.app_id,
            api_key_id = %auth.api_key_id,
            expected_slug = %expected_slug,
            "Bridge webhook callback secret mismatch"
        );

        return Err(BridgeError::Forbidden(
            "WEBHOOK_CALLBACK_SECRET is not valid for expected Bridge app".to_string(),
        ));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn api_key_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, BridgeError> {
    if request.method() == Method::OPTIONS {
        return Ok(next.run(request).await);
    }

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| BridgeError::UnauthorizedError("Missing authorization header".to_string()))?;

    let parts: Vec<&str> = auth_header.split_whitespace().collect();
    if parts.len() != 2 || parts[0] != "Bearer" {
        return Err(BridgeError::UnauthorizedError(
            "Invalid authorization header format".to_string(),
        ));
    }

    let api_key = parts[1];
    let database = state.database();
    let auth = database.as_ref().authenticate_api_key(api_key).await?;

    request.extensions_mut().insert(AppAuth {
        app_id: auth.app_id,
        api_key_id: auth.api_key_id,
    });
    Ok(next.run(request).await)
}
