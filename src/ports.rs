use async_trait::async_trait;
use std::{future::Future, pin::Pin};
use uuid::Uuid;

use crate::{
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
        provider_configs::ProviderConfig,
        subscriptions::{Subscription, SubscriptionUpsertResult},
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
pub trait CheckoutRepository: AppProviderRepository + Send + Sync {
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
pub trait VerifyPurchaseRepository:
    AppProviderRepository + WebhookWriteRepository + WebhookForwardRepository + Send + Sync
{
    async fn lookup_user_by_google_obfuscated_id(
        &self,
        app_id: Uuid,
        obfuscated_id: &str,
    ) -> Result<Option<String>, BridgeError>;

    async fn get_subscription(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Option<VerifyPurchaseSubscriptionSnapshot>, BridgeError>;

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<VerifyPurchaseSubscriptionSnapshot>, BridgeError>;

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

    async fn commit_verified_purchase(
        &self,
        request: VerifyPurchaseCommitRequest<'_>,
    ) -> Result<VerifyPurchaseCommitResult, BridgeError>;
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
pub trait WebhookIngressRepository: AppProviderRepository + WebhookWriteRepository + Send + Sync {
    async fn get_app_by_webhook_token(&self, token: Uuid) -> Result<App, BridgeError>;
}

#[async_trait]
pub trait AppProviderRepository: Send + Sync {
    async fn get_app(&self, app_id: Uuid) -> Result<App, BridgeError>;

    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfig, BridgeError>;
}

pub trait AppWebhookRepository:
    WebhookForwardRepository + WebhookWriteRepository + Send + Sync
{
}

impl<T> AppWebhookRepository for T where
    T: WebhookForwardRepository + WebhookWriteRepository + Send + Sync + ?Sized
{
}

#[async_trait]
pub trait WebhookForwardRepository: AppProviderRepository + Send + Sync {
    async fn get_webhook_delivery(&self, id: Uuid) -> Result<WebhookDelivery, BridgeError>;

    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<Subscription>, BridgeError>;

    async fn suppress_webhook(&self, webhook_id: Uuid, reason: &str) -> Result<(), BridgeError>;

    async fn update_webhook_delivery_attempt(
        &self,
        delivery_id: Uuid,
        http_status: Option<i32>,
        error: Option<String>,
        forwarded: bool,
    ) -> Result<(), BridgeError>;
}

#[async_trait]
pub trait WebhookProcessingRepository: Send + Sync {
    fn with_transaction<'a, T, F>(
        &'a self,
        f: F,
    ) -> Pin<Box<dyn Future<Output = Result<T, BridgeError>> + Send + 'a>>
    where
        T: Send + 'a,
        F: for<'tx> FnOnce(
                &'tx mut sqlx::Transaction<'a, sqlx::Postgres>,
            ) -> Pin<Box<dyn Future<Output = Result<TransactionOutcome<T>, BridgeError>> + Send + 'tx>>
            + Send
            + 'a;

    async fn get_webhook_provider(&self, id: Uuid) -> Result<WebhookProvider, BridgeError>;

    async fn get_app(&self, app_id: Uuid) -> Result<App, BridgeError>;

    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfig, BridgeError>;

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

    async fn lookup_user_by_google_obfuscated_id(
        &self,
        app_id: Uuid,
        obfuscated_id: &str,
    ) -> Result<Option<String>, BridgeError>;

    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<Subscription>, BridgeError>;

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<Subscription>, BridgeError>;

    async fn get_payment_status(
        &self,
        app_id: Uuid,
        provider_transaction_id: &str,
    ) -> Result<Option<String>, BridgeError>;

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
        transition: db::subscriptions::SubscriptionWebhookTransition,
    ) -> Result<Option<Subscription>, BridgeError>;

    async fn upsert_subscription_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
        status: &str,
        current_period_end: Option<chrono::DateTime<chrono::Utc>>,
        purchase_token: Option<&str>,
        auto_renewing: Option<bool>,
        payment_state: Option<i32>,
        provider_customer_id: Option<&str>,
        event_time_ms: i64,
    ) -> Result<SubscriptionUpsertResult, BridgeError>;

    async fn record_payment_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        provider_transaction_id: &str,
        subscription_id: Option<&str>,
        amount_cents: i32,
        status: &str,
    ) -> Result<(), BridgeError>;

    async fn adopt_stale_payment(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
    ) -> Result<(), BridgeError>;

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

    async fn suppress_webhook(&self, webhook_id: Uuid, reason: &str) -> Result<(), BridgeError>;

    async fn mark_webhook_processed(&self, webhook_id: Uuid) -> Result<(), BridgeError>;
}

