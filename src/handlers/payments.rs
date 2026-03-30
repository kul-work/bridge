use crate::error::BridgeError;
use crate::handlers::api_key::AppAuth;
use axum::{
    extract::{State, Extension, Query},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use sqlx::Row;

#[derive(Debug, Deserialize)]
pub struct PaymentsQuery {
    pub external_user_id: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PaymentDetail {
    pub id: String,
    pub external_user_id: String,
    pub subscription_id: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct PaymentsResponse {
    pub payments: Vec<PaymentDetail>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
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

    let limit = query.limit.unwrap_or(20).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Get total count
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pay.payments WHERE app_id = $1 AND external_user_id = $2",
    )
    .bind(auth.app_id)
    .bind(&query.external_user_id)
    .fetch_one(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    // Get payments
    let rows = sqlx::query(
        r#"
        SELECT 
            id, external_user_id, subscription_id, amount, currency, status, created_at
        FROM pay.payments
        WHERE app_id = $1 AND external_user_id = $2
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(auth.app_id)
    .bind(&query.external_user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&database.pool)
    .await
    .map_err(|e| BridgeError::DbError(e.to_string()))?;

    let payments = rows
        .into_iter()
        .map(|row| PaymentDetail {
            id: row.get::<String, _>("id"),
            external_user_id: row.get::<String, _>("external_user_id"),
            subscription_id: row.get::<Option<String>, _>("subscription_id"),
            amount: row.get::<i64, _>("amount"),
            currency: row.get::<String, _>("currency"),
            status: row.get::<String, _>("status"),
            created_at: row
                .get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .to_rfc3339(),
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(PaymentsResponse {
            payments,
            total,
            limit,
            offset,
        }),
    ))
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
    pub success: bool,
    pub message: String,
    pub subscription_id: String,
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
    let sub = crate::db::subscriptions::upsert_pending_subscription(
        &database.pool,
        auth.app_id,
        &request.external_user_id,
        &request.subscription_id,
        &request.provider,
    ).await?;

    Ok((
        StatusCode::CREATED,
        Json(RegisterPurchaseResponse {
            success: true,
            message: "Purchase registered as pending subscription".to_string(),
            subscription_id: sub.subscription_id,
            status: sub.status,
        }),
    ))
}
