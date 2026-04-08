use async_trait::async_trait;
use std::{future::Future, pin::Pin};
use uuid::Uuid;

use crate::{
    application::app_context::{AppSnapshot, ProviderConfigSnapshot},
    application::verify_purchase_types::{
        VerifyPurchaseCommitRequest, VerifyPurchaseCommitResult,
        VerifyPurchaseSubscriptionSnapshot,
    },
    db::{
        self,
        api_keys::AuthenticatedApiKey,
        agent::{AgentCredit, AgentTransaction},
        apps::App,
        checkout_idempotency::CachedCheckout,
        payments::{Payment, PaymentHistoryEntry},
        subscriptions::Subscription,
        webhooks::{WebhookDelivery, WebhookProvider, WebhookRecord},
    },
    error::BridgeError,
};

#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn authenticate_api_key(
        &self,
        raw_key: &str,
    ) -> Result<AuthenticatedApiKey, BridgeError>;
}

#[async_trait]
pub trait AdminRepository: Send + Sync {
    async fn list_apps(&self) -> Result<Vec<App>, BridgeError>;

    async fn count_failed_webhooks(&self, app_id: Uuid) -> Result<i64, BridgeError>;

    async fn list_app_webhooks(
        &self,
        app_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<(WebhookDelivery, WebhookProvider)>, BridgeError>;
}

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
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn anonymize_user(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(i64, i64, String), BridgeError>;
}

#[async_trait]
pub trait AgentReadRepository: Send + Sync {
    async fn get_agent_credit(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Option<AgentCredit>, BridgeError>;

    async fn list_agent_transactions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Vec<AgentTransaction>, BridgeError>;
}

#[async_trait]
pub trait AgentRepository: AgentReadRepository + Send + Sync {
    async fn upsert_agent_credit(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        balance_delta: i32,
        spent_delta: i32,
    ) -> Result<AgentCredit, BridgeError>;

    async fn insert_agent_token(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        endpoint: &str,
        amount_cents: i32,
        nonce: &str,
    ) -> Result<crate::db::agent::AgentPaymentToken, BridgeError>;

    async fn charge_agent(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        token_id: Uuid,
        endpoint: &str,
    ) -> Result<(i32, i32), BridgeError>;

    async fn topup_agent(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        amount_cents: i32,
        charge_id: Option<&str>,
    ) -> Result<AgentCredit, BridgeError>;
}

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
pub trait SubscriptionLookupRepository: Send + Sync {
    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError>;

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError>;
}

#[derive(Debug, Clone)]
pub struct SubscriptionLookupSnapshot {
    pub id: Uuid,
    pub external_user_id: String,
    pub provider: String,
    pub subscription_id: String,
    pub purchase_token: Option<String>,
    pub status: String,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub auto_renewing: Option<bool>,
    pub revocation_reason: Option<String>,
    pub last_event_time: i64,
}

#[derive(Debug, Clone)]
pub enum SubscriptionWebhookTransition {
    Pending,
    GracePeriod {
        grace_period_end: Option<chrono::DateTime<chrono::Utc>>,
    },
    Revoked {
        revocation_reason: Option<String>,
    },
    OnHold,
    Paused,
    Resumed,
    CancellationScheduled {
        google_cancellation_context: Option<String>,
        google_cancellation_feedback: Option<String>,
    },
    Expired,
    Cancelled {
        current_period_end: Option<chrono::DateTime<chrono::Utc>>,
        google_cancellation_context: Option<String>,
        google_cancellation_feedback: Option<String>,
    },
    PaymentFailed,
    PendingPurchaseCancelled,
    PriceStepUp {
        google_new_price_cents: Option<i32>,
        google_price_step_up_consent_deadline: Option<chrono::DateTime<chrono::Utc>>,
    },
    PauseScheduled {
        google_pause_scheduled_at: chrono::DateTime<chrono::Utc>,
    },
    Deferred {
        google_deferred_until: chrono::DateTime<chrono::Utc>,
    },
}

#[async_trait]
pub trait VerifyPurchaseRepository: Send + Sync {
    async fn get_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Option<VerifyPurchaseSubscriptionSnapshot>, BridgeError>;

