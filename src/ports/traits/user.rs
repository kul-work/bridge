use async_trait::async_trait;
use uuid::Uuid;

use crate::error::BridgeError;

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn anonymize_user(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(i64, i64, String), BridgeError>;
}
