use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{subscriptions::Subscription, webhooks::{WebhookDelivery, WebhookProvider}},
    error::BridgeError,
};

#[async_trait]
pub trait SchedulerRepository: Send + Sync {
    async fn list_enabled_app_ids(&self) -> Result<Vec<Uuid>, BridgeError>;

    async fn claim_pending_webhook_deliveries(
        &self,
        app_id: Uuid,
        worker_id: &str,
        lease_secs: i64,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, BridgeError>;

    async fn claim_unprocessed_webhook_providers(
        &self,
        app_id: Uuid,
        created_before: chrono::DateTime<chrono::Utc>,
        claim_expired_before: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<WebhookProvider>, BridgeError>;

    async fn list_reconciliation_subscriptions(
        &self,
        app_id: Uuid,
    ) -> Result<Vec<Subscription>, BridgeError>;

    async fn update_reconciled_subscription_status(
        &self,
        app_id: Uuid,
        id: Uuid,
        new_status: &str,
        current_period_end: Option<chrono::DateTime<chrono::Utc>>,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError>;

    async fn claim_price_step_up_expired_subscriptions(
        &self,
        app_id: Uuid,
        worker_id: &str,
        lease_secs: i64,
        limit: i64,
    ) -> Result<Vec<Subscription>, BridgeError>;

    async fn mark_subscription_price_step_up_expired(
        &self,
        app_id: Uuid,
        id: Uuid,
        claim_token: Uuid,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError>;

    async fn list_pending_pause_subscriptions(
        &self,
        limit: i64,
    ) -> Result<Vec<Subscription>, BridgeError>;

    async fn mark_subscription_paused(
        &self,
        app_id: Uuid,
        id: Uuid,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError>;

    async fn delete_orphaned_pending_subscriptions(&self) -> Result<u64, BridgeError>;

    async fn cleanup_old_webhook_provider(&self) -> Result<(), BridgeError>;

    async fn cleanup_purged_fraud_prevention(&self) -> Result<(), BridgeError>;
}
