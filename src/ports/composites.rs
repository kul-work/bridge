use uuid::Uuid;

use crate::ports::traits::*;

pub trait VerifyPurchaseHandlerRepository:
    AppConfigRepository
    + PaymentRepository
    + SubscriptionRepository
    + VerifyPurchaseRepository
    + WebhookRepository
    + Send
    + Sync
{
}

impl<T> VerifyPurchaseHandlerRepository for T
where
    T: AppConfigRepository
        + PaymentRepository
        + SubscriptionRepository
        + VerifyPurchaseRepository
        + WebhookRepository
        + Send
        + Sync,
{
}

pub trait SubscriptionActionsHandlerRepository:
    AppConfigRepository + SubscriptionRepository + WebhookRepository + Send + Sync
{
}

impl<T> SubscriptionActionsHandlerRepository for T
where
    T: AppConfigRepository + SubscriptionRepository + WebhookRepository + Send + Sync,
{
}

pub trait CheckoutHandlerRepository:
    AppConfigRepository + CheckoutRepository + Send + Sync
{
}

impl<T> CheckoutHandlerRepository for T
where
    T: AppConfigRepository + CheckoutRepository + Send + Sync,
{
}

#[async_trait::async_trait]
pub trait WebhookIngressRepository:
    AppConfigRepository + WebhookRepository + Send + Sync
{
    async fn get_app_by_webhook_token(&self, token: Uuid) -> Result<crate::application::app_context::AppSnapshot, crate::error::BridgeError>;
}

#[async_trait::async_trait]
impl WebhookIngressRepository for crate::db::Database {
    async fn get_app_by_webhook_token(&self, token: Uuid) -> Result<crate::application::app_context::AppSnapshot, crate::error::BridgeError> {
        crate::db::apps::get_app_by_webhook_token(self.pool(), token)
            .await
            .map(|app| crate::application::app_context::AppSnapshot {
                id: app.id,
                slug: app.slug,
                display_name: app.display_name,
                webhook_callback_url: app.webhook_callback_url,
                webhook_callback_secret: app.webhook_callback_secret,
                api_rate_limit_per_minute: app.api_rate_limit_per_minute,
                api_rate_limit_rules: app.api_rate_limit_rules,
                app_url: app.app_url,
                google_package_name: app.google_package_name,
                apple_bundle_id: app.apple_bundle_id,
            })
    }
}

#[async_trait::async_trait]
pub trait WebhookProcessingTransactionRepository: Send + Sync {
    async fn record_webhook_payment(
        &self,
        request: crate::ports::types::WebhookPaymentRecordRequest<'_>,
    ) -> Result<(), crate::error::BridgeError>;

    async fn commit_webhook_subscription(
        &self,
        request: crate::ports::types::WebhookSubscriptionCommitRequest<'_>,
    ) -> Result<Option<crate::ports::types::WebhookSubscriptionSnapshot>, crate::error::BridgeError>;
}

#[async_trait::async_trait]
pub trait WebhookProcessingLookupRepository:
    GooglePlayAccountLookupRepository
    + SubscriptionLookupRepository
    + PurchaseOwnerLookupRepository
    + WebhookProviderLookupRepository
    + PaymentReadRepository
    + Send
    + Sync
{
    async fn get_subscription_by_sub_id_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        subscription_id: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, crate::error::BridgeError>;

    async fn get_subscription_by_sub_id_and_user_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        subscription_id: &str,
        external_user_id: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, crate::error::BridgeError>;

    async fn get_subscription_by_purchase_token_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        purchase_token: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, crate::error::BridgeError>;

}

#[async_trait::async_trait]
impl WebhookProcessingLookupRepository for crate::db::Database {
    async fn get_subscription_by_sub_id_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        subscription_id: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, crate::error::BridgeError> {
        crate::db::subscriptions::get_subscription_by_sub_id_for_provider(self.pool(), app_id, provider, subscription_id)
            .await
            .map(|subscription| subscription.map(crate::ports::helpers::map_subscription_lookup_snapshot))
    }

    async fn get_subscription_by_sub_id_and_user_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        subscription_id: &str,
        external_user_id: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, crate::error::BridgeError> {
        crate::db::subscriptions::get_subscription_by_sub_id_and_user_for_provider(
            self.pool(),
            app_id,
            provider,
            subscription_id,
            external_user_id,
        )
        .await
        .map(|subscription| subscription.map(crate::ports::helpers::map_subscription_lookup_snapshot))
    }

    async fn get_subscription_by_purchase_token_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        purchase_token: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, crate::error::BridgeError> {
        crate::db::subscriptions::get_subscription_by_purchase_token_for_provider(self.pool(), app_id, provider, purchase_token)
            .await
            .map(|subscription| subscription.map(crate::ports::helpers::map_subscription_lookup_snapshot))
    }

}

#[async_trait::async_trait]
pub trait WebhookProcessingMutationRepository: WebhookSuppressionRepository + Send + Sync {
    async fn update_payment_status_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
        new_status: &str,
    ) -> Result<(), crate::error::BridgeError>;

    async fn apply_subscription_transition(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        subscription_id: &str,
        event_time_ms: i64,
        transition: crate::ports::types::SubscriptionWebhookTransition,
    ) -> Result<Option<crate::db::subscriptions::Subscription>, crate::error::BridgeError>;

    async fn link_replacement_subscriptions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        current_subscription_id: &str,
        last_event_time: i64,
    ) -> Result<(), crate::error::BridgeError>;

    async fn mark_webhook_processed(&self, webhook_id: Uuid) -> Result<(), crate::error::BridgeError>;
}

#[async_trait::async_trait]
pub trait WebhookProcessingRepository:
    AppConfigRepository
    + SubscriptionRepository
    + PaymentRepository
    + WebhookRepository
    + WebhookProcessingLookupRepository
    + WebhookProcessingMutationRepository
    + WebhookProcessingTransactionRepository
    + Send
    + Sync
{
}

#[async_trait::async_trait]
impl WebhookProcessingRepository for crate::db::Database {
}

#[async_trait::async_trait]
pub trait VerifyPurchaseRepository: Send + Sync {
    async fn get_subscription_snapshot(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Option<crate::application::verify_purchase_types::VerifyPurchaseSubscriptionSnapshot>, crate::error::BridgeError>;

    async fn commit_verified_purchase(
        &self,
        request: crate::application::verify_purchase_types::VerifyPurchaseCommitRequest<'_>,
    ) -> Result<crate::application::verify_purchase_types::VerifyPurchaseCommitResult, crate::error::BridgeError>;
}
