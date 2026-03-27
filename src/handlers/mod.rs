pub mod api_key;
pub mod checkout;
pub mod verify_purchase;
pub mod subscriptions;
pub mod admin;
pub mod users;
pub mod agent;

/// Health check handler
pub async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "healthy"}))
}
