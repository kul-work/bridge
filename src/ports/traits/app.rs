use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    application::app_context::{AppSnapshot, ProviderConfigSnapshot},
    error::BridgeError,
};

#[async_trait]
pub trait AppLookupRepository: Send + Sync {
    async fn get_app(&self, app_id: Uuid) -> Result<AppSnapshot, BridgeError>;
}

#[async_trait]
pub trait ProviderConfigLookupRepository: Send + Sync {
    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfigSnapshot, BridgeError>;
}

#[async_trait]
pub trait AppConfigRepository: AppLookupRepository + ProviderConfigLookupRepository + Send + Sync {}
