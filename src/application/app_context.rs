use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AppSnapshot {
    pub id: Uuid,
    pub slug: String,
    pub display_name: String,
    pub webhook_callback_url: String,
    pub webhook_callback_secret: String,
    pub api_rate_limit_per_minute: i32,
    pub api_rate_limit_rules: Option<Value>,
    pub app_url: Option<String>,
    pub google_package_name: Option<String>,
    pub apple_bundle_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderConfigSnapshot {
    pub config: Value,
}
