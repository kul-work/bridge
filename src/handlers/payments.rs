use crate::config::API_PAGINATION_LIMIT;
use crate::config::MAX_PAGINATION_LIMIT;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use crate::ports::{PaymentReadRepository, SubscriptionWriteRepository};
use crate::state::AppState;
use axum::{
    extract::{State, Extension, Query},
    http::StatusCode,
    Json,
};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct PaymentsQuery {
    pub external_user_id: String,
    pub limit: Option<i64>,
    pub after: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaymentDetail {
    pub id: String,
    pub external_user_id: String,
    pub subscription_id: Option<String>,
    pub provider: String,
    pub provider_transaction_id: String,
    pub amount_cents: i64,
    pub currency: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct PaymentsPagination {
    pub has_more: bool,
    pub after: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PaymentsResponse {
    pub payments: Vec<PaymentDetail>,
    pub total: i64,
    pub limit: i64,
    pub pagination: PaymentsPagination,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PaymentsCursor {
    created_at: DateTime<Utc>,
    id: Uuid,
}

pub async fn get_payments(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Query(query): Query<PaymentsQuery>,
) -> Result<(StatusCode, Json<PaymentsResponse>), BridgeError> {
    let database = state.database();
    if query.external_user_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "external_user_id is required".to_string(),
        ));
    }

    let limit = query.limit.unwrap_or(API_PAGINATION_LIMIT).min(MAX_PAGINATION_LIMIT);
    let cursor = decode_cursor(query.after.as_deref())?;

    let total = database
        .as_ref()
        .count_user_payments(auth.app_id, &query.external_user_id)
        .await?;

    let rows = database
        .as_ref()
        .list_user_payments_keyset(
            auth.app_id,
            &query.external_user_id,
            limit + 1,
            cursor.as_ref().map(|c| c.created_at),
            cursor.as_ref().map(|c| c.id),
        )
        .await?;

    let has_more = rows.len() > limit as usize;
    let page_rows: Vec<_> = rows.into_iter().take(limit as usize).collect();

    let payments = page_rows
        .iter()
        .map(|row| PaymentDetail {
            id: row.id.to_string(),
            external_user_id: row.external_user_id.clone(),
            subscription_id: row.subscription_id.clone(),
            provider: row.provider.clone(),
            provider_transaction_id: row.provider_transaction_id.clone(),
            amount_cents: row.amount_cents as i64,
            currency: row.currency.clone(),
            status: row.status.clone(),
            created_at: row.created_at.to_rfc3339(),
        })
        .collect();

    let next_cursor = if has_more {
        let last = page_rows.last().expect("has_more implies non-empty page");
        let cursor = PaymentsCursor {
            created_at: last.created_at,
            id: last.id,
        };
        Some(encode_cursor(&cursor)?)
    } else {
        None
    };

    Ok((
        StatusCode::OK,
        Json(PaymentsResponse {
            payments,
            total,
            limit,
            pagination: PaymentsPagination {
                has_more,
                after: next_cursor,
            },
        }),
    ))
}

fn decode_cursor(after: Option<&str>) -> Result<Option<PaymentsCursor>, BridgeError> {
    let Some(raw) = after else {
        return Ok(None);
    };

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .map_err(|_| BridgeError::ValidationError("Invalid cursor encoding".to_string()))?;
    let cursor: PaymentsCursor = serde_json::from_slice(&decoded)
        .map_err(|_| BridgeError::ValidationError("Invalid cursor payload".to_string()))?;
    Ok(Some(cursor))
}

fn encode_cursor(cursor: &PaymentsCursor) -> Result<String, BridgeError> {
    let bytes = serde_json::to_vec(cursor)
        .map_err(|e| BridgeError::InternalServerError(format!("Failed to encode cursor: {}", e)))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterPurchaseRequest {
    pub external_user_id: String,
    pub subscription_id: String,
    pub provider: String,
    #[allow(dead_code)]
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct RegisterPurchaseResponse {
    pub status: String,
}

pub async fn register_purchase(
    State(state): State<AppState>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<RegisterPurchaseRequest>,
) -> Result<(StatusCode, Json<RegisterPurchaseResponse>), BridgeError> {
    let database = state.database();
    if request.external_user_id.is_empty()
        || request.subscription_id.is_empty()
        || request.provider.is_empty()
    {
        return Err(BridgeError::ValidationError(
            "external_user_id, subscription_id, and provider are required".to_string(),
        ));
    }

    // §6: Create pending subscription placeholder (not a manual grant)
    let _sub = database
        .as_ref()
        .upsert_pending_subscription(
            auth.app_id,
            &request.external_user_id,
            &request.subscription_id,
            &request.provider,
        )
        .await?;

    Ok((
        StatusCode::OK,
        Json(RegisterPurchaseResponse {
            status: "registered".to_string(),
        }),
    ))
}
