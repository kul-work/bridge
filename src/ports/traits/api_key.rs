use async_trait::async_trait;

use crate::{
    db::api_keys::AuthenticatedApiKey,
    error::BridgeError,
};

#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn authenticate_api_key(
        &self,
        raw_key: &str,
    ) -> Result<AuthenticatedApiKey, BridgeError>;
}
