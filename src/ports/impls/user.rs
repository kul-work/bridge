use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{self},
    error::BridgeError,
    ports::traits::UserRepository,
};

#[async_trait]
impl UserRepository for db::Database {
    async fn anonymize_user(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(i64, i64, String), BridgeError> {
        db::users::anonymize_user(self.pool(), app_id, external_user_id, reason).await
    }
}
