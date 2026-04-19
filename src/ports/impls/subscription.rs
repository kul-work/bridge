use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{self, subscriptions::Subscription},
    error::BridgeError,
    ports::helpers::{map_subscription_lookup_snapshot},
    ports::traits::{
        GooglePlayAccountLookupRepository, PurchaseOwnerLookupRepository, SubscriptionLookupRepository,
        SubscriptionReadRepository, SubscriptionRepository, SubscriptionWriteRepository,
    },
};

#[async_trait]
impl SubscriptionReadRepository for db::Database {
    async fn get_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::get_subscription(
            self.pool(),
            app_id,
            external_user_id,
            subscription_id,
            provider,
        )
        .await
    }

    async fn get_user_subscriptions_keyset(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        cursor_created_at: Option<chrono::DateTime<chrono::Utc>>,
        cursor_id: Option<Uuid>,
    ) -> Result<Vec<Subscription>, BridgeError> {
        db::subscriptions::get_user_subscriptions_keyset(
            self.pool(),
            app_id,
            external_user_id,
            limit,
            cursor_created_at,
            cursor_id,
        )
        .await
    }

    async fn get_user_subscriptions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Subscription>, BridgeError> {
        db::subscriptions::get_user_subscriptions(
            self.pool(),
            app_id,
            external_user_id,
            limit,
            offset,
        )
        .await
    }
}

#[async_trait]
impl SubscriptionWriteRepository for db::Database {
    async fn upsert_pending_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::upsert_pending_subscription(
            self.pool(),
            app_id,
            external_user_id,
            subscription_id,
            provider,
        )
        .await
    }

    async fn cancel_subscription_scheduled(
        &self,
        app_id: Uuid,
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::cancel_subscription_scheduled(self.pool(), app_id, id).await
    }

    async fn cancel_subscription_immediate(
        &self,
        app_id: Uuid,
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::cancel_subscription_immediate(self.pool(), app_id, id).await
    }

    async fn resume_subscription(
        &self,
        app_id: Uuid,
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::resume_subscription(self.pool(), app_id, id).await
    }

    async fn mark_payment_acknowledged_for_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        subscription_id: &str,
        purchase_token: Option<&str>,
    ) -> Result<(), BridgeError> {
        db::subscriptions::mark_payment_acknowledged_for_subscription(
            self.pool(),
            app_id,
            external_user_id,
            provider,
            subscription_id,
            purchase_token,
        )
        .await
    }

    async fn accept_price_step_up(
        &self,
        app_id: Uuid,
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::accept_price_step_up(self.pool(), app_id, id).await
    }

    async fn decline_price_step_up(
        &self,
        app_id: Uuid,
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::decline_price_step_up(self.pool(), app_id, id).await
    }

    async fn clear_payment_failure_notification(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        subscription_id: &str,
    ) -> Result<(), BridgeError> {
        db::subscriptions::clear_payment_failure_notification(
            self.pool(),
            app_id,
            external_user_id,
            provider,
            subscription_id,
        ).await
    }

    async fn delete_pending_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<(), BridgeError> {
        db::subscriptions::delete_pending_subscription(
            self.pool(),
            app_id,
            external_user_id,
            subscription_id,
            provider,
        )
        .await
    }
}

#[async_trait]
impl SubscriptionLookupRepository for db::Database {
    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, BridgeError> {
        db::subscriptions::get_subscription_by_sub_id(self.pool(), app_id, subscription_id)
            .await
            .map(|subscription| subscription.map(map_subscription_lookup_snapshot))
    }

    async fn get_subscription_by_sub_id_and_user(
        &self,
        app_id: Uuid,
        subscription_id: &str,
        external_user_id: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, BridgeError> {
        db::subscriptions::get_subscription_by_sub_id_and_user(self.pool(), app_id, subscription_id, external_user_id)
            .await
            .map(|subscription| subscription.map(map_subscription_lookup_snapshot))
    }

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<crate::ports::types::SubscriptionLookupSnapshot>, BridgeError> {
        db::subscriptions::get_subscription_by_purchase_token(self.pool(), app_id, purchase_token)
            .await
            .map(|subscription| subscription.map(map_subscription_lookup_snapshot))
    }
}

#[async_trait]
impl GooglePlayAccountLookupRepository for db::Database {
    async fn lookup_user_by_google_obfuscated_id(
        &self,
        app_id: Uuid,
        obfuscated_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::subscriptions::lookup_user_by_google_obfuscated_id(self.pool(), app_id, obfuscated_id).await
    }
}

#[async_trait]
impl PurchaseOwnerLookupRepository for db::Database {
    async fn lookup_user_by_subscription_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::subscriptions::lookup_user_by_subscription_id(self.pool(), app_id, subscription_id).await
    }

    async fn lookup_user_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::subscriptions::lookup_user_by_purchase_token(self.pool(), app_id, purchase_token).await
    }

    async fn lookup_user_by_purchase_token_payment(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::payments::lookup_user_by_purchase_token_payment(self.pool(), app_id, purchase_token).await
    }
}

#[async_trait]
impl SubscriptionRepository for db::Database {}
