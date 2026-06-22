use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{apps::AppSummary, webhooks::{WebhookDelivery, WebhookProvider}},
    error::BridgeError,
};

#[async_trait]
pub trait AdminRepository: Send + Sync {
    async fn list_app_summaries(&self) -> Result<Vec<AppSummary>, BridgeError>;

    async fn count_failed_webhooks(&self, app_id: Uuid) -> Result<i64, BridgeError>;

    async fn list_app_webhooks(
        &self,
        app_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<(WebhookDelivery, WebhookProvider)>, BridgeError>;

    async fn get_webhook_provider_for_delivery(
        &self,
        delivery_id: Uuid,
    ) -> Result<WebhookProvider, BridgeError>;
}