    async fn commit_verified_purchase(
        &self,
        request: VerifyPurchaseCommitRequest<'_>,
    ) -> Result<VerifyPurchaseCommitResult, BridgeError>;
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
    ) -> Result<Uuid, BridgeError>;
}

#[async_trait]
pub trait WebhookIngressRepository:
    AppLookupRepository + ProviderConfigLookupRepository + WebhookWriteRepository + Send + Sync
{
    async fn get_app_by_webhook_token(&self, token: Uuid) -> Result<AppSnapshot, BridgeError>;
}

#[async_trait]
pub trait AppLookupRepository: Send + Sync {
    async fn get_app(&self, app_id: Uuid) -> Result<AppSnapshot, BridgeError>;
}

#[async_trait]
pub trait ProviderConfigLookupRepository: Send + Sync {
    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfigSnapshot, BridgeError>;
}

#[async_trait]
pub trait WebhookForwardRepository:
    WebhookSuppressionRepository + SubscriptionLookupRepository + Send + Sync
{
    async fn get_webhook_delivery(&self, id: Uuid) -> Result<WebhookDelivery, BridgeError>;

    async fn update_webhook_delivery_attempt(
        &self,
        delivery_id: Uuid,
        http_status: Option<i32>,
        error: Option<String>,
        forwarded: bool,
    ) -> Result<(), BridgeError>;
}

pub(crate) trait VerifyPurchaseHandlerRepository:
    AppLookupRepository
    + GooglePlayAccountLookupRepository
    + PaymentAcknowledgementRepository
    + ProviderConfigLookupRepository
    + SubscriptionLookupRepository
    + VerifyPurchaseRepository
    + WebhookForwardRepository
    + WebhookWriteRepository
    + Send
    + Sync
{
}

impl<T> VerifyPurchaseHandlerRepository for T
where
    T: AppLookupRepository
        + GooglePlayAccountLookupRepository
        + PaymentAcknowledgementRepository
        + ProviderConfigLookupRepository
        + SubscriptionLookupRepository
        + VerifyPurchaseRepository
        + WebhookForwardRepository
        + WebhookWriteRepository
        + Send
        + Sync,
{
}

pub(crate) trait SubscriptionActionsHandlerRepository:
    AppLookupRepository
    + ProviderConfigLookupRepository
    + SubscriptionLookupRepository
    + SubscriptionReadRepository
    + SubscriptionWriteRepository
    + WebhookForwardRepository
    + WebhookWriteRepository
    + Send
    + Sync
{
}

impl<T> SubscriptionActionsHandlerRepository for T
where
    T: AppLookupRepository
        + ProviderConfigLookupRepository
        + SubscriptionLookupRepository
        + SubscriptionReadRepository
        + SubscriptionWriteRepository
        + WebhookForwardRepository
        + WebhookWriteRepository
        + Send
        + Sync,
{
}

#[derive(Debug, Clone)]
pub struct WebhookPaymentRecordRequest<'a> {
    pub app_id: Uuid,
    pub external_user_id: &'a str,
    pub provider: &'a str,
    pub provider_transaction_id: &'a str,
    pub subscription_id: Option<&'a str>,
    pub amount_cents: i32,
    pub status: &'a str,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionCommitRequest<'a> {
    pub app_id: Uuid,
    pub external_user_id: &'a str,
    pub subscription_id: &'a str,
    pub provider: &'a str,
    pub status: &'a str,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub purchase_token: Option<&'a str>,
    pub auto_renewing: Option<bool>,
    pub payment_state: Option<i32>,
    pub provider_customer_id: Option<&'a str>,
    pub event_time_ms: i64,
    pub payment: Option<WebhookPaymentRecordRequest<'a>>,
    pub adopt_stale_payment: bool,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionSnapshot {
    pub purchase_token: Option<String>,
    pub status: String,
    pub current_period_end: Option<chrono::DateTime<chrono::Utc>>,
    pub auto_renewing: Option<bool>,
    pub revocation_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WebhookProviderSnapshot {
    pub provider: String,
    pub provider_webhook_id: String,
    pub event_type: String,
    pub subscription_id: Option<String>,
    pub purchase_token: Option<String>,
    pub payload: serde_json::Value,
    pub timestamp_epoch_ms: Option<i64>,
    pub suppressed: bool,
    pub suppressed_reason: Option<String>,
}

#[async_trait]
pub trait WebhookProcessingTransactionRepository: Send + Sync {
    async fn record_webhook_payment(
        &self,
        request: WebhookPaymentRecordRequest<'_>,
    ) -> Result<(), BridgeError>;

    async fn commit_webhook_subscription(
        &self,
        request: WebhookSubscriptionCommitRequest<'_>,
    ) -> Result<Option<WebhookSubscriptionSnapshot>, BridgeError>;
}

#[async_trait]
pub trait WebhookProcessingLookupRepository:
    GooglePlayAccountLookupRepository
    + SubscriptionLookupRepository
    + PurchaseOwnerLookupRepository
    + WebhookProviderLookupRepository
    + PaymentStatusLookupRepository
    + Send
    + Sync
{
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
pub trait WebhookProviderLookupRepository: Send + Sync {
    async fn get_webhook_provider(&self, id: Uuid) -> Result<WebhookProviderSnapshot, BridgeError>;
}

#[async_trait]
pub trait PaymentStatusLookupRepository: Send + Sync {
    async fn get_payment_status(
        &self,
        app_id: Uuid,
        provider_transaction_id: &str,
    ) -> Result<Option<String>, BridgeError>;
}

#[async_trait]
pub trait WebhookProcessingMutationRepository: WebhookSuppressionRepository + Send + Sync {
    async fn update_payment_status(
        &self,
        app_id: Uuid,
        provider_transaction_id: &str,
        new_status: &str,
    ) -> Result<(), BridgeError>;

    async fn apply_subscription_transition(
        &self,
        app_id: Uuid,
        subscription_id: &str,
        event_time_ms: i64,
        transition: SubscriptionWebhookTransition,
    ) -> Result<Option<Subscription>, BridgeError>;

    async fn link_replacement_subscriptions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        current_subscription_id: &str,
        last_event_time: i64,
    ) -> Result<(), BridgeError>;

    async fn apply_topup_if_new(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        amount_cents: i32,
        charge_id: &str,
    ) -> Result<bool, BridgeError>;

    async fn mark_webhook_processed(&self, webhook_id: Uuid) -> Result<(), BridgeError>;
}

#[async_trait]
pub trait WebhookProcessingRepository:
    AppLookupRepository
    + ProviderConfigLookupRepository
    + WebhookProcessingLookupRepository
    + WebhookProcessingMutationRepository
    + WebhookProcessingTransactionRepository
    + Send
    + Sync
{
}

#[async_trait]
pub trait WebhookSuppressionRepository: Send + Sync {
    async fn suppress_webhook(&self, webhook_id: Uuid, reason: &str) -> Result<(), BridgeError>;
}

#[async_trait]
pub trait SchedulerRepository: Send + Sync {
    async fn list_enabled_apps(&self) -> Result<Vec<App>, BridgeError>;

    async fn list_pending_webhook_deliveries(
        &self,
        app_id: Uuid,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, BridgeError>;

    async fn list_reconciliation_subscriptions(
        &self,
        app_id: Uuid,
    ) -> Result<Vec<Subscription>, BridgeError>;

    async fn update_subscription_status(
        &self,
        app_id: Uuid,
        subscription_id: &str,
        new_status: &str,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError>;

    async fn list_price_step_up_expired_subscriptions(
        &self,
        limit: i64,
    ) -> Result<Vec<Subscription>, BridgeError>;

    async fn mark_subscription_price_step_up_expired(
        &self,
        id: Uuid,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError>;

    async fn list_pending_pause_subscriptions(
        &self,
        limit: i64,
    ) -> Result<Vec<Subscription>, BridgeError>;

    async fn mark_subscription_paused(
        &self,
        id: Uuid,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError>;

    async fn delete_orphaned_pending_subscriptions(&self) -> Result<u64, BridgeError>;

    async fn cleanup_old_webhook_provider(&self) -> Result<(), BridgeError>;

    async fn cleanup_expired_agent_tokens(&self) -> Result<(), BridgeError>;

    async fn cleanup_purged_fraud_prevention(&self) -> Result<(), BridgeError>;
}

#[derive(Debug)]
pub enum TransactionOutcome<T> {
    Commit(T),
    Rollback(T),
}

struct OwnedWebhookPaymentRecord {
    app_id: Uuid,
    external_user_id: String,
    provider: String,
    provider_transaction_id: String,
    subscription_id: Option<String>,
    amount_cents: i32,
    status: String,
}

impl<'a> From<WebhookPaymentRecordRequest<'a>> for OwnedWebhookPaymentRecord {
    fn from(request: WebhookPaymentRecordRequest<'a>) -> Self {
        Self {
            app_id: request.app_id,
            external_user_id: request.external_user_id.to_string(),
            provider: request.provider.to_string(),
            provider_transaction_id: request.provider_transaction_id.to_string(),
            subscription_id: request.subscription_id.map(str::to_string),
            amount_cents: request.amount_cents,
            status: request.status.to_string(),
        }
    }
}

impl From<Subscription> for WebhookSubscriptionSnapshot {
    fn from(subscription: Subscription) -> Self {
        Self {
            purchase_token: subscription.purchase_token,
            status: subscription.status,
            current_period_end: subscription.current_period_end,
            auto_renewing: subscription.auto_renewing,
            revocation_reason: subscription.revocation_reason,
        }
    }
}

fn map_subscription_lookup_snapshot(subscription: Subscription) -> SubscriptionLookupSnapshot {
    SubscriptionLookupSnapshot {
        id: subscription.id,
        external_user_id: subscription.external_user_id,
        provider: subscription.provider,
        subscription_id: subscription.subscription_id,
        purchase_token: subscription.purchase_token,
        status: subscription.status,
        current_period_end: subscription.current_period_end,
        auto_renewing: subscription.auto_renewing,
        revocation_reason: subscription.revocation_reason,
        last_event_time: subscription.last_event_time,
    }
}

fn map_verify_purchase_subscription(
    subscription: Subscription,
) -> VerifyPurchaseSubscriptionSnapshot {
    VerifyPurchaseSubscriptionSnapshot {
        current_period_end: subscription.current_period_end,
        auto_renewing: subscription.auto_renewing,
        payment_state: subscription.payment_state,
        provider_customer_id: subscription.provider_customer_id,
    }
}

fn with_transaction_impl<'a, T, F>(
    pool: &'a sqlx::PgPool,
    f: F,
) -> Pin<Box<dyn Future<Output = Result<T, BridgeError>> + Send + 'a>>
where
    T: Send + 'a,
    F: for<'tx> FnOnce(
            &'tx mut sqlx::Transaction<'a, sqlx::Postgres>,
        ) -> Pin<Box<dyn Future<Output = Result<TransactionOutcome<T>, BridgeError>> + Send + 'tx>>
        + Send
        + 'a,
{
    Box::pin(async move {
        let mut tx = pool
            .begin()
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;

        let outcome = f(&mut tx).await?;

        match outcome {
            TransactionOutcome::Commit(value) => {
                tx.commit()
                    .await
                    .map_err(|e| BridgeError::DbError(e.to_string()))?;
                Ok(value)
            }
            TransactionOutcome::Rollback(value) => {
                tx.rollback()
                    .await
                    .map_err(|e| BridgeError::DbError(e.to_string()))?;
                Ok(value)
            }
        }
    })
}
#[async_trait]
impl ApiKeyRepository for db::Database {
    async fn authenticate_api_key(
        &self,
        raw_key: &str,
    ) -> Result<AuthenticatedApiKey, BridgeError> {
        let pool = self.pool();
        db::api_keys::authenticate_api_key(pool, raw_key).await
    }
}

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
impl AdminRepository for db::Database {
    async fn list_apps(&self) -> Result<Vec<App>, BridgeError> {
        db::apps::list_apps(self.pool()).await
    }

    async fn count_failed_webhooks(&self, app_id: Uuid) -> Result<i64, BridgeError> {
        db::webhooks::count_failed_webhooks(self.pool(), app_id).await
    }

    async fn list_app_webhooks(
        &self,
        app_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<(WebhookDelivery, WebhookProvider)>, BridgeError> {
        db::webhooks::list_app_webhooks(self.pool(), app_id, limit, offset).await
    }
}

#[async_trait]
impl PaymentReadRepository for db::Database {
    async fn count_user_payments(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<i64, BridgeError> {
        db::payments::count_user_payments(self.pool(), app_id, external_user_id).await
    }

    async fn list_user_payments_keyset(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        after_created_at: Option<chrono::DateTime<chrono::Utc>>,
        after_id: Option<Uuid>,
    ) -> Result<Vec<PaymentHistoryEntry>, BridgeError> {
        db::payments::list_user_payments_keyset(
            self.pool(),
            app_id,
            external_user_id,
            limit,
            after_created_at,
            after_id,
        )
        .await
    }

    async fn get_user_payments(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Payment>, BridgeError> {
        db::payments::get_user_payments(self.pool(), app_id, external_user_id, limit, offset).await
    }
}

#[async_trait]
impl AgentRepository for db::Database {
    async fn upsert_agent_credit(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        balance_delta: i32,
        spent_delta: i32,
    ) -> Result<AgentCredit, BridgeError> {
        db::agent::upsert_agent_credit(
            self.pool(),
            app_id,
            external_user_id,
            balance_delta,
            spent_delta,
        )
        .await
    }

    async fn insert_agent_token(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        endpoint: &str,
        amount_cents: i32,
        nonce: &str,
    ) -> Result<crate::db::agent::AgentPaymentToken, BridgeError> {
        db::agent::insert_agent_token(
            self.pool(),
            app_id,
            external_user_id,
            endpoint,
            amount_cents,
            nonce,
        )
        .await
    }

    async fn charge_agent(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        token_id: Uuid,
        endpoint: &str,
    ) -> Result<(i32, i32), BridgeError> {
        db::agent::charge_agent(self.pool(), app_id, external_user_id, token_id, endpoint).await
    }

    async fn topup_agent(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        amount_cents: i32,
        charge_id: Option<&str>,
    ) -> Result<AgentCredit, BridgeError> {
        db::agent::topup_agent(self.pool(), app_id, external_user_id, amount_cents, charge_id).await
    }
}

#[async_trait]
impl UserRepository for db::Database {
    async fn anonymize_user(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        reason: Option<&str>,
    ) -> Result<(i64, i64, String), BridgeError> {
        db::users::anonymize_user(self.pool(), app_id, external_user_id, reason).await
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
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::cancel_subscription_scheduled(self.pool(), id).await
    }

    async fn cancel_subscription_immediate(
        &self,
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::cancel_subscription_immediate(self.pool(), id).await
    }

    async fn resume_subscription(
        &self,
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::resume_subscription(self.pool(), id).await
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
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::accept_price_step_up(self.pool(), id).await
    }

    async fn decline_price_step_up(
        &self,
        id: Uuid,
    ) -> Result<Subscription, BridgeError> {
        db::subscriptions::decline_price_step_up(self.pool(), id).await
    }
}

#[async_trait]
impl AgentReadRepository for db::Database {
    async fn get_agent_credit(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Option<AgentCredit>, BridgeError> {
        db::agent::get_agent_credit(self.pool(), app_id, external_user_id).await
    }

    async fn list_agent_transactions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
    ) -> Result<Vec<AgentTransaction>, BridgeError> {
        db::agent::list_agent_transactions(self.pool(), app_id, external_user_id).await
    }
}

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
impl AppLookupRepository for db::Database {
    async fn get_app(&self, app_id: Uuid) -> Result<AppSnapshot, BridgeError> {
        db::apps::get_app(self.pool(), app_id)
            .await
            .map(|app| AppSnapshot {
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

#[async_trait]
impl ProviderConfigLookupRepository for db::Database {
    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfigSnapshot, BridgeError> {
        db::provider_configs::get_provider_config(self.pool(), app_id, provider)
            .await
            .map(|provider_config| ProviderConfigSnapshot {
                config: provider_config.config,
            })
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
impl SubscriptionLookupRepository for db::Database {
    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
        db::subscriptions::get_subscription_by_sub_id(self.pool(), app_id, subscription_id)
            .await
            .map(|subscription| subscription.map(map_subscription_lookup_snapshot))
    }

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<SubscriptionLookupSnapshot>, BridgeError> {
        db::subscriptions::get_subscription_by_purchase_token(self.pool(), app_id, purchase_token)
            .await
            .map(|subscription| subscription.map(map_subscription_lookup_snapshot))
    }
}

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

#[async_trait]
impl VerifyPurchaseRepository for db::Database {
    async fn get_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Option<VerifyPurchaseSubscriptionSnapshot>, BridgeError> {
        db::subscriptions::get_subscription(
            self.pool(),
            app_id,
            external_user_id,
            subscription_id,
            provider,
        )
        .await
        .map(map_verify_purchase_subscription)
        .map(Some)
    }

    async fn commit_verified_purchase(
        &self,
        request: VerifyPurchaseCommitRequest<'_>,
    ) -> Result<VerifyPurchaseCommitResult, BridgeError> {
        let app_id = request.app_id;
        let resolved_external_user_id = request.resolved_external_user_id.to_string();
        let provider = request.provider.to_string();
        let subscription_id = request.subscription_id.to_string();
        let purchase_token = request.purchase_token.to_string();
        let subscription_status = request.subscription_status.to_string();
        let payment_status = request.payment_status.to_string();
        let provider_customer_id = request.provider_customer_id.map(|value| value.to_string());
        let google_obfuscated_account_id = request
            .google_obfuscated_account_id
            .map(|value| value.to_string());
        let current_period_end = request.current_period_end;
        let auto_renewing = request.auto_renewing;
        let payment_state = request.payment_state;
        let amount_cents = request.amount_cents;
        let event_time_ms = request.event_time_ms;
        let is_subscription = request.is_subscription;

        let pool = self.pool();
        with_transaction_impl(pool, move |tx| {
            Box::pin(async move {
                db::payments::record_payment_tx(
                    tx,
                    app_id,
                    &resolved_external_user_id,
                    &provider,
                    &purchase_token,
                    Some(&subscription_id),
                    amount_cents,
                    &payment_status,
                )
                .await?;

                let mut subscription = None;

                if is_subscription {
                    let upsert_result = db::subscriptions::upsert_subscription_tx(
                        tx,
                        app_id,
                        &resolved_external_user_id,
                        &subscription_id,
                        &provider,
                        &subscription_status,
                        current_period_end,
                        Some(&purchase_token),
                        auto_renewing,
                        payment_state,
                        provider_customer_id.as_deref(),
                        event_time_ms,
                    )
                    .await?;

                    if provider == "google_play" {
                        sqlx::query(
                            "UPDATE pay.subscriptions
                             SET google_obfuscated_account_id = COALESCE($1, google_obfuscated_account_id),
                                 updated_at = NOW()
                             WHERE app_id = $2 AND external_user_id = $3 AND subscription_id = $4 AND provider = $5",
                        )
                        .bind(google_obfuscated_account_id.as_deref())
                        .bind(app_id)
                        .bind(&resolved_external_user_id)
                        .bind(&subscription_id)
                        .bind(&provider)
                        .execute(&mut **tx)
                        .await
                        .map_err(|e| BridgeError::DbError(e.to_string()))?;
                    }

                    subscription = Some(map_verify_purchase_subscription(
                        upsert_result.subscription,
                    ));
                }

                Ok(TransactionOutcome::Commit(VerifyPurchaseCommitResult { subscription }))
            })
        })
        .await
    }
}

#[async_trait]
impl PaymentAcknowledgementRepository for db::Database {
    async fn payment_acknowledged_at(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, BridgeError> {
        db::payments::get_payment_acknowledged_at(
            self.pool(),
            app_id,
            provider,
            provider_transaction_id,
        )
        .await
    }

    async fn mark_payment_acknowledged(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<(), BridgeError> {
        db::payments::mark_payment_acknowledged(
            self.pool(),
            app_id,
            provider,
            provider_transaction_id,
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
    ) -> Result<Uuid, BridgeError> {
        db::webhooks::create_webhook_delivery(self.pool(), app_id, webhook_provider_id).await
    }
}

#[async_trait]
impl WebhookIngressRepository for db::Database {
    async fn get_app_by_webhook_token(&self, token: Uuid) -> Result<AppSnapshot, BridgeError> {
        db::apps::get_app_by_webhook_token(self.pool(), token)
            .await
            .map(|app| AppSnapshot {
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

    async fn update_webhook_delivery_attempt(
        &self,
        delivery_id: Uuid,
        http_status: Option<i32>,
        error: Option<String>,
        forwarded: bool,
    ) -> Result<(), BridgeError> {
        db::webhooks::update_webhook_delivery_attempt(
            self.pool(),
            delivery_id,
            http_status,
            error,
            forwarded,
        )
        .await
    }
}

#[async_trait]
impl WebhookProcessingLookupRepository for db::Database {
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
impl WebhookProviderLookupRepository for db::Database {
    async fn get_webhook_provider(&self, id: Uuid) -> Result<WebhookProviderSnapshot, BridgeError> {
        let webhook = db::webhooks::get_webhook_provider(self.pool(), id).await?;
        Ok(WebhookProviderSnapshot {
            provider: webhook.provider,
            provider_webhook_id: webhook.provider_webhook_id,
            event_type: webhook.event_type,
            subscription_id: webhook.subscription_id,
            purchase_token: webhook.purchase_token,
            payload: webhook.payload,
            timestamp_epoch_ms: webhook.timestamp_epoch_ms,
            suppressed: webhook.suppressed,
            suppressed_reason: webhook.suppressed_reason,
        })
    }
}

#[async_trait]
impl PaymentStatusLookupRepository for db::Database {
    async fn get_payment_status(
        &self,
        app_id: Uuid,
        provider_transaction_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::payments::get_payment_status(self.pool(), app_id, provider_transaction_id).await
    }
}

#[async_trait]
impl WebhookProcessingMutationRepository for db::Database {
    async fn update_payment_status(
        &self,
        app_id: Uuid,
        provider_transaction_id: &str,
        new_status: &str,
    ) -> Result<(), BridgeError> {
        db::payments::update_payment_status(self.pool(), app_id, provider_transaction_id, new_status).await
    }

    async fn apply_subscription_transition(
        &self,
        app_id: Uuid,
        subscription_id: &str,
        event_time_ms: i64,
        transition: SubscriptionWebhookTransition,
    ) -> Result<Option<Subscription>, BridgeError> {
        db::subscriptions::apply_webhook_transition(
            self.pool(),
            app_id,
            subscription_id,
            event_time_ms,
            transition,
        )
        .await
    }

    async fn link_replacement_subscriptions(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        current_subscription_id: &str,
        last_event_time: i64,
    ) -> Result<(), BridgeError> {
        db::subscriptions::link_replacement_subscriptions(
            self.pool(),
            app_id,
            external_user_id,
            current_subscription_id,
            last_event_time,
        )
        .await
    }

    async fn apply_topup_if_new(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        amount_cents: i32,
        charge_id: &str,
    ) -> Result<bool, BridgeError> {
        db::agent::apply_topup_if_new(
            self.pool(),
            app_id,
            external_user_id,
            amount_cents,
            charge_id,
        )
        .await
    }

    async fn mark_webhook_processed(&self, webhook_id: Uuid) -> Result<(), BridgeError> {
        db::webhooks::mark_webhook_processed(self.pool(), webhook_id).await
    }
}

#[async_trait]
impl WebhookProcessingTransactionRepository for db::Database {
    async fn record_webhook_payment(
        &self,
        request: WebhookPaymentRecordRequest<'_>,
    ) -> Result<(), BridgeError> {
        let request = OwnedWebhookPaymentRecord::from(request);
        let pool = self.pool();

        with_transaction_impl(pool, move |tx| {
            Box::pin(async move {
                db::payments::record_payment_tx(
                    tx,
                    request.app_id,
                    &request.external_user_id,
                    &request.provider,
                    &request.provider_transaction_id,
                    request.subscription_id.as_deref(),
                    request.amount_cents,
                    &request.status,
                )
                .await?;

                Ok(TransactionOutcome::Commit(()))
            })
        })
        .await
    }

    async fn commit_webhook_subscription(
        &self,
        request: WebhookSubscriptionCommitRequest<'_>,
    ) -> Result<Option<WebhookSubscriptionSnapshot>, BridgeError> {
        let app_id = request.app_id;
        let external_user_id = request.external_user_id.to_string();
        let subscription_id = request.subscription_id.to_string();
        let provider = request.provider.to_string();
        let status = request.status.to_string();
        let current_period_end = request.current_period_end;
        let purchase_token = request.purchase_token.map(str::to_string);
        let auto_renewing = request.auto_renewing;
        let payment_state = request.payment_state;
        let provider_customer_id = request.provider_customer_id.map(str::to_string);
        let event_time_ms = request.event_time_ms;
        let payment = request.payment.map(OwnedWebhookPaymentRecord::from);
        let adopt_stale_payment = request.adopt_stale_payment;

        let pool = self.pool();
        with_transaction_impl(pool, move |tx| {
            Box::pin(async move {
                let upsert_result = db::subscriptions::upsert_subscription_tx(
                    tx,
                    app_id,
                    &external_user_id,
                    &subscription_id,
                    &provider,
                    &status,
                    current_period_end,
                    purchase_token.as_deref(),
                    auto_renewing,
                    payment_state,
                    provider_customer_id.as_deref(),
                    event_time_ms,
                )
                .await?;

                if !upsert_result.applied {
                    return Ok(TransactionOutcome::Rollback(None));
                }

                if let Some(payment) = payment.as_ref() {
                    db::payments::record_payment_tx(
                        tx,
                        payment.app_id,
                        &payment.external_user_id,
                        &payment.provider,
                        &payment.provider_transaction_id,
                        payment.subscription_id.as_deref(),
                        payment.amount_cents,
                        &payment.status,
                    )
                    .await?;
                }

                if adopt_stale_payment {
                    let _ = db::payments::adopt_stale_payment(
                        tx,
                        app_id,
                        &external_user_id,
                        &subscription_id,
                    )
                    .await;
                }

                Ok(TransactionOutcome::Commit(Some(upsert_result.subscription.into())))
            })
        })
        .await
    }
}

#[async_trait]
impl WebhookProcessingRepository for db::Database {}

#[async_trait]
impl SchedulerRepository for db::Database {
    async fn list_enabled_apps(&self) -> Result<Vec<App>, BridgeError> {
        db::apps::list_enabled_apps(self.pool()).await
    }

    async fn list_pending_webhook_deliveries(
        &self,
        app_id: Uuid,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, BridgeError> {
        db::webhooks::list_pending_webhook_deliveries(self.pool(), app_id, limit).await
    }

    async fn list_reconciliation_subscriptions(
        &self,
        app_id: Uuid,
    ) -> Result<Vec<Subscription>, BridgeError> {
        db::subscriptions::list_reconciliation_subscriptions(self.pool(), app_id).await
    }

    async fn update_subscription_status(
        &self,
        app_id: Uuid,
        subscription_id: &str,
        new_status: &str,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError> {
        db::subscriptions::update_subscription_status(
            self.pool(),
            app_id,
            subscription_id,
            new_status,
            event_time_ms,
        )
        .await
    }

    async fn list_price_step_up_expired_subscriptions(
        &self,
        limit: i64,
    ) -> Result<Vec<Subscription>, BridgeError> {
        db::subscriptions::list_price_step_up_expired_subscriptions(self.pool(), limit).await
    }

    async fn mark_subscription_price_step_up_expired(
        &self,
        id: Uuid,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError> {
        db::subscriptions::mark_subscription_price_step_up_expired(self.pool(), id, event_time_ms).await
    }

    async fn list_pending_pause_subscriptions(
        &self,
        limit: i64,
    ) -> Result<Vec<Subscription>, BridgeError> {
        db::subscriptions::list_pending_pause_subscriptions(self.pool(), limit).await
    }

    async fn mark_subscription_paused(
        &self,
        id: Uuid,
        event_time_ms: i64,
    ) -> Result<bool, BridgeError> {
        db::subscriptions::mark_subscription_paused(self.pool(), id, event_time_ms).await
    }

    async fn delete_orphaned_pending_subscriptions(&self) -> Result<u64, BridgeError> {
        db::subscriptions::delete_orphaned_pending_subscriptions(self.pool()).await
    }

    async fn cleanup_old_webhook_provider(&self) -> Result<(), BridgeError> {
        db::webhooks::cleanup_old_webhook_provider(self.pool()).await
    }

    async fn cleanup_expired_agent_tokens(&self) -> Result<(), BridgeError> {
        db::agent::cleanup_expired_agent_tokens(self.pool()).await
    }

    async fn cleanup_purged_fraud_prevention(&self) -> Result<(), BridgeError> {
        db::users::cleanup_purged_fraud_prevention(self.pool()).await
    }
}
