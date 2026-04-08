use axum::{
    extract::{Path, State},
    Extension, Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

use crate::config::DATA_EXPORT_LIMIT;
use crate::db::{payments::Payment, subscriptions::Subscription};
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::ports::{
    AgentReadRepository, PaymentReadRepository, SubscriptionReadRepository, UserRepository,
    WebhookReadRepository,
};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AnonymizeRequest {
    pub reason: Option<String>,
}

pub async fn anonymize(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(external_user_id): Path<String>,
    Json(request): Json<AnonymizeRequest>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let (subscriptions_cancelled, payments_anonymized, new_anonymous_id) = state
        .user_repo
        .anonymize_user(auth.app_id, &external_user_id, request.reason.as_deref())
        .await?;

    if subscriptions_cancelled == 0 && payments_anonymized == 0 {
        return Err(BridgeError::ValidationError("User not found".to_string()));
    }

    Ok(Json(json!({
        "anonymized": true,
        "subscriptions_cancelled": subscriptions_cancelled,
        "payments_anonymized": payments_anonymized,
        "new_anonymous_id": new_anonymous_id
    })))
}

pub async fn data_export(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(external_user_id): Path<String>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let subscriptions = collect_all_subscriptions(
        state.subscription_read_repo.as_ref(),
        auth.app_id,
        &external_user_id,
    )
    .await?;
    let payments = collect_all_payments(
        state.payment_read_repo.as_ref(),
        auth.app_id,
        &external_user_id,
    )
    .await?;

    let agent_credits = state
        .agent_read_repo
        .get_agent_credit(auth.app_id, &external_user_id)
        .await?
        .map(|c| {
            json!({
                "balance_cents": c.balance_cents,
                "lifetime_spent_cents": c.lifetime_spent_cents,
                "updated_at": c.updated_at
            })
        });

    let agent_transactions = state
        .agent_read_repo
        .list_agent_transactions(auth.app_id, &external_user_id)
        .await?
        .into_iter()
        .map(|t| {
            json!({
                "request_type": t.request_type,
                "amount_cents": t.amount_cents,
                "charge_id": t.charge_id,
                "status": t.status,
                "created_at": t.created_at
            })
        })
        .collect::<Vec<_>>();

    let sub_ids: Vec<String> = subscriptions.iter().map(|s| s.subscription_id.clone()).collect();
    let tokens: Vec<String> = payments.iter().map(|p| p.provider_transaction_id.clone()).collect();

    let webhook_records = state
        .webhook_read_repo
        .list_user_webhook_records(auth.app_id, &sub_ids, &tokens)
        .await?
        .into_iter()
        .map(|w| {
            json!({
                "provider": w.provider,
                "event_type": w.event_type,
                "payload": w.payload,
                "created_at": w.created_at
            })
        })
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "external_user_id": external_user_id,
        "export_date": Utc::now(),
        "subscriptions": subscriptions,
        "payments": payments,
        "agent_credits": agent_credits,
        "agent_transactions": agent_transactions,
        "webhook_records": webhook_records
    })))
}

async fn collect_all_subscriptions(
    repository: &(impl SubscriptionReadRepository + ?Sized),
    app_id: uuid::Uuid,
    external_user_id: &str,
) -> Result<Vec<Subscription>, BridgeError> {
    let mut subscriptions = Vec::new();
    let mut offset = 0;

    loop {
        let batch = repository
            .get_user_subscriptions(
            app_id,
            external_user_id,
            DATA_EXPORT_LIMIT,
            offset,
        )
        .await?;

        let batch_len = batch.len() as i64;
        subscriptions.extend(batch);

        if batch_len < DATA_EXPORT_LIMIT {
            break;
        }

        offset += DATA_EXPORT_LIMIT;
    }

    Ok(subscriptions)
}

async fn collect_all_payments(
    repository: &(impl PaymentReadRepository + ?Sized),
    app_id: uuid::Uuid,
    external_user_id: &str,
) -> Result<Vec<Payment>, BridgeError> {
    let mut payments = Vec::new();
    let mut offset = 0;

    loop {
        let batch = repository
            .get_user_payments(
            app_id,
            external_user_id,
            DATA_EXPORT_LIMIT,
            offset,
        )
        .await?;

        let batch_len = batch.len() as i64;
        payments.extend(batch);

        if batch_len < DATA_EXPORT_LIMIT {
            break;
        }

        offset += DATA_EXPORT_LIMIT;
    }

    Ok(payments)
}
