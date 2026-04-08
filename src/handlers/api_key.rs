use crate::error::BridgeError;
use crate::state::AppState;
use axum::{
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppAuth {
    pub app_id: Uuid,
    pub api_key_id: Uuid,
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
    let auth = state.api_key_repo.authenticate_api_key(api_key).await?;

    request.extensions_mut().insert(AppAuth {
        app_id: auth.app_id,
        api_key_id: auth.api_key_id,
    });
    Ok(next.run(request).await)
}
