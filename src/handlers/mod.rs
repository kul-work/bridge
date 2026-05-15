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
