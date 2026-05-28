use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::{self, checkout_idempotency::CachedCheckout},
    error::BridgeError,
    ports::traits::CheckoutRepository,
};

#[async_trait]
impl CheckoutRepository for db::Database {
    async fn get_cached_checkout(
        &self,
        app_id: Uuid,
        idempotency_key: &str,
    ) -> Result<Option<CachedCheckout>, BridgeError> {
        db::checkout_idempotency::get_cached_checkout(self.pool(), app_id, idempotency_key).await
    }

    async fn cache_checkout_response(
        &self,
        app_id: Uuid,
        idempotency_key: &str,
        request_fingerprint: &str,
        response_payload: &serde_json::Value,
    ) -> Result<(), BridgeError> {
        db::checkout_idempotency::cache_checkout_response(
            self.pool(),
            app_id,
            idempotency_key,
            request_fingerprint,
            response_payload,
        )
        .await
    }

    async fn has_live_subscription_for_product(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        product_id: &str,
    ) -> Result<bool, BridgeError> {
        db::subscriptions::has_live_subscription_for_product(
            self.pool(),
            app_id,
            external_user_id,
            provider,
            product_id,
        )
        .await
    }
}
