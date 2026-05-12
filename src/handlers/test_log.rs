use axum::{
    extract::ConnectInfo,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use std::net::SocketAddr;

use crate::state::AppState;

const MAX_MARKER_LEN: usize = 200;

#[derive(Debug, Deserialize)]
pub struct TestLogMarkerRequest {
    pub message: String
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/log-marker", post(write_log_marker))
}

pub async fn write_log_marker(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(payload): Json<TestLogMarkerRequest>,
) -> impl IntoResponse {
    if !addr.ip().is_loopback() {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "forbidden",
                "message": "test log markers are only accepted from loopback clients"
            })),
        );
    }

    if let Err(message) = validate_marker(&payload) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "bad_request",
                "message": message
            })),
        );
    }

    tracing::info!(
        target: "bridge::test_marker",
        "{}", payload.message.trim()
    );

    (StatusCode::NO_CONTENT, Json(serde_json::json!({})))
}

fn validate_marker(payload: &TestLogMarkerRequest) -> Result<(), &'static str> {
    let message = payload.message.trim();
    if message.is_empty() {
        return Err("message must not be empty");
    }
    if message.len() > MAX_MARKER_LEN {
        return Err("message is too long");
    }
    if !is_safe_log_text(message) {
        return Err("message contains unsupported characters");
    }

    Ok(())
}

fn is_safe_log_text(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch == ' ' || ch.is_ascii_graphic())
}
