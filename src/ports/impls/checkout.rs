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
}
