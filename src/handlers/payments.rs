use crate::config::API_PAGINATION_LIMIT;
use crate::config::MAX_PAGINATION_LIMIT;
use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use axum::{
    extract::{State, Extension, Query},
    http::StatusCode,
    Json,
};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;
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
struct PaymentsCursor {
    created_at: DateTime<Utc>,
    id: Uuid,
}

pub async fn get_payments(
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Query(query): Query<PaymentsQuery>,
) -> Result<(StatusCode, Json<PaymentsResponse>), BridgeError> {
    if query.external_user_id.is_empty() {
        return Err(BridgeError::ValidationError(
            "external_user_id is required".to_string(),
        ));
    }

    let limit = query.limit.unwrap_or(API_PAGINATION_LIMIT).min(MAX_PAGINATION_LIMIT);
    let cursor = decode_cursor(query.after.as_deref())?;

    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pay.payments WHERE app_id = $1 AND external_user_id = $2",
    )
    .bind(auth.app_id)
    .bind(&query.external_user_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let rows = if let Some(cursor) = cursor.as_ref() {
        sqlx::query(
            r#"
            SELECT
                id, external_user_id, subscription_id, provider, provider_transaction_id, amount_cents, currency, status, created_at
            FROM pay.payments
            WHERE app_id = $1 AND external_user_id = $2
              AND (created_at, id) < ($3, $4)
            ORDER BY created_at DESC, id DESC
            LIMIT $5
            "#,
        )
        .bind(auth.app_id)
        .bind(&query.external_user_id)
        .bind(cursor.created_at)
        .bind(cursor.id)
        .bind(limit + 1)
        .fetch_all(&database.pool)
        .await
    } else {
        sqlx::query(
            r#"
            SELECT
                id, external_user_id, subscription_id, provider, provider_transaction_id, amount_cents, currency, status, created_at
            FROM pay.payments
            WHERE app_id = $1 AND external_user_id = $2
            ORDER BY created_at DESC, id DESC
            LIMIT $3
            "#,
        )
        .bind(auth.app_id)
        .bind(&query.external_user_id)
        .bind(limit + 1)
        .fetch_all(&database.pool)
        .await
    }
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let has_more = rows.len() > limit as usize;
    let page_rows: Vec<_> = rows.into_iter().take(limit as usize).collect();

    let payments = page_rows
        .iter()
        .map(|row| PaymentDetail {
            id: row.get::<Uuid, _>("id").to_string(),
            external_user_id: row.get::<String, _>("external_user_id"),
            subscription_id: row.get::<Option<String>, _>("subscription_id"),
            provider: row.get::<String, _>("provider"),
            provider_transaction_id: row.get::<String, _>("provider_transaction_id"),
            amount_cents: row.get::<i32, _>("amount_cents") as i64,
            currency: row.get::<String, _>("currency"),
            status: row.get::<String, _>("status"),
            created_at: row
                .get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .to_rfc3339(),
        })
        .collect();

    let next_cursor = if has_more {
        let last = page_rows.last().expect("has_more implies non-empty page");
        let cursor = PaymentsCursor {
            created_at: last.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
            id: last.get::<Uuid, _>("id"),
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
    State(database): State<Arc<crate::db::Database>>,
    Extension(auth): Extension<AppAuth>,
    Json(request): Json<RegisterPurchaseRequest>,
) -> Result<(StatusCode, Json<RegisterPurchaseResponse>), BridgeError> {
    if request.external_user_id.is_empty()
        || request.subscription_id.is_empty()
        || request.provider.is_empty()
    {
        return Err(BridgeError::ValidationError(
            "external_user_id, subscription_id, and provider are required".to_string(),
        ));
    }

    // §6: Create pending subscription placeholder (not a manual grant)
    let _sub = crate::db::subscriptions::upsert_pending_subscription(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &request.subscription_id,
        &request.provider,
    ).await?;

    Ok((
        StatusCode::OK,
        Json(RegisterPurchaseResponse {
            status: "registered".to_string(),
        }),
    ))
}
