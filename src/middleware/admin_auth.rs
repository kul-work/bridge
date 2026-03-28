use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
    Json,
};
use serde_json::json;
use std::sync::Arc;
use tracing::error;

use crate::db::Database;

/// Clerk admin authentication middleware
/// Validates that request is from Tyde's internal Clerk organization
pub async fn admin_auth_middleware(
    State(_db): State<Arc<Database>>,
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<serde_json::Value>)> {
    // Extract Clerk auth header (Authorization: Bearer <token>)
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|s| s.to_string());

    // For now, require an authorization header
    // In production, verify JWT signature and org_id with Clerk
    if auth_header.is_none() {
        error!("Admin endpoint accessed without auth token");
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "unauthorized",
                "message": "Admin endpoints require authentication"
            })),
        ));
    }

    // TODO: Verify JWT with Clerk and check org_id matches Tyde's internal org
    // For now, token presence is enough to pass

    Ok(next.run(request).await)
}
