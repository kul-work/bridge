pub mod api_key;
pub mod checkout;
pub mod verify_purchase;
pub mod subscriptions;
pub mod subscriptions_actions;
pub mod payments;
pub mod admin;
pub mod users;
pub mod test_log;

/// Health check handler
pub async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "version": env!("CARGO_PKG_VERSION")
    }))
}

/// Readiness check handler
pub async fn readiness_check(
    axum::extract::State(state): axum::extract::State<crate::state::AppState>,
) -> Result<(axum::http::StatusCode, axum::Json<serde_json::Value>), crate::error::BridgeError> {
    let db = state.database();
    let count = crate::db::readiness::count_enabled_provider_configs(db.pool()).await
        .map_err(|err| {
            tracing::error!(error = %err, "Database readiness check failed");
            crate::error::BridgeError::DbError("Database readiness check failed".to_string())
        })?;

    if count == 0 {
        Ok((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "status": "not_ready",
                "database": "ok",
                "enabled_provider_configs": 0,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "version": env!("CARGO_PKG_VERSION"),
            }))
        ))
    } else {
        Ok((
            axum::http::StatusCode::OK,
            axum::Json(serde_json::json!({
                "status": "ready",
                "database": "ok",
                "enabled_provider_configs": count,
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "version": env!("CARGO_PKG_VERSION"),
            }))
        ))
    }
}
