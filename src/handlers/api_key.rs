use crate::db;
use crate::error::BridgeError;
use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppAuth {
    pub app_id: Uuid,
}

pub async fn api_key_auth(
    State(database): State<Arc<crate::db::Database>>,
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
    let key_hash = sha256_hash(api_key);

    let app_id = db::api_keys::validate_api_key(&database.pool, &key_hash).await?;

    request.extensions_mut().insert(AppAuth { app_id });
    Ok(next.run(request).await)
}

fn sha256_hash(input: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}
