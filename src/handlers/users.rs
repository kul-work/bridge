use axum::{
    extract::{Path, State},
    Extension, Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::application;
use crate::application::users::AnonymizeUserInput;
use crate::config::DATA_EXPORT_LIMIT;
use crate::db::{payments::Payment, subscriptions::Subscription};
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::ports::{
    PaymentReadRepository, SubscriptionReadRepository, WebhookReadRepository,
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
    let database = state.database();
    let result = application::users::anonymize_user(
        database.as_ref(),
        AnonymizeUserInput {
            app_id: auth.app_id,
            external_user_id: &external_user_id,
            reason: request.reason.as_deref(),
        },
    )
    .await?;

    if result.subscriptions_cancelled == 0 && result.payments_anonymized == 0 {
        return Err(BridgeError::ValidationError("User not found".to_string()));
    }

    Ok(Json(json!({
        "anonymized": true,
        "subscriptions_cancelled": result.subscriptions_cancelled,
        "payments_anonymized": result.payments_anonymized,
        "new_anonymous_id": result.new_anonymous_id
    })))
}

pub async fn data_export(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Path(external_user_id): Path<String>,
) -> Result<Json<serde_json::Value>, BridgeError> {
    let database = state.database();
    let subscriptions =
        collect_all_subscriptions(database.as_ref(), auth.app_id, &external_user_id).await?;
    let payments = collect_all_payments(database.as_ref(), auth.app_id, &external_user_id).await?;

    let sub_ids: Vec<String> = subscriptions.iter().map(|s| s.subscription_id.clone()).collect();
    let tokens: Vec<String> = payments.iter().map(|p| p.provider_transaction_id.clone()).collect();

    let webhook_records = database
        .as_ref()
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
