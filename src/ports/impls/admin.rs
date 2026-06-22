use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{self, apps::AppSummary, webhooks::{WebhookDelivery, WebhookProvider}},
    error::BridgeError,
    ports::traits::AdminRepository,
};

#[async_trait]
impl AdminRepository for db::Database {
    async fn list_app_summaries(&self) -> Result<Vec<AppSummary>, BridgeError> {
        db::apps::list_app_summaries(self.pool()).await
    }

    async fn count_failed_webhooks(&self, app_id: Uuid) -> Result<i64, BridgeError> {
        db::webhooks::count_failed_webhooks(self.pool(), app_id).await
    }

    async fn count_app_webhooks(&self, app_id: Uuid) -> Result<i64, BridgeError> {
        db::webhooks::count_app_webhooks(self.pool(), app_id).await
    }

    async fn list_app_webhooks(
        &self,
        app_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<(WebhookDelivery, WebhookProvider)>, BridgeError> {
        db::webhooks::list_app_webhooks(self.pool(), app_id, limit, offset).await
    }

    async fn get_webhook_provider_for_delivery(
        &self,
        delivery_id: Uuid,
    ) -> Result<WebhookProvider, BridgeError> {
        db::webhooks::get_webhook_provider_for_delivery(self.pool(), delivery_id).await
    }
}
