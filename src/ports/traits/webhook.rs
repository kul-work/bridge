use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::webhooks::{WebhookDelivery, WebhookDeliveryEnqueue, WebhookRecord},
    error::BridgeError,
};

#[async_trait]
pub trait WebhookReadRepository: Send + Sync {
    async fn list_user_webhook_records(
        &self,
        app_id: Uuid,
        subscription_ids: &[String],
        purchase_tokens: &[String],
    ) -> Result<Vec<WebhookRecord>, BridgeError>;
}

#[async_trait]
#[allow(clippy::too_many_arguments)]
pub trait WebhookWriteRepository: Send + Sync {
    async fn create_webhook_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_webhook_id: &str,
        event_type: &str,
        subscription_id: Option<String>,
        purchase_token: Option<String>,
        payload: serde_json::Value,
        timestamp_epoch_ms: Option<i64>,
    ) -> Result<(Uuid, bool), BridgeError>;

    async fn create_webhook_delivery(
        &self,
        app_id: Uuid,
        webhook_provider_id: Uuid,
    ) -> Result<WebhookDeliveryEnqueue, BridgeError>;
}

#[async_trait]
pub trait WebhookForwardRepository:
    crate::ports::traits::subscription::SubscriptionLookupRepository 
    + WebhookSuppressionRepository 
    + Send 
    + Sync
{
    async fn get_webhook_delivery(&self, id: Uuid) -> Result<WebhookDelivery, BridgeError>;

    async fn webhook_delivery_exists(&self, webhook_provider_id: Uuid) -> Result<bool, BridgeError>;

    async fn update_webhook_delivery_attempt(
        &self,
        delivery_id: Uuid,
        http_status: Option<i32>,
        error: Option<String>,
        forwarded: bool,
    ) -> Result<(), BridgeError>;

    async fn reset_webhook_delivery(&self, delivery_id: Uuid) -> Result<bool, BridgeError>;
}

#[async_trait]
pub trait WebhookSuppressionRepository: Send + Sync {
    async fn suppress_webhook(&self, webhook_id: Uuid, reason: &str) -> Result<(), BridgeError>;
}

#[async_trait]
pub trait WebhookProviderLookupRepository: Send + Sync {
    async fn get_webhook_provider(&self, id: Uuid) -> Result<crate::ports::types::WebhookProviderSnapshot, BridgeError>;
}

#[async_trait]
pub trait WebhookRepository:
    WebhookWriteRepository
    + WebhookReadRepository
    + WebhookForwardRepository
    + WebhookSuppressionRepository
    + WebhookProviderLookupRepository
    + Send
    + Sync
{
}
