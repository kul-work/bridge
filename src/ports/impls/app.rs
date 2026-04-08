use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    application::app_context::{AppSnapshot, ProviderConfigSnapshot},
    db::{self},
    error::BridgeError,
    ports::traits::{AppLookupRepository, ProviderConfigLookupRepository, AppConfigRepository},
};

#[async_trait]
impl AppLookupRepository for db::Database {
    async fn get_app(&self, app_id: Uuid) -> Result<AppSnapshot, BridgeError> {
        db::apps::get_app(self.pool(), app_id)
            .await
            .map(|app| AppSnapshot {
                id: app.id,
                slug: app.slug,
                display_name: app.display_name,
                webhook_callback_url: app.webhook_callback_url,
                webhook_callback_secret: app.webhook_callback_secret,
                api_rate_limit_per_minute: app.api_rate_limit_per_minute,
                api_rate_limit_rules: app.api_rate_limit_rules,
                app_url: app.app_url,
                google_package_name: app.google_package_name,
                apple_bundle_id: app.apple_bundle_id,
            })
    }
}

#[async_trait]
impl ProviderConfigLookupRepository for db::Database {
    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfigSnapshot, BridgeError> {
        db::provider_configs::get_provider_config(self.pool(), app_id, provider)
            .await
            .map(|provider_config| ProviderConfigSnapshot {
                config: provider_config.config,
            })
    }
}

#[async_trait]
impl AppConfigRepository for db::Database {}
