use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{
        self,
        webhooks::{WebhookDelivery, WebhookDeliveryEnqueue, WebhookRecord},
    },
    error::BridgeError,
    ports::traits::{
        WebhookForwardRepository, WebhookProviderLookupRepository, WebhookReadRepository,
        WebhookRepository, WebhookSuppressionRepository, WebhookWriteRepository,
    },
};

#[async_trait]
impl WebhookReadRepository for db::Database {
    async fn list_user_webhook_records(
        &self,
        app_id: Uuid,
        subscription_ids: &[String],
        purchase_tokens: &[String],
    ) -> Result<Vec<WebhookRecord>, BridgeError> {
        db::webhooks::list_user_webhook_records(
            self.pool(),
            app_id,
            subscription_ids,
            purchase_tokens,
        )
        .await
    }
}

#[async_trait]
impl WebhookWriteRepository for db::Database {
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
    ) -> Result<(Uuid, bool), BridgeError> {
        db::webhooks::create_webhook_provider(
            self.pool(),
            app_id,
            provider,
            provider_webhook_id,
            event_type,
            subscription_id,
            purchase_token,
            payload,
            timestamp_epoch_ms,
        )
        .await
    }

    async fn create_webhook_delivery(
        &self,
        app_id: Uuid,
        webhook_provider_id: Uuid,
    ) -> Result<WebhookDeliveryEnqueue, BridgeError> {
        db::webhooks::create_webhook_delivery(self.pool(), app_id, webhook_provider_id).await
    }
}

#[async_trait]
impl WebhookSuppressionRepository for db::Database {
    async fn suppress_webhook(&self, webhook_id: Uuid, reason: &str) -> Result<(), BridgeError> {
        db::webhooks::suppress_webhook(self.pool(), webhook_id, reason).await
    }
}

#[async_trait]
impl WebhookForwardRepository for db::Database {
    async fn get_webhook_delivery(&self, id: Uuid) -> Result<WebhookDelivery, BridgeError> {
        db::webhooks::get_webhook_delivery(self.pool(), id).await
    }

    async fn webhook_delivery_exists(
        &self,
        webhook_provider_id: Uuid,
    ) -> Result<bool, BridgeError> {
        db::webhooks::webhook_delivery_exists(self.pool(), webhook_provider_id).await
    }

    async fn update_webhook_delivery_attempt(
        &self,
        delivery_id: Uuid,
        http_status: Option<i32>,
        error: Option<String>,
        forwarded: bool,
    ) -> Result<WebhookDelivery, BridgeError> {
        db::webhooks::update_webhook_delivery_attempt(
            self.pool(),
            delivery_id,
            http_status,
            error,
            forwarded,
        )
        .await
    }

    async fn reset_webhook_delivery(&self, delivery_id: Uuid) -> Result<bool, BridgeError> {
        db::webhooks::reset_webhook_delivery(self.pool(), delivery_id).await
    }
}

#[async_trait]
impl WebhookProviderLookupRepository for db::Database {
    async fn get_webhook_provider(
        &self,
        id: Uuid,
    ) -> Result<crate::ports::types::WebhookProviderSnapshot, BridgeError> {
        let webhook = db::webhooks::get_webhook_provider(self.pool(), id).await?;
        Ok(crate::ports::types::WebhookProviderSnapshot {
            provider: webhook.provider,
            provider_webhook_id: webhook.provider_webhook_id,
            event_type: webhook.event_type,
            subscription_id: webhook.subscription_id,
            purchase_token: webhook.purchase_token,
            payload: webhook.payload,
            processed: webhook.processed,
            timestamp_epoch_ms: webhook.timestamp_epoch_ms,
            suppressed: webhook.suppressed,
            suppressed_reason: webhook.suppressed_reason,
        })
    }
}

#[async_trait]
impl WebhookRepository for db::Database {}
