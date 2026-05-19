use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    db::payments::{Payment, PaymentHistoryEntry},
    error::BridgeError,
};

#[async_trait]
pub trait PaymentReadRepository: Send + Sync {
    async fn count_user_payments(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<i64, BridgeError>;

    async fn list_user_payments_keyset(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        after_created_at: Option<chrono::DateTime<chrono::Utc>>,
        after_id: Option<Uuid>,
    ) -> Result<Vec<PaymentHistoryEntry>, BridgeError>;

    async fn get_user_payments(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Payment>, BridgeError>;

    async fn get_payment_status_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<Option<String>, BridgeError>;

    async fn get_payment_currency_for_subscription(
        &self,
        app_id: Uuid,
        provider: &str,
        external_user_id: &str,
        subscription_id: &str,
    ) -> Result<Option<String>, BridgeError>;
}

#[async_trait]
pub trait PaymentAcknowledgementRepository: Send + Sync {
    async fn payment_acknowledged_at(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, BridgeError>;

    async fn mark_payment_acknowledged(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<(), BridgeError>;
}

#[async_trait]
pub trait PaymentRepository:
    PaymentReadRepository + PaymentAcknowledgementRepository + Send + Sync
{
}
