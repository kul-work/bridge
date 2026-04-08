use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{self, apps::App, webhooks::{WebhookDelivery, WebhookProvider}},
    error::BridgeError,
    ports::traits::AdminRepository,
};

#[async_trait]
impl AdminRepository for db::Database {
    async fn list_apps(&self) -> Result<Vec<App>, BridgeError> {
        db::apps::list_apps(self.pool()).await
    }

    async fn count_failed_webhooks(&self, app_id: Uuid) -> Result<i64, BridgeError> {
        db::webhooks::count_failed_webhooks(self.pool(), app_id).await
    }

    async fn list_app_webhooks(
        &self,
        app_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<(WebhookDelivery, WebhookProvider)>, BridgeError> {
        db::webhooks::list_app_webhooks(self.pool(), app_id, limit, offset).await
    }
}
