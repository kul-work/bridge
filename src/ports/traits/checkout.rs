use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::checkout_idempotency::CachedCheckout,
    error::BridgeError,
};

#[async_trait]
pub trait CheckoutRepository: Send + Sync {
    async fn get_cached_checkout(
        &self,
        app_id: Uuid,
        idempotency_key: &str,
    ) -> Result<Option<CachedCheckout>, BridgeError>;

    async fn cache_checkout_response(
        &self,
        app_id: Uuid,
        idempotency_key: &str,
        request_fingerprint: &str,
        response_payload: &serde_json::Value,
    ) -> Result<(), BridgeError>;

    async fn has_live_subscription_for_product(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        product_id: &str,
    ) -> Result<bool, BridgeError>;
}
