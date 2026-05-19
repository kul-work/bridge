use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    application::verify_purchase_types::{VerifyPurchaseCommitRequest, VerifyPurchaseCommitResult},
    db::{self, payments::{Payment, PaymentHistoryEntry}},
    error::BridgeError,
    ports::composites::VerifyPurchaseRepository,
    ports::helpers::{map_verify_purchase_subscription, with_transaction_impl},
    ports::traits::{
        PaymentAcknowledgementRepository, PaymentReadRepository, PaymentRepository,
    },
    ports::types::TransactionOutcome,
};

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

    async fn get_payment_status_for_provider(
        &self,
        app_id: Uuid,
        provider: &str,
        provider_transaction_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::payments::get_payment_status_for_provider(
            self.pool(),
            app_id,
            provider,
            provider_transaction_id,
        )
        .await
    }

    async fn get_payment_currency_for_subscription(
        &self,
        app_id: Uuid,
        provider: &str,
        external_user_id: &str,
        subscription_id: &str,
    ) -> Result<Option<String>, BridgeError> {
        db::payments::get_payment_currency_for_subscription(
            self.pool(),
            app_id,
            provider,
            external_user_id,
            subscription_id,
        )
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
impl PaymentRepository for db::Database {}

#[async_trait]
impl VerifyPurchaseRepository for db::Database {
    async fn get_subscription_snapshot(
        &self,
        app_id: Uuid,
        external_user_id: &str,
        subscription_id: &str,
        provider: &str,
    ) -> Result<Option<crate::application::verify_purchase_types::VerifyPurchaseSubscriptionSnapshot>, BridgeError> {
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
        let provider_transaction_id = request.provider_transaction_id.to_string();
        let purchase_token = request.purchase_token.to_string();
        let subscription_status = request.subscription_status.to_string();
        let payment_status = request.payment_status.to_string();
        let provider_customer_id = request.provider_customer_id.map(|value| value.to_string());
        let google_obfuscated_account_id = request
            .google_obfuscated_account_id
            .map(|value| value.to_string());
        let google_linked_purchase_token = request
            .google_linked_purchase_token
            .map(|value| value.to_string());
        let current_period_end = request.current_period_end;
        let auto_renewing = request.auto_renewing;
        let payment_state = request.payment_state;
        let amount_cents = request.amount_cents;
        let currency = request.currency.map(str::to_string);
        let event_time_ms = request.event_time_ms;
        let is_subscription = request.is_subscription;

        let pool = self.pool();
        with_transaction_impl(pool, app_id, move |tx| {
            Box::pin(async move {
                db::payments::record_payment_with_purchase_token_tx(
                    tx,
                    app_id,
                    &resolved_external_user_id,
                    &provider,
                    &provider_transaction_id,
                    Some(&purchase_token),
                    provider == "google_play",
                    if is_subscription { Some(&subscription_id) } else { None },
                    // For subscriptions, also store the product id on the payment row.
                    // For one-time products, this was already the existing behavior: subscription_id carries the product id.
                    Some(&subscription_id),
                    amount_cents,
                    currency.as_deref(),
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
                                 google_linked_purchase_token = COALESCE($2, google_linked_purchase_token),
                                 updated_at = NOW()
                             WHERE app_id = $3 AND external_user_id = $4 AND subscription_id = $5 AND provider = $6",
                        )
                        .bind(google_obfuscated_account_id.as_deref())
                        .bind(google_linked_purchase_token.as_deref())
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
