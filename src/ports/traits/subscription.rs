use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::subscriptions::Subscription,
    error::BridgeError,
};

#[async_trait]
pub trait SubscriptionReadRepository: Send + Sync {
    async fn get_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Subscription, BridgeError>;

    async fn get_user_subscriptions_keyset(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        cursor_created_at: Option<chrono::DateTime<chrono::Utc>>,
        cursor_id: Option<Uuid>,
    ) -> Result<Vec<Subscription>, BridgeError>;

    async fn get_user_subscriptions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Subscription>, BridgeError>;
}

#[async_trait]
pub trait SubscriptionWriteRepository: Send + Sync {
    async fn upsert_pending_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Subscription, BridgeError>;

    async fn cancel_subscription_scheduled(
        &self,
        id: Uuid,
    ) -> Result<Subscription, BridgeError>;

    async fn cancel_subscription_immediate(
        &self,
        id: Uuid,
    ) -> Result<Subscription, BridgeError>;

    async fn resume_subscription(
        &self,
        id: Uuid,
    ) -> Result<Subscription, BridgeError>;

    async fn mark_payment_acknowledged_for_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        subscription_id: &str,
        purchase_token: Option<&str>,
    ) -> Result<(), BridgeError>;

    async fn accept_price_step_up(
        &self,
        id: Uuid,
    ) -> Result<Subscription, BridgeError>;

    async fn decline_price_step_up(
        &self,
        id: Uuid,
    ) -> Result<Subscription, BridgeError>;
}

#[async_trait]
pub trait SubscriptionLookupRepository: Send + Sync {
    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, BridgeError>;

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, BridgeError>;
}

#[async_trait]
pub trait GooglePlayAccountLookupRepository: Send + Sync {
    async fn lookup_user_by_google_obfuscated_id(
        &self,
        app_id: Uuid,
        obfuscated_id: &str,
    ) -> Result<Option<String>, BridgeError>;
}

#[async_trait]
pub trait PurchaseOwnerLookupRepository: Send + Sync {
    async fn lookup_user_by_subscription_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<String>, BridgeError>;

    async fn lookup_user_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<String>, BridgeError>;

    async fn lookup_user_by_purchase_token_payment(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<String>, BridgeError>;
}

#[async_trait]
pub trait SubscriptionRepository:
    SubscriptionReadRepository
    + SubscriptionWriteRepository
    + SubscriptionLookupRepository
    + PurchaseOwnerLookupRepository
    + GooglePlayAccountLookupRepository
    + Send
    + Sync
{
}
