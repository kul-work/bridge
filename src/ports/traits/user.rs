use async_trait::async_trait;
use uuid::Uuid;

use crate::{error::BridgeError, ports::UserSubscriptionCancellationSnapshot};

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn list_user_subscriptions_to_cancel(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Vec<UserSubscriptionCancellationSnapshot>, BridgeError>;

    async fn anonymize_user_records(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(i64, i64, String), BridgeError>;
}
