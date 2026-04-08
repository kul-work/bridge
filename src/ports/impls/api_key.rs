use async_trait::async_trait;

use crate::{
    db::{self, api_keys::AuthenticatedApiKey},
    error::BridgeError,
    ports::traits::ApiKeyRepository,
};

#[async_trait]
impl ApiKeyRepository for db::Database {
    async fn authenticate_api_key(
        &self,
        raw_key: &str,
    ) -> Result<AuthenticatedApiKey, BridgeError> {
        let pool = self.pool();
        db::api_keys::authenticate_api_key(pool, raw_key).await
    }
}
