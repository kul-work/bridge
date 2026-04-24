use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{self},
    error::BridgeError,
    ports::UserSubscriptionCancellationSnapshot,
    ports::traits::UserRepository,
};

#[async_trait]
impl UserRepository for db::Database {
    async fn list_user_subscriptions_to_cancel(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Vec<UserSubscriptionCancellationSnapshot>, BridgeError> {
        db::users::list_user_subscriptions_to_cancel(self.pool(), app_id, external_user_id).await
    }

    async fn anonymize_user_records(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(i64, i64, String), BridgeError> {
        db::users::anonymize_user_records(self.pool(), app_id, external_user_id, reason).await
    }
}