#[async_trait]
pub trait SchedulerRepository: AppProviderRepository + WebhookWriteRepository + Send + Sync {
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

fn map_verify_purchase_subscription(
    subscription: Subscription,
) -> VerifyPurchaseSubscriptionSnapshot {
    VerifyPurchaseSubscriptionSnapshot {
        external_user_id: subscription.external_user_id,
        subscription_id: subscription.subscription_id,
        provider: subscription.provider,
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
impl AppProviderRepository for db::Database {
    async fn get_app(&self, app_id: Uuid) -> Result<App, BridgeError> {
        db::apps::get_app(self.pool(), app_id).await
    }

    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfig, BridgeError> {
        db::provider_configs::get_provider_config(self.pool(), app_id, provider).await
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
    async fn lookup_user_by_google_obfuscated_id(
        &self,
        app_id: Uuid,
        obfuscated_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::subscriptions::lookup_user_by_google_obfuscated_id(self.pool(), app_id, obfuscated_id).await
    }

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

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<VerifyPurchaseSubscriptionSnapshot>, BridgeError> {
        db::subscriptions::get_subscription_by_purchase_token(self.pool(), app_id, purchase_token)
            .await
            .map(|subscription| subscription.map(map_verify_purchase_subscription))
    }

    async fn payment_acknowledged_at(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, BridgeError> {
        sqlx::query_scalar(
            "SELECT acknowledged_at FROM pay.payments WHERE app_id = $1 AND provider = $2 AND provider_transaction_id = $3",
        )
        .bind(app_id)
        .bind(provider)
        .bind(provider_transaction_id)
        .fetch_optional(self.pool())
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))
        .map(|row| row.flatten())
    }

    async fn mark_payment_acknowledged(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<(), BridgeError> {
        sqlx::query(
            "UPDATE pay.payments
             SET acknowledged_at = COALESCE(acknowledged_at, NOW())
             WHERE app_id = $1 AND provider = $2 AND provider_transaction_id = $3",
        )
        .bind(app_id)
        .bind(provider)
        .bind(provider_transaction_id)
        .execute(self.pool())
        .await
        .map_err(|e| BridgeError::DbError(e.to_string()))?;

        Ok(())
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
        let current_period_end = request.current_period_end.clone();
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
    async fn get_app_by_webhook_token(&self, token: Uuid) -> Result<App, BridgeError> {
        db::apps::get_app_by_webhook_token(self.pool(), token).await
    }
}

#[async_trait]
impl WebhookForwardRepository for db::Database {
    async fn get_webhook_delivery(&self, id: Uuid) -> Result<WebhookDelivery, BridgeError> {
        db::webhooks::get_webhook_delivery(self.pool(), id).await
    }

    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<Subscription>, BridgeError> {
        db::subscriptions::get_subscription_by_sub_id(self.pool(), app_id, subscription_id).await
    }

    async fn suppress_webhook(&self, webhook_id: Uuid, reason: &str) -> Result<(), BridgeError> {
        db::webhooks::suppress_webhook(self.pool(), webhook_id, reason).await
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
impl WebhookProcessingRepository for db::Database {
    fn with_transaction<'a, T, F>(
        &'a self,
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
        let pool = self.pool();
        with_transaction_impl(pool, f)
    }

    async fn get_webhook_provider(&self, id: Uuid) -> Result<WebhookProvider, BridgeError> {
        db::webhooks::get_webhook_provider(self.pool(), id).await
    }

    async fn get_app(&self, app_id: Uuid) -> Result<App, BridgeError> {
        db::apps::get_app(self.pool(), app_id).await
    }

    async fn get_provider_config(
        &self,
        app_id: Uuid,
        provider: &str,
    ) -> Result<ProviderConfig, BridgeError> {
        db::provider_configs::get_provider_config(self.pool(), app_id, provider).await
    }

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

    async fn lookup_user_by_google_obfuscated_id(
        &self,
        app_id: Uuid,
        obfuscated_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::subscriptions::lookup_user_by_google_obfuscated_id(self.pool(), app_id, obfuscated_id).await
    }

    async fn get_subscription_by_sub_id(
        &self,
        app_id: Uuid,
        subscription_id: &str,
    ) -> Result<Option<Subscription>, BridgeError> {
        db::subscriptions::get_subscription_by_sub_id(self.pool(), app_id, subscription_id).await
    }

    async fn get_subscription_by_purchase_token(
        &self,
        app_id: Uuid,
        purchase_token: &str,
    ) -> Result<Option<Subscription>, BridgeError> {
        db::subscriptions::get_subscription_by_purchase_token(self.pool(), app_id, purchase_token).await
    }

    async fn get_payment_status(
        &self,
        app_id: Uuid,
        provider_transaction_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::payments::get_payment_status(self.pool(), app_id, provider_transaction_id).await
    }

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
        transition: db::subscriptions::SubscriptionWebhookTransition,
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

    async fn upsert_subscription_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
        status: &str,
        current_period_end: Option<chrono::DateTime<chrono::Utc>>,
        purchase_token: Option<&str>,
        auto_renewing: Option<bool>,
        payment_state: Option<i32>,
        provider_customer_id: Option<&str>,
        event_time_ms: i64,
    ) -> Result<SubscriptionUpsertResult, BridgeError> {
        db::subscriptions::upsert_subscription_tx(
            tx,
            app_id,
            external_user_id,
            subscription_id,
            provider,
            status,
            current_period_end,
            purchase_token,
            auto_renewing,
            payment_state,
            provider_customer_id,
            event_time_ms,
        )
        .await
    }

    async fn record_payment_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        app_id: Uuid,
        external_user_id: &str,
        provider: &str,
        provider_transaction_id: &str,
        subscription_id: Option<&str>,
        amount_cents: i32,
        status: &str,
    ) -> Result<(), BridgeError> {
        db::payments::record_payment_tx(
            tx,
            app_id,
            external_user_id,
            provider,
            provider_transaction_id,
            subscription_id,
            amount_cents,
            status,
        )
        .await
    }

    async fn adopt_stale_payment(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
    ) -> Result<(), BridgeError> {
        db::payments::adopt_stale_payment(tx, app_id, external_user_id, subscription_id).await
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

    async fn suppress_webhook(&self, webhook_id: Uuid, reason: &str) -> Result<(), BridgeError> {
        db::webhooks::suppress_webhook(self.pool(), webhook_id, reason).await
    }

    async fn mark_webhook_processed(&self, webhook_id: Uuid) -> Result<(), BridgeError> {
        let pool = self.pool();
        sqlx::query("UPDATE pay.webhook_provider SET processed = true WHERE id = $1")
            .bind(webhook_id)
            .execute(pool)
            .await
            .map_err(|e| BridgeError::DbError(e.to_string()))?;

        Ok(())
    }
}

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




